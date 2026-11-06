//! Each worker constructs and owns its filesystem clients; HostBridge need not be Send.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::connection::{ConnectionSettings, Credentials, Secrets};
use super::filesystem::{CompareFs, LocalFs, RemoteFs};
use super::parallel::Pool;
use super::profiles::{Endpoint, Profile, absolute_directory};
use super::settings::{Parallelism, QuickCheck};
use super::text::{self, TextView};
use super::{ComparisonMode, Row, ScanEvent, cache, credentials, diagnostics, webdav_version};
use crate::filetransfer::params::ProtocolParams;
use crate::filetransfer::{FileTransferProtocol, RemoteFsBuilder};
use crate::host::{HostBridge, RemoteBridged};
use crate::system::bookmarks_client::BookmarksClient;
use crate::system::config_client::ConfigClient;
use crate::system::{KeyringStorage, environment};

pub enum Event {
    Scan {
        id: u64,
        event: ScanEvent,
    },
    Finished {
        id: u64,
        rows: Vec<Row>,
        cancelled: bool,
        /// Full request-to-result wall time, including connection/queue wait.
        elapsed: Duration,
    },
    Text {
        id: u64,
        path: PathBuf,
        view: Result<TextView, String>,
    },
    /// A scoped post-copy observation, not a new whole-comparison generation.
    Updated {
        id: u64,
        paths: Vec<PathBuf>,
        rows: Result<Vec<Row>, String>,
    },
    Copied {
        id: u64,
        result: Result<usize, String>,
    },
    Failed {
        id: u64,
        error: String,
        elapsed: Duration,
    },
}

enum Command {
    Scan {
        id: u64,
        /// Capture before queuing, not after the preceding I/O finishes.
        started: Instant,
        cancel: Arc<AtomicBool>,
        ignores: Vec<String>,
        parallelism: Parallelism,
        mode: ComparisonMode,
        quick_check: QuickCheck,
    },
    Diff {
        id: u64,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    CopyEntries {
        id: u64,
        paths: Vec<PathBuf>,
        to_right: bool,
        ignores: Vec<String>,
        cancel: Arc<AtomicBool>,
    },
    Copy {
        id: u64,
        path: PathBuf,
        view: Box<TextView>,
        selected: usize,
        to_right: bool,
        whole: bool,
    },
    Quit,
}

fn connect_pool(
    profile: &Profile,
    passwords: Credentials,
    cancel: &Arc<AtomicBool>,
    cache_path: Option<PathBuf>,
    connect: impl Fn(&Endpoint, Secrets) -> Result<Box<dyn CompareFs>, String> + Send + Sync + 'static,
) -> Result<Pool, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Connection cancelled".into());
    }
    let source = profile.source.clone();
    let destination = profile.destination.clone();
    let source_connect = Arc::new(connect);
    let destination_connect = source_connect.clone();
    Pool::connect(
        // Factories are reused for lazily connected lanes on later refreshes;
        // never capture the first scan's cancellation token here.
        move || source_connect(&source, passwords.source.clone()),
        move || destination_connect(&destination, passwords.destination.clone()),
        cache_path,
    )
}

