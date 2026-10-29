//! Each worker constructs and owns its filesystem clients; HostBridge need not be Send.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::connection::{ConnectionSettings, Credentials, Secrets};
use super::credentials;
use super::filesystem::{CompareFs, LocalFs, RemoteFs};
use super::profiles::{Endpoint, Profile, absolute_directory};
use super::text::{self, TextView};
use super::{Row, ScanEvent, scan};
use crate::filetransfer::params::ProtocolParams;
use crate::filetransfer::{FileTransferProtocol, RemoteFsBuilder};
use crate::host::{HostBridge, RemoteBridged};
use crate::system::KeyringStorage;
use crate::system::bookmarks_client::BookmarksClient;
use crate::system::config_client::ConfigClient;
use crate::system::environment;

pub enum Event {
    Scan {
        id: u64,
        event: ScanEvent,
    },
    Finished {
        id: u64,
        rows: Vec<Row>,
        cancelled: bool,
    },
    Text {
        id: u64,
        view: Result<TextView, String>,
    },
    Failed {
        id: u64,
        error: String,
    },
}

enum Command {
    Scan {
        id: u64,
        cancel: Arc<AtomicBool>,
        ignores: Vec<String>,
    },
    Diff {
        id: u64,
        path: PathBuf,
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

type Filesystems = (Box<dyn CompareFs>, Box<dyn CompareFs>);

fn connect_pair(
    profile: &Profile,
    passwords: Credentials,
    cancel: &AtomicBool,
    mut connect: impl FnMut(&Endpoint, Secrets) -> Result<Box<dyn CompareFs>, String>,
) -> Result<Filesystems, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Connection cancelled".into());
    }
    let source =
        connect(&profile.source, passwords.source).map_err(|error| format!("Source: {error}"))?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Connection cancelled".into());
    }
    let destination = connect(&profile.destination, passwords.destination)
        .map_err(|error| format!("Destination: {error}"))?;
    Ok((source, destination))
}

