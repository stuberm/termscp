//! A bounded, configurable pool of persistent independent endpoint pairs.
//! Clients (and remote streams) are constructed/used/dropped on their owner
//! thread; no Send requirement is imposed on protocol implementations.
#[cfg(test)]
#[path = "parallel_concurrency_tests.rs"]
mod concurrency_tests;
#[cfg(test)]
#[path = "parallel_tests.rs"]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::cache::HashCache;
use super::copy::{self, Prepared};
use super::diagnostics::{self, ScanTrace};
use super::filesystem::CompareFs;
use super::hashing::{self, Peer, Verification};
use super::settings::{Parallelism, QuickCheck};
use super::text::{self, Document};
use super::{ComparisonMode, Row, ScanEvent, Status, classify, finish_scan, inventory, quick};

type Reply<T> = Sender<Result<T, String>>;
type HashResult = Option<Verification>;
type HashResults = [Result<HashResult, String>; 2];
type Factory = Arc<dyn Fn() -> Result<Box<dyn CompareFs>, String> + Send + Sync>;
type Endpoints = [EndpointWorker; 2];

enum VerificationEvent {
    Started(usize),
    Finished(usize, HashResults),
}

enum Command {
    Inventory {
        side: usize,
        ignores: Vec<String>,
        cancel: Arc<AtomicBool>,
        events: SyncSender<InventoryEvent>,
        roots: Option<Vec<PathBuf>>,
    },
    Hash {
        path: PathBuf,
        cancel: Arc<AtomicBool>,
        peer: Peer,
        reply: Reply<HashResult>,
    },
    Text {
        path: PathBuf,
        cancel: Arc<AtomicBool>,
        reply: Reply<Document>,
    },
    PlanCopy {
        paths: Vec<PathBuf>,
        ignores: Vec<String>,
        cancel: Arc<AtomicBool>,
        reply: Reply<Vec<PathBuf>>,
    },
    PrepareCopy {
        path: PathBuf,
        cancel: Arc<AtomicBool>,
        reply: Reply<Prepared>,
    },
    InstallCopy {
        path: PathBuf,
        file: Option<std::fs::File>,
        reply: Reply<()>,
    },
    Write {
        path: PathBuf,
        bytes: Vec<u8>,
        reply: Reply<()>,
    },
}

enum InventoryEvent {
    Update {
        side: usize,
        event: ScanEvent,
    },
    Finished {
        side: usize,
        errors: BTreeMap<PathBuf, String>,
    },
}

pub(super) struct EndpointWorker {
    commands: SyncSender<Command>,
}