pub struct Worker {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(
        mut profile: Profile,
        passwords: Credentials,
        keyring: bool,
        id: u64,
        parallelism: Parallelism,
    ) -> Self {
        let started = Instant::now();
        diagnostics::record("profile_open", || {
            serde_json::json!({
                "request_id": id, "source_backend": diagnostics::backend(&profile.source),
                "destination_backend": diagnostics::backend(&profile.destination),
                "parallelism": parallelism.get(), "quick_check": profile.quick_check,
            })
        });
        let (commands, requests) = mpsc::channel();
        let (events, responses) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let initial_cancel = cancel.clone();
        thread::spawn(move || {
            let result = connect_pool(
                &profile,
                passwords,
                // The shared connection pool must survive cancellation of the
                // initial scan: a quick/full refresh may already be queued.
                // Handshakes are blocking anyway; cancel the scan below, not
                // the reusable pool's construction before it has even started.
                &Arc::new(AtomicBool::new(false)),
                comparison_cache_path(),
                move |endpoint, secrets| build_endpoint(endpoint, secrets, keyring),
            );
            let mut pool = match result {
                Ok(pool) => pool,
                Err(error) => {
                    diagnostics::record(
                        "connection_failed",
                        || serde_json::json!({ "request_id": id }),
                    );
                    let _ = events.send(Event::Failed {
                        id,
                        error,
                        elapsed: started.elapsed(),
                    });
                    return;
                }
            };
            diagnostics::record("connection_ready", || {
                serde_json::json!({
                    "request_id": id, "pool_id": pool.diagnostic_id,
                })
            });
            pool.set_parallelism(parallelism);
            pool.set_quick_check(profile.quick_check);
            let mut comparison_mode = ComparisonMode::default().for_checks(profile.quick_check);
            perform_scan(
                id,
                started,
                &initial_cancel,
                &pool,
                &profile,
                &events,
                comparison_mode,
            );
            while let Ok(command) = requests.recv() {
                match command {
                    Command::Quit => break,
                    Command::CopyEntries {
                        id,
                        paths,
                        to_right,
                        ignores,
                        cancel,
                    } => {
                        diagnostics::record("copy_entries_start", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id, "to_right": to_right,
                                "paths": paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
                            })
                        });
                        let result = pool.copy_paths(paths.clone(), to_right, &ignores, &cancel);
                        let rows = copy_updates(&pool, &paths, &ignores, &cancel, comparison_mode);
                        let result = match (result, rows.as_ref().err()) {
                            (Ok(count), Some(error)) => Err(format!(
                                "Copied {count} entries; could not recheck copied paths: {error}"
                            )),
                            (result, _) => result,
                        };
                        diagnostics::record("copy_entries_end", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id,
                                "result_ok": result.is_ok(), "recheck_returned_ok": rows.is_ok(),
                                "cancelled": cancel.load(Ordering::Relaxed),
                            })
                        });
                        if events.send(Event::Updated { id, paths, rows }).is_err()
                            || events.send(Event::Copied { id, result }).is_err()
                        {
                            break;
                        }
                    }
                    Command::Copy {
                        id,
                        path,
                        view,
                        selected,
                        to_right,
                        whole,
                    } => {
                        diagnostics::record("diff_copy_start", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id, "path": path.to_string_lossy(),
                                "to_right": to_right, "whole": whole, "selected_chunk_line": selected,
                            })
                        });
                        let result = (|| {
                            let cancel = Arc::new(AtomicBool::new(false));
                            let (a, b) = pool.read_pair(&path, &cancel)?;
                            if a.bytes != view.left_document.bytes
                                || b.bytes != view.right_document.bytes
                            {
                                return Err(
                                    "File changed since opening the diff; reopen it before copying"
                                        .into(),
                                );
                            }
                            let merged = view.merged_bytes(selected, to_right, whole)?;
                            pool.write(usize::from(to_right), &path, merged)?;
                            let (a, b) = pool.read_pair(&path, &cancel)?;
                            Ok(text::diff_documents(path.display().to_string(), a, b))
                        })();
                        // Verify just this file, using byte/normalization rules,
                        // not decoded-text equality. Unrelated files are untouched.
                        let rows = copy_updates(
                            &pool,
                            std::slice::from_ref(&path),
                            &profile.ignores,
                            &Arc::new(AtomicBool::new(false)),
                            ComparisonMode::Full,
                        );
                        let result = match (result, rows.as_ref().err()) {
                            (Ok(_), Some(error)) => Err(format!(
                                "Copy finished but its result could not be rechecked: {error}"
                            )),
                            (result, _) => result,
                        };
                        diagnostics::record("diff_copy_end", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id,
                                "path": path.to_string_lossy(), "result_ok": result.is_ok(),
                                "recheck_returned_ok": rows.is_ok(),
                            })
                        });
                        if events
                            .send(Event::Updated {
                                id,
                                paths: vec![path.clone()],
                                rows,
                            })
                            .is_err()
                            || events
                                .send(Event::Text {
                                    id,
                                    path,
                                    view: result,
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                    Command::Scan {
                        id,
                        started,
                        cancel,
                        ignores,
                        parallelism,
                        mode,
                        quick_check,
                    } => {
                        profile.ignores = ignores;
                        profile.quick_check = quick_check;
                        pool.set_quick_check(quick_check);
                        comparison_mode = mode.for_checks(quick_check);
                        pool.set_parallelism(parallelism);
                        perform_scan(
                            id,
                            started,
                            &cancel,
                            &pool,
                            &profile,
                            &events,
                            comparison_mode,
                        );
                    }
                    Command::Diff { id, path, cancel } => {
                        diagnostics::record("diff_start", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id, "path": path.to_string_lossy(),
                            })
                        });
                        let view = (|| {
                            let (a, b) = pool.read_pair(&path, &cancel)?;
                            if cancel.load(Ordering::Relaxed) {
                                return Err("Diff cancelled".into());
                            }
                            Ok(text::diff_documents(path.display().to_string(), a, b))
                        })();
                        diagnostics::record("diff_end", || {
                            serde_json::json!({
                                "request_id": id, "pool_id": pool.diagnostic_id, "path": path.to_string_lossy(),
                                "result_ok": view.is_ok(), "decoded_text_has_changes": view.as_ref().ok()
                                    .map(|view| view.lines.iter().any(|line| line.changed)),
                            })
                        });
                        if events.send(Event::Text { id, path, view }).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            commands,
            events: responses,
            cancel,
        }
    }

    pub fn copy(
        &self,
        id: u64,
        path: PathBuf,
        view: TextView,
        selected: usize,
        to_right: bool,
        whole: bool,
    ) -> Result<(), String> {
        self.commands
            .send(Command::Copy {
                id,
                path,
                view: Box::new(view),
                selected,
                to_right,
                whole,
            })
            .map_err(|_| "Connection worker stopped".into())
    }

    pub fn copy_entries(
        &mut self,
        id: u64,
        paths: Vec<PathBuf>,
        to_right: bool,
        ignores: Vec<String>,
    ) -> Result<(), String> {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.commands
            .send(Command::CopyEntries {
                id,
                paths,
                to_right,
                ignores,
                cancel: self.cancel.clone(),
            })
            .map_err(|_| "Connection worker stopped".into())
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn refresh(
        &mut self,
        id: u64,
        ignores: Vec<String>,
        parallelism: Parallelism,
        mode: ComparisonMode,
        quick_check: QuickCheck,
    ) -> Result<(), String> {
        let started = Instant::now();
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.commands
            .send(Command::Scan {
                id,
                started,
                cancel: self.cancel.clone(),
                ignores,
                parallelism,
                mode,
                quick_check,
            })
            .map_err(|_| "Connection worker stopped; reopen the profile to reconnect".into())
    }

    pub fn diff(&mut self, id: u64, path: PathBuf) -> Result<(), String> {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.commands
            .send(Command::Diff {
                id,
                path,
                cancel: self.cancel.clone(),
            })
            .map_err(|_| "Connection worker stopped; reopen the profile to reconnect".into())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.commands.send(Command::Quit);
    }
}