pub struct Worker {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(mut profile: Profile, passwords: Credentials, keyring: bool, id: u64) -> Self {
        let (commands, requests) = mpsc::channel();
        let (events, responses) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let initial_cancel = cancel.clone();
        thread::spawn(move || {
            let result = connect_pair(
                &profile,
                passwords,
                &initial_cancel,
                |endpoint, password| build_endpoint(endpoint, password, keyring),
            );
            let (mut left, mut right) = match result {
                Ok(pair) => pair,
                Err(error) => {
                    let _ = events.send(Event::Failed { id, error });
                    return;
                }
            };
            perform_scan(
                id,
                &initial_cancel,
                left.as_mut(),
                right.as_mut(),
                &profile,
                &events,
            );
            while let Ok(command) = requests.recv() {
                match command {
                    Command::Quit => break,
                    Command::Copy {
                        id,
                        path,
                        view,
                        selected,
                        to_right,
                        whole,
                    } => {
                        let result = (|| {
                            let cancel = AtomicBool::new(false);
                            let a = text::read_text(left.as_mut(), &path, &cancel)?;
                            let b = text::read_text(right.as_mut(), &path, &cancel)?;
                            if a.bytes != view.left_document.bytes
                                || b.bytes != view.right_document.bytes
                            {
                                return Err(
                                    "File changed since opening the diff; reopen it before copying"
                                        .into(),
                                );
                            }
                            let merged = view.merged_bytes(selected, to_right, whole)?;
                            if to_right {
                                right.write_bytes(&path, &merged)?;
                            } else {
                                left.write_bytes(&path, &merged)?;
                            }
                            let a = text::read_text(left.as_mut(), &path, &cancel)?;
                            let b = text::read_text(right.as_mut(), &path, &cancel)?;
                            Ok(text::diff_documents(path.display().to_string(), a, b))
                        })();
                        if events.send(Event::Text { id, view: result }).is_err() {
                            break;
                        }
                    }
                    Command::Scan {
                        id,
                        cancel,
                        ignores,
                    } => {
                        profile.ignores = ignores;
                        perform_scan(
                            id,
                            &cancel,
                            left.as_mut(),
                            right.as_mut(),
                            &profile,
                            &events,
                        );
                    }
                    Command::Diff { id, path, cancel } => {
                        let view = (|| {
                            let a = text::read_text(left.as_mut(), &path, &cancel)?;
                            let b = text::read_text(right.as_mut(), &path, &cancel)?;
                            if cancel.load(Ordering::Relaxed) {
                                return Err("Diff cancelled".into());
                            }
                            Ok(text::diff_documents(path.display().to_string(), a, b))
                        })();
                        if events.send(Event::Text { id, view }).is_err() {
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

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn refresh(&mut self, id: u64, ignores: Vec<String>) -> Result<(), String> {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.commands
            .send(Command::Scan {
                id,
                cancel: self.cancel.clone(),
                ignores,
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

fn perform_scan(
    id: u64,
    cancel: &AtomicBool,
    left: &mut dyn CompareFs,
    right: &mut dyn CompareFs,
    profile: &Profile,
    events: &Sender<Event>,
) {
    let mut last_progress = Instant::now() - Duration::from_secs(1);
    let rows = scan(left, right, &profile.ignores, cancel, |event| {
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
    let _ = events.send(Event::Finished {
        id,
        rows,
        cancelled: cancel.load(Ordering::Relaxed),
    });
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
    Ok(Box::new(RemoteFs {
        host: Box::new(host),
        root,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connects_both_typed_endpoints_independently_with_separate_credentials() {
        let folder = tempfile::tempdir().unwrap();
        let local = folder.path().to_str().unwrap();
        for source in [local, "sftp://source/folder", "bookmark:source"] {
            for destination in [local, "ftps://destination/folder", "bookmark:destination"] {
                let profile = Profile::new("test".into(), source, destination, vec![]).unwrap();
                let mut calls = Vec::new();
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
                let pair = connect_pair(
                    &profile,
                    passwords,
                    &AtomicBool::new(false),
                    |endpoint, password| {
                        calls.push((endpoint.clone(), password.password));
                        Ok(Box::new(LocalFs::new(folder.path().into())))
                    },
                )
                .unwrap();
                assert_eq!(
                    calls,
                    vec![
                        (profile.source.clone(), Some("source-secret".into())),
                        (
                            profile.destination.clone(),
                            Some("destination-secret".into())
                        )
                    ]
                );
                drop(pair);
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
                let mut calls = Vec::new();
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
                let _pair = connect_pair(
                    &profile,
                    credentials,
                    &AtomicBool::new(false),
                    |endpoint, secret| {
                        calls.push((endpoint.clone(), secret.password, secret.security_token));
                        Ok(Box::new(LocalFs::new(folder.path().into())))
                    },
                )
                .unwrap();
                assert_eq!(
                    calls,
                    vec![
                        (
                            profile.source.clone(),
                            Some("source".into()),
                            Some("source-token".into())
                        ),
                        (
                            profile.destination.clone(),
                            Some("destination".into()),
                            Some("destination-token".into())
                        )
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
        let mut called = false;
        assert!(
            connect_pair(
                &profile,
                Credentials::default(),
                &AtomicBool::new(true),
                |_, _| {
                    called = true;
                    Err("must not connect".into())
                }
            )
            .is_err()
        );
        assert!(!called);
    }

    #[test]
    fn local_worker_scans_refreshes_and_opens_diff() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("file"), "old\n").unwrap();
        std::fs::write(b.path().join("file"), "new\n").unwrap();
        let profile = Profile::new(
            "test".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap();
        let mut worker = Worker::start(profile, Credentials::default(), false, 1);
        loop {
            if let Event::Finished { id, rows, .. } =
                worker.events.recv_timeout(Duration::from_secs(5)).unwrap()
            {
                assert_eq!(id, 1);
                assert_eq!(rows[0].status, super::super::Status::Different);
                break;
            }
        }
        worker.diff(2, PathBuf::from("file")).unwrap();
        match worker.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Text { id, view } => {
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
        worker.refresh(3, vec!["file".into()]).unwrap();
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