impl EndpointWorker {
    pub fn spawn(
        name: &str,
        factory: impl FnOnce() -> Result<Box<dyn CompareFs>, String> + Send + 'static,
        cache_path: Option<PathBuf>,
    ) -> Result<(Self, Receiver<Result<(), String>>), String> {
        // One active operation and at most one queued operation per endpoint.
        let (commands, requests) = mpsc::sync_channel(1);
        let (ready, connected) = mpsc::channel();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let mut fs = match factory() {
                    Ok(fs) => fs,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                // Finish cache setup before accepting work. With multiple
                // lanes, schema/WAL initialization must not race live writes
                // on already-ready connections to the same database.
                let mut cache = HashCache::open(cache_path.as_deref());
                if ready.send(Ok(())).is_err() {
                    return;
                }
                while let Ok(command) = requests.recv() {
                    match command {
                        Command::Inventory {
                            side,
                            ignores,
                            cancel,
                            events,
                            roots,
                        } => {
                            let mut discovered = BTreeMap::new();
                            let result = inventory(
                                fs.as_mut(),
                                &ignores,
                                &cancel,
                                &mut |event| {
                                    if events.send(InventoryEvent::Update { side, event }).is_err()
                                    {
                                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                    }
                                },
                                if side == 0 { "left" } else { "right" },
                                &mut discovered,
                                roots.as_deref(),
                            );
                            let _ = events.send(InventoryEvent::Finished {
                                side,
                                errors: result.errors,
                            });
                        }
                        Command::Hash {
                            path,
                            cancel,
                            peer,
                            reply,
                        } => {
                            let _ = reply.send(hashing::verify_with_peer(
                                fs.as_mut(),
                                &mut cache,
                                &path,
                                &cancel,
                                Some(peer),
                            ));
                        }
                        Command::Text {
                            path,
                            cancel,
                            reply,
                        } => {
                            let _ = reply.send(text::read_text(fs.as_mut(), &path, &cancel));
                        }
                        Command::PlanCopy {
                            paths,
                            ignores,
                            cancel,
                            reply,
                        } => {
                            let _ = reply.send(copy::plan(fs.as_mut(), paths, &ignores, &cancel));
                        }
                        Command::PrepareCopy {
                            path,
                            cancel,
                            reply,
                        } => {
                            let _ = reply.send(copy::prepare(fs.as_mut(), &path, &cancel));
                        }
                        Command::InstallCopy { path, file, reply } => {
                            if let Some(namespace) = fs.cache_namespace() {
                                cache.invalidate(&namespace, &path);
                            }
                            let result = match file {
                                Some(mut file) => copy::install(fs.as_mut(), &path, &mut file),
                                None => copy::ensure_directory(fs.as_mut(), &path),
                            };
                            let _ = reply.send(result);
                        }
                        Command::Write { path, bytes, reply } => {
                            // Invalidate before attempting a write: a failed remote write can
                            // leave changed/partial bytes too. Text reads never use the cache.
                            if let Some(namespace) = fs.cache_namespace() {
                                cache.invalidate(&namespace, &path);
                            }
                            let _ = reply.send(fs.write_bytes(&path, &bytes));
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok((Self { commands }, connected))
    }

    fn request<T>(
        &self,
        command: impl FnOnce(Reply<T>) -> Command,
    ) -> Result<Receiver<Result<T, String>>, String> {
        let (reply, response) = mpsc::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| "Endpoint worker stopped".to_owned())?;
        Ok(response)
    }
}

fn receive<T>(response: Receiver<Result<T, String>>) -> Result<T, String> {
    response
        .recv()
        .map_err(|_| "Endpoint worker stopped".to_owned())?
}

pub(super) struct Pool {
    endpoints: Endpoints,
    pub(super) diagnostic_id: u64,
    factories: [Factory; 2],
    cache_path: Option<PathBuf>,
    parallelism: Parallelism,
    quick_check: QuickCheck,
    // The scan coordinator serializes scans with this lock. Endpoint clients
    // themselves remain confined to their original owner threads.
    extra_pairs: Mutex<Vec<Endpoints>>,
}

impl Pool {
    pub fn connect(
        source: impl Fn() -> Result<Box<dyn CompareFs>, String> + Send + Sync + 'static,
        destination: impl Fn() -> Result<Box<dyn CompareFs>, String> + Send + Sync + 'static,
        cache_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let factories: [Factory; 2] = [Arc::new(source), Arc::new(destination)];
        let endpoints = Self::connect_pair(&factories, cache_path.clone())?;
        Ok(Self {
            endpoints,
            diagnostic_id: diagnostics::new_id(),
            factories,
            cache_path,
            parallelism: Parallelism::default(),
            quick_check: QuickCheck::default(),
            extra_pairs: Mutex::new(Vec::new()),
        })
    }

    pub fn set_parallelism(&mut self, parallelism: Parallelism) {
        self.parallelism = parallelism;
    }

    pub fn set_quick_check(&mut self, checks: QuickCheck) {
        self.quick_check = checks;
    }

    fn connect_pair(
        factories: &[Factory; 2],
        cache_path: Option<PathBuf>,
    ) -> Result<Endpoints, String> {
        let [source, destination] = factories.clone();
        let (a, source_ready) =
            EndpointWorker::spawn("compare-source", move || source(), cache_path.clone())?;
        let (b, destination_ready) =
            EndpointWorker::spawn("compare-destination", move || destination(), cache_path)?;
        // Connect concurrently and drain both replies even if one fails.
        let source = receive(source_ready).map_err(|error| format!("Source: {error}"));
        let destination =
            receive(destination_ready).map_err(|error| format!("Destination: {error}"));
        source?;
        destination?;
        Ok([a, b])
    }

    fn verify_pair(endpoints: &Endpoints, path: &Path, cancel: &Arc<AtomicBool>) -> HashResults {
        let [a_peer, b_peer] = Peer::pair();
        let a = endpoints[0].request(|reply| Command::Hash {
            path: path.into(),
            cancel: cancel.clone(),
            peer: a_peer,
            reply,
        });
        let b = endpoints[1].request(|reply| Command::Hash {
            path: path.into(),
            cancel: cancel.clone(),
            peer: b_peer,
            reply,
        });
        // A failed enqueue drops its peer, unblocking the other endpoint. Always
        // drain both streams before this lane is reused or the scan returns.
        [a.and_then(receive), b.and_then(receive)]
    }

    pub fn read_pair(
        &self,
        path: &Path,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(Document, Document), String> {
        let a = self.endpoints[0].request(|reply| Command::Text {
            path: path.into(),
            cancel: cancel.clone(),
            reply,
        })?;
        let b = self.endpoints[1].request(|reply| Command::Text {
            path: path.into(),
            cancel: cancel.clone(),
            reply,
        })?;
        // Drain both replies even if one side fails, so all streams finish before
        // the next command (especially a copy or a refresh).
        let a = receive(a);
        let b = receive(b);
        Ok((a?, b?))
    }

    pub fn write(&self, side: usize, path: &Path, bytes: Vec<u8>) -> Result<(), String> {
        receive(self.endpoints[side].request(|reply| Command::Write {
            path: path.into(),
            bytes,
            reply,
        })?)
    }

    pub fn copy_paths(
        &self,
        paths: Vec<PathBuf>,
        to_right: bool,
        ignores: &[String],
        cancel: &Arc<AtomicBool>,
    ) -> Result<usize, String> {
        let target = usize::from(to_right);
        let paths = receive(
            self.endpoints[1 - target].request(|reply| Command::PlanCopy {
                paths,
                ignores: ignores.into(),
                cancel: cancel.clone(),
                reply,
            })?,
        )?;
        let mut copied = 0;
        for path in paths {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(format!(
                    "Copy cancelled after {copied} entries; no rollback"
                ));
            }
            let result = (|| {
                let prepared =
                    receive(
                        self.endpoints[1 - target].request(|reply| Command::PrepareCopy {
                            path: path.clone(),
                            cancel: cancel.clone(),
                            reply,
                        })?,
                    )?;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Copy cancelled".to_owned());
                }
                let file = match prepared {
                    Prepared::File(file) => Some(file),
                    Prepared::Directory => None,
                };
                receive(
                    self.endpoints[target].request(|reply| Command::InstallCopy {
                        path: path.clone(),
                        file,
                        reply,
                    })?,
                )
            })();
            if let Err(error) = result {
                return Err(format!(
                    "Copy stopped at {} after {copied} entries: {error}. Earlier copies are not rolled back",
                    path.display()
                ));
            }
            copied += 1;
        }
        Ok(copied)
    }

    /// Full-scan shorthand retained for content-verification regression tests.
    #[cfg(test)]
    pub fn scan(
        &self,
        ignores: &[String],
        cancel: &Arc<AtomicBool>,
        emit: impl FnMut(ScanEvent),
    ) -> Result<Vec<Row>, String> {
        self.scan_with_mode(ComparisonMode::Full, ignores, cancel, emit)
    }

    pub fn scan_with_mode(
        &self,
        mode: ComparisonMode,
        ignores: &[String],
        cancel: &Arc<AtomicBool>,
        emit: impl FnMut(ScanEvent),
    ) -> Result<Vec<Row>, String> {
        self.scan_from(mode, ignores, cancel, None, emit)
    }

    /// Recheck copied entries only; a file never recursively lists its siblings.
    pub fn scan_paths(
        &self,
        mode: ComparisonMode,
        paths: &[PathBuf],
        ignores: &[String],
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<Row>, String> {
        let mut roots = Vec::<PathBuf>::new();
        let paths: std::collections::BTreeSet<_> = paths.iter().cloned().collect();
        for path in paths {
            copy::validate(&path)?;
            if !roots.iter().any(|root| path.starts_with(root)) {
                roots.push(path);
            }
        }
        self.scan_from(mode, ignores, cancel, Some(&roots), |_| {})
    }

    fn scan_from(
        &self,
        mode: ComparisonMode,
        ignores: &[String],
        cancel: &Arc<AtomicBool>,
        roots: Option<&[PathBuf]>,
        mut emit: impl FnMut(ScanEvent),
    ) -> Result<Vec<Row>, String> {
        let mode = mode.for_checks(self.quick_check);
        let mut trace =
            ScanTrace::begin(self.diagnostic_id, mode, roots, ignores, self.quick_check);
        let mut extra_pairs = self.extra_pairs.lock().map_err(|error| error.to_string())?;
        // Scans are serialized; no lane is active when excess connections drop.
        extra_pairs.truncate(self.parallelism.get() - 1);
        // Backpressure bounds discovery events, independently of the tree size.
        let (events, updates) = mpsc::sync_channel(8);
        for (side, endpoint) in self.endpoints.iter().enumerate() {
            endpoint
                .commands
                .send(Command::Inventory {
                    side,
                    ignores: ignores.into(),
                    cancel: cancel.clone(),
                    events: events.clone(),
                    roots: roots.map(<[PathBuf]>::to_vec),
                })
                .map_err(|_| "Endpoint worker stopped".to_owned())?;
        }
        drop(events);
        let mut discovered: BTreeMap<PathBuf, Row> = BTreeMap::new();
        let mut errors = [None, None];
        while errors.iter().any(Option::is_none) {
            match updates
                .recv()
                .map_err(|_| "Endpoint worker stopped during inventory".to_owned())?
            {
                InventoryEvent::Finished {
                    side,
                    errors: result,
                } => {
                    errors[side] = Some(result);
                }
                InventoryEvent::Update {
                    side,
                    event: ScanEvent::Rows(rows),
                } => {
                    let mut merged = Vec::with_capacity(rows.len());
                    for row in rows {
                        let current = discovered.entry(row.path.clone()).or_insert_with(|| Row {
                            path: row.path.clone(),
                            left: None,
                            right: None,
                            status: Status::Pending,
                            detail: String::new(),
                            aggregate_base: None,
                        });
                        if side == 0 {
                            current.left = row.left;
                        } else {
                            current.right = row.right;
                        }
                        if row.status == Status::Error {
                            current.status = row.status;
                            current.detail = row.detail;
                        }
                        merged.push(current.clone());
                    }
                    emit(ScanEvent::Rows(merged));
                }
                InventoryEvent::Update { event, .. } => emit(event),
            }
        }
        let [left_errors, right_errors] = errors.map(Option::unwrap);
        let mut rows: Vec<_> = discovered.into_values().collect();
        let mut batch = Vec::new();
        let mut last_update = Instant::now();
        let mut candidates = Vec::new();
        for (index, row) in rows.iter_mut().enumerate() {
            let verify = classify(row, &left_errors, &right_errors, cancel);
            if verify && mode == ComparisonMode::Full {
                candidates.push((index, row.path.clone()));
            } else {
                if verify {
                    quick::compare_with(row, self.quick_check);
                }
                if let Some(trace) = &trace {
                    trace.row("classified", row);
                }
                batch.push(row.clone());
                if batch.len() == 64 {
                    emit(ScanEvent::Rows(std::mem::take(&mut batch)));
                }
            }
        }
        // Only connect extra lanes when there is work for them. A server may
        // limit simultaneous sessions: keep using healthy lanes if expansion
        // fails, rather than failing an otherwise usable comparison.
        let lanes = candidates.len().min(self.parallelism.get());
        while extra_pairs.len() + 1 < lanes && !cancel.load(Ordering::Relaxed) {
            emit(ScanEvent::Progress("Connecting comparison workers".into()));
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            match Self::connect_pair(&self.factories, self.cache_path.clone()) {
                Ok(pair) => extra_pairs.push(pair),
                Err(error) => {
                    log::warn!(
                        "Additional comparison connections unavailable; using fewer file pairs: {error}"
                    );
                    break;
                }
            }
        }
        let next = AtomicUsize::new(0);
        thread::scope(|scope| {
            let (events, results) = mpsc::sync_channel(8);
            for endpoints in std::iter::once(&self.endpoints)
                .chain(extra_pairs.iter())
                .take(lanes)
            {
                let events = events.clone();
                let next = &next;
                let candidates = &candidates;
                scope.spawn(move || {
                    while !cancel.load(Ordering::Relaxed) {
                        let Some((index, path)) =
                            candidates.get(next.fetch_add(1, Ordering::Relaxed))
                        else {
                            break;
                        };
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if events.send(VerificationEvent::Started(*index)).is_err() {
                            break;
                        }
                        let result = Self::verify_pair(endpoints, path, cancel);
                        if events
                            .send(VerificationEvent::Finished(*index, result))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
            drop(events);
            // Receive in completion order, so a slow file never hides other
            // lanes' results. Unassigned/cancelled files remain pending.
            loop {
                let event = match results.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !batch.is_empty() {
                            emit(ScanEvent::Rows(std::mem::take(&mut batch)));
                            last_update = Instant::now();
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                match event {
                    VerificationEvent::Started(index) => emit(ScanEvent::Progress(format!(
                        "Verifying {}",
                        rows[index].path.display()
                    ))),
                    VerificationEvent::Finished(index, [a, b]) => {
                        let row = &mut rows[index];
                        row.status = match (a, b) {
                            (Err(error), _) | (_, Err(error)) => {
                                row.detail = error;
                                Status::Error
                            }
                            (
                                Ok(Some(Verification::Complete(a))),
                                Ok(Some(Verification::Complete(b))),
                            ) => {
                                if a.cached || b.cached {
                                    row.detail = "SHA-256 verified; cached hashes validated by live strong content versions".into();
                                }
                                if a.hash.digest == b.hash.digest {
                                    Status::Equal
                                } else {
                                    Status::Different
                                }
                            }
                            (Ok(Some(_)), Ok(Some(_))) => Status::Different,
                            _ => Status::Pending,
                        };
                        batch.push(row.clone());
                    }
                }
                if batch.len() >= 64 || last_update.elapsed() >= Duration::from_millis(100) {
                    emit(ScanEvent::Rows(std::mem::take(&mut batch)));
                    last_update = Instant::now();
                }
            }
        });
        if !batch.is_empty() {
            emit(ScanEvent::Rows(batch));
        }
        if mode == ComparisonMode::Quick && !cancel.load(Ordering::Relaxed) {
            for row in &mut rows {
                if row.status == Status::Pending && row.is_directory() {
                    row.status = Status::QuickMatch;
                    row.detail =
                        "Quick comparison of descendants; file contents not verified".into();
                }
            }
        }
        finish_scan(&mut rows, cancel);
        if let Some(trace) = &mut trace {
            trace.finish(&rows, cancel.load(Ordering::Relaxed));
        }
        Ok(rows)
    }
}