fn copy_updates(
    pool: &Pool,
    paths: &[PathBuf],
    ignores: &[String],
    cancel: &Arc<AtomicBool>,
    mode: ComparisonMode,
) -> Result<Vec<Row>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Copy cancelled; affected paths need verification (r/F5)".into());
    }
    pool.scan_paths(mode, paths, ignores, cancel)
}

fn perform_scan(
    id: u64,
    started: Instant,
    cancel: &Arc<AtomicBool>,
    pool: &Pool,
    profile: &Profile,
    events: &Sender<Event>,
    mode: ComparisonMode,
) {
    diagnostics::record("scan_request", || {
        serde_json::json!({
            "request_id": id, "pool_id": pool.diagnostic_id, "mode": diagnostics::mode_name(mode),
            "quick_check": profile.quick_check,
        })
    });
    let mut last_progress = Instant::now() - Duration::from_secs(1);
    let result = pool.scan_with_mode(mode, &profile.ignores, cancel, |event| {
        if matches!(event, ScanEvent::Progress(_)) {
            if last_progress.elapsed() < Duration::from_millis(100) {
                return;
            }
            last_progress = Instant::now();
        }
        if events.send(Event::Scan { id, event }).is_err() {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    diagnostics::record("scan_request_end", || {
        serde_json::json!({
            "request_id": id, "pool_id": pool.diagnostic_id, "result_ok": result.is_ok(),
            "cancelled": cancel.load(Ordering::Relaxed), "elapsed_ms": started.elapsed().as_millis(),
        })
    });
    match result {
        Ok(rows) => {
            let _ = events.send(Event::Finished {
                id,
                rows,
                cancelled: cancel.load(Ordering::Relaxed),
                elapsed: started.elapsed(),
            });
        }
        Err(error) => {
            let _ = events.send(Event::Failed {
                id,
                error,
                elapsed: started.elapsed(),
            });
        }
    }
}

fn comparison_cache_path() -> Option<PathBuf> {
    // Individual cache tests use isolated databases; UI/worker unit tests must
    // not share a persistent database or touch the user's native cache.
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        match environment::init_cache_dir() {
            Ok(path) => path.map(|path| path.join("comparison-hashes.sqlite3")),
            Err(error) => {
                log::warn!("Cannot initialize comparison cache; using live reads: {error}");
                None
            }
        }
    }
}

fn build_endpoint(
    endpoint: &Endpoint,
    mut password: Secrets,
    keyring: bool,
) -> Result<Box<dyn CompareFs>, String> {
    if let Endpoint::Local { path } = endpoint {
        return Ok(Box::new(LocalFs::new(absolute_directory(path)?)));
    }
    crate::system::require_native_store(keyring)?;
    let username = whoami::username().map_err(|error| error.to_string())?;
    let vault = KeyringStorage::new(&username);
    if !crate::system::KeyStorage::is_supported(&vault) {
        return Err("Native credential store is unavailable or locked; unlock or configure it and retry. No fallback is supported.".into());
    }
    let supplied = credentials::supplied_secret(&password).map(str::to_owned);
    credentials::restore(&vault, endpoint, &mut password)?;
    let directory =
        environment::init_config_dir()?.ok_or("No configuration directory available")?;
    let (config_path, keys_path) = environment::get_config_paths(&directory);
    let config = ConfigClient::new(&config_path, &keys_path).map_err(|error| error.to_string())?;
    let settings = match endpoint {
        Endpoint::Connection { settings } => Some(settings.as_ref().clone()),
        Endpoint::Remote { url } => Some(ConnectionSettings::from_url(url)?),
        _ => None,
    };
    let mut transfer = if let Some(settings) = &settings {
        settings.transfer_params()
    } else if let Endpoint::Bookmark { name } = endpoint {
        let bookmarks = BookmarksClient::new(
            &environment::get_bookmarks_paths(&directory),
            &directory,
            16,
            keyring,
        )
        .map_err(|error| error.to_string())?;
        bookmarks
            .get_bookmark(name)
            .ok_or_else(|| format!("Bookmark not found: {name}"))?
    } else {
        return Err("Invalid remote endpoint".into());
    };
    let protocol = transfer.protocol;
    if protocol == FileTransferProtocol::Smb && !cfg!(smb) {
        return Err("SMB requires a build with the smb feature enabled".into());
    }
    if matches!(
        protocol,
        FileTransferProtocol::Sftp | FileTransferProtocol::Scp
    ) && let Some(settings) = &settings
        && let ProtocolParams::Generic(params) = &mut transfer.params
    {
        let ssh_config = config
            .get_ssh_config()
            .map(crate::utils::ssh::parse_ssh2_config)
            .transpose()?;
        let resolved =
            crate::utils::ssh::resolve_ssh_host_params(ssh_config.as_ref(), &params.address);
        params.port = settings.bookmark().port.unwrap_or(resolved.port);
        params.username = params.username.take().or(resolved.username);
    }
    password.apply(&mut transfer.params);
    // Resolve bookmarks/SSH settings before deriving the secret-free identity;
    // a bookmark name alone must never reuse hashes after its target changes.
    // Cache identity failures must not prevent otherwise valid legacy bookmark
    // connections. Disable caching if their settings cannot be normalized.
    let identity = ConnectionSettings::from_params(transfer.clone())
        .ok()
        .and_then(|(settings, _)| serde_json::to_vec(&settings).ok());
    let version_probe = match &transfer.params {
        ProtocolParams::WebDAV(params) => webdav_version::probe(params),
        _ => None,
    };
    let params = transfer.params;
    let root = transfer.remote_path.unwrap_or_else(|| "/".into());
    let client = RemoteFsBuilder::build(protocol, params, &config)?;
    let mut host = RemoteBridged::from(client);
    host.connect().map_err(|error| error.to_string())?;
    let root = match host.change_wrkdir(&root) {
        Ok(root) => root,
        Err(error) => {
            let _ = host.disconnect();
            return Err(error.to_string());
        }
    };
    if let Some(secret) = supplied
        && let Err(error) = credentials::remember(&vault, endpoint, &secret)
    {
        let _ = host.disconnect();
        return Err(error);
    }
    let namespace = identity.map(|mut identity| {
        identity.push(0);
        identity.extend_from_slice(root.as_os_str().as_encoded_bytes());
        cache::namespace(&identity)
    });
    Ok(Box::new(RemoteFs {
        host: Box::new(host),
        root,
        namespace,
        version_probe,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured_connections(
        profile: &Profile,
        credentials: Credentials,
    ) -> Vec<(Endpoint, Option<String>, Option<String>)> {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let (calls, observed) = mpsc::channel();
        let _pool = connect_pool(
            profile,
            credentials,
            &Arc::new(AtomicBool::new(false)),
            None,
            move |endpoint, secrets| {
                calls
                    .send((endpoint.clone(), secrets.password, secrets.security_token))
                    .unwrap();
                Ok(Box::new(LocalFs::new(root.clone())))
            },
        )
        .unwrap();
        let mut calls: Vec<_> = observed.try_iter().collect();
        calls.sort_by(|a, b| a.1.cmp(&b.1));
        calls
    }

    #[test]
    fn connects_both_typed_endpoints_independently_with_separate_credentials() {
        let folder = tempfile::tempdir().unwrap();
        let local = folder.path().to_str().unwrap();
        for source in [local, "sftp://source/folder", "bookmark:source"] {
            for destination in [local, "ftps://destination/folder", "bookmark:destination"] {
                let profile = Profile::new("test".into(), source, destination, vec![]).unwrap();
                let passwords = Credentials {
                    source: Secrets {
                        password: Some("source-secret".into()),
                        ..Secrets::default()
                    },
                    destination: Secrets {
                        password: Some("destination-secret".into()),
                        ..Secrets::default()
                    },
                };
                assert_eq!(
                    captured_connections(&profile, passwords),
                    vec![
                        (
                            profile.destination.clone(),
                            Some("destination-secret".into()),
                            None
                        ),
                        (profile.source.clone(), Some("source-secret".into()), None),
                    ]
                );
            }
        }
    }

    #[test]
    fn all_inherited_protocols_dispatch_on_either_side_without_crossing_credentials() {
        let folder = tempfile::tempdir().unwrap();
        let mut endpoints = vec![Endpoint::Local {
            path: folder.path().into(),
        }];
        endpoints.extend(
            super::super::connection::tests::fixtures()
                .into_iter()
                .map(|transfer| {
                    let (settings, _) = ConnectionSettings::from_params(transfer).unwrap();
                    Endpoint::Connection {
                        settings: Box::new(settings),
                    }
                }),
        );
        for source in &endpoints {
            for destination in &endpoints {
                let profile = Profile::from_endpoints(
                    "test".into(),
                    source.clone(),
                    destination.clone(),
                    vec![],
                )
                .unwrap();
                let credentials = Credentials {
                    source: Secrets {
                        password: Some("source".into()),
                        security_token: Some("source-token".into()),
                        ..Secrets::default()
                    },
                    destination: Secrets {
                        password: Some("destination".into()),
                        security_token: Some("destination-token".into()),
                        ..Secrets::default()
                    },
                };
                assert_eq!(
                    captured_connections(&profile, credentials),
                    vec![
                        (
                            profile.destination.clone(),
                            Some("destination".into()),
                            Some("destination-token".into())
                        ),
                        (
                            profile.source.clone(),
                            Some("source".into()),
                            Some("source-token".into())
                        ),
                    ]
                );
            }
        }
    }

    #[test]
    fn cancelled_connection_never_connects_another_endpoint() {
        let profile = Profile::new(
            "test".into(),
            "sftp://source/folder",
            "sftp://destination/folder",
            vec![],
        )
        .unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let observed = called.clone();
        assert!(
            connect_pool(
                &profile,
                Credentials::default(),
                &Arc::new(AtomicBool::new(true)),
                None,
                move |_, _| {
                    observed.store(true, Ordering::Relaxed);
                    Err("must not connect".into())
                }
            )
            .is_err()
        );
        assert!(!called.load(Ordering::Relaxed));
    }

    #[test]
    fn lazy_lanes_keep_credentials_separate_and_do_not_reuse_initial_cancellation() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..8 {
            std::fs::write(folder.path().join(format!("file{index}")), b"same").unwrap();
        }
        let root = folder.path().to_path_buf();
        let profile = Profile::new(
            "test".into(),
            "sftp://source/folder",
            "sftp://destination/folder",
            vec![],
        )
        .unwrap();
        let credentials = Credentials {
            source: Secrets {
                password: Some("source-secret".into()),
                ..Secrets::default()
            },
            destination: Secrets {
                password: Some("destination-secret".into()),
                ..Secrets::default()
            },
        };
        let (calls, observed) = mpsc::channel();
        let initial_cancel = Arc::new(AtomicBool::new(false));
        let pool = connect_pool(
            &profile,
            credentials,
            &initial_cancel,
            None,
            move |endpoint, secrets| {
                calls.send((endpoint.clone(), secrets.password)).unwrap();
                Ok(Box::new(LocalFs::new(root.clone())))
            },
        )
        .unwrap();
        initial_cancel.store(true, Ordering::Relaxed);
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert!(
            rows.iter()
                .all(|row| row.status == super::super::Status::Equal)
        );
        let calls: Vec<_> = observed.try_iter().collect();
        assert_eq!(calls.len(), 8);
        for (endpoint, password) in calls {
            let expected = if endpoint == profile.source {
                "source-secret"
            } else {
                assert_eq!(endpoint, profile.destination);
                "destination-secret"
            };
            assert_eq!(password.as_deref(), Some(expected));
        }
    }

    #[test]
    fn refresh_timing_starts_when_requested_not_when_the_worker_dequeues_it() {
        let (commands, requests) = mpsc::channel();
        let (_, events) = mpsc::channel();
        let mut worker = Worker {
            commands,
            events,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let before = Instant::now();
        let parallelism = Parallelism::try_from(8).unwrap();
        worker
            .refresh(
                7,
                vec![],
                parallelism,
                ComparisonMode::Full,
                QuickCheck::Timestamp,
            )
            .unwrap();
        let after = Instant::now();
        match requests.recv().unwrap() {
            Command::Scan {
                id,
                started,
                parallelism: requested,
                mode,
                quick_check,
                ..
            } => {
                assert_eq!(requested, parallelism);
                assert_eq!(mode, ComparisonMode::Full);
                assert_eq!(quick_check, QuickCheck::Timestamp);
                assert_eq!(id, 7);
                assert!(started >= before && started <= after);
            }
            _ => panic!("expected a scan request"),
        }
    }

    #[test]
    fn finished_events_report_request_to_full_scan_duration_even_when_cancelled() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let profile = Profile::new(
            "timing".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap();
        let a_root = a.path().to_path_buf();
        let b_root = b.path().to_path_buf();
        // Model a request already waiting on connections or previous I/O.
        let started = Instant::now() - Duration::from_secs(2);
        let pool = Pool::connect(
            move || Ok(Box::new(LocalFs::new(a_root.clone()))),
            move || Ok(Box::new(LocalFs::new(b_root.clone()))),
            None,
        )
        .unwrap();
        for cancelled in [false, true] {
            let (events, responses) = mpsc::channel();
            perform_scan(
                1,
                started,
                &Arc::new(AtomicBool::new(cancelled)),
                &pool,
                &profile,
                &events,
                ComparisonMode::Full,
            );
            let event = responses
                .try_iter()
                .find(|event| matches!(event, Event::Finished { .. }))
                .unwrap();
            let Event::Finished {
                elapsed,
                cancelled: reported,
                ..
            } = event
            else {
                unreachable!()
            };
            assert_eq!(reported, cancelled);
            assert!(elapsed >= Duration::from_secs(2));
            assert!(elapsed <= started.elapsed());
        }
    }

    #[test]
    fn local_worker_scans_refreshes_and_opens_diff() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("file"), "old\n").unwrap();
        std::fs::write(b.path().join("file"), "new\n").unwrap();
        for root in [a.path(), b.path()] {
            std::fs::OpenOptions::new()
                .write(true)
                .open(root.join("file"))
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(100))
                .unwrap();
        }
        let profile = Profile::new(
            "test".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap();
        let mut worker = Worker::start(
            profile,
            Credentials::default(),
            false,
            1,
            Parallelism::default(),
        );
        loop {
            if let Event::Finished { id, rows, .. } =
                worker.events.recv_timeout(Duration::from_secs(5)).unwrap()
            {
                assert_eq!(id, 1);
                assert_eq!(rows[0].status, super::super::Status::QuickMatch);
                break;
            }
        }
        worker.diff(2, PathBuf::from("file")).unwrap();
        match worker.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Text { id, view, .. } => {
                assert_eq!(id, 2);
                assert!(view.unwrap().lines[0].changed);
            }
            _ => panic!("expected diff"),
        }
        let snapshot = text::diff("file".into(), "old\n", "new\n");
        std::fs::write(b.path().join("file"), "external\n").unwrap();
        worker
            .copy(20, "file".into(), snapshot, 0, true, false)
            .unwrap();
        assert!(matches!(
            worker.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Updated { id: 20, .. }
        ));
        match worker.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Text { view, .. } => assert!(view.unwrap_err().contains("changed since")),
            _ => panic!("expected stale snapshot error"),
        }
        assert_eq!(
            std::fs::read_to_string(b.path().join("file")).unwrap(),
            "external\n"
        );
        let snapshot = text::diff("file".into(), "old\n", "external\n");
        worker
            .copy(21, "file".into(), snapshot, 0, true, false)
            .unwrap();
        assert!(matches!(
            worker.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Updated { id: 21, .. }
        ));
        match worker.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Text { view, .. } => {
                assert!(view.unwrap().lines.iter().all(|line| !line.changed))
            }
            _ => panic!("expected copied diff"),
        }
        assert_eq!(
            std::fs::read_to_string(b.path().join("file")).unwrap(),
            "old\n"
        );
        worker
            .refresh(
                3,
                vec!["file".into()],
                Parallelism::default(),
                ComparisonMode::Quick,
                QuickCheck::default(),
            )
            .unwrap();
        loop {
            if let Event::Finished { id, rows, .. } =
                worker.events.recv_timeout(Duration::from_secs(5)).unwrap()
            {
                assert_eq!(id, 3);
                assert!(rows.is_empty());
                break;
            }
        }
    }
}
