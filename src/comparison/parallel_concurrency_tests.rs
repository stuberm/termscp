//! Exercise scheduling with !Send clients and finalized remote streams.
use std::ffi::OsString;
use std::io::{self, Cursor, Read};
use std::rc::Rc;
use std::sync::Condvar;
use std::thread::ThreadId;

use remotefs::fs::{ReadStream, RemoteRead};

use super::*;
use crate::comparison::cache;
use crate::comparison::filesystem::{Entry, Fingerprint, Kind, LocalFs};
use crate::host::HostReader;

// These existing regression tests exercise the built-in four-pair default.
const DEFAULT_FILE_PAIRS: usize = 4;

#[derive(Default)]
struct Metrics {
    connections: [AtomicUsize; 2],
    active: AtomicUsize,
    peak: AtomicUsize,
    opens: AtomicUsize,
    finishes: AtomicUsize,
}

#[derive(Default)]
struct Gate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}

impl Gate {
    fn wait(&self, participants: Option<usize>) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        if participants == Some(state.0) {
            state.1 = true;
            self.changed.notify_all();
        }
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.1)
            .unwrap();
        if state.1 {
            Ok(())
        } else {
            Err("Timed out waiting for concurrent work".into())
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

#[derive(Clone, Default)]
struct Settings {
    files: usize,
    rendezvous: Option<Arc<Gate>>,
    rendezvous_pairs: Option<usize>,
    slow: Option<Arc<Gate>>,
    cancel_on_open: Option<Arc<AtomicBool>>,
    connection_limit: Option<usize>,
    fail_open: Option<usize>,
    fail_finish: Option<usize>,
    mixed_contents: bool,
}

struct Fs {
    _not_send: Rc<()>,
    owner: ThreadId,
    side: usize,
    settings: Settings,
    metrics: Arc<Metrics>,
}

impl Drop for Fs {
    fn drop(&mut self) {
        assert_eq!(self.owner, thread::current().id());
    }
}

impl CompareFs for Fs {
    fn list(&mut self, _: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        assert_eq!(self.owner, thread::current().id());
        Ok((0..self.settings.files)
            .map(|index| {
                (
                    format!("file{index:02}").into(),
                    Entry {
                        kind: Kind::File,
                        size: None,
                        modified: None,
                    },
                )
            })
            .collect())
    }

    fn open(&mut self, path: &Path) -> Result<HostReader, String> {
        assert_eq!(self.owner, thread::current().id());
        let index: usize = path.to_str().unwrap()[4..].parse().unwrap();
        if self.settings.fail_open == Some(index) {
            return Err("open failed".into());
        }
        let active = self.metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.metrics.peak.fetch_max(active, Ordering::SeqCst);
        self.metrics.opens.fetch_add(1, Ordering::SeqCst);
        let wait = (|| {
            if let Some(gate) = &self.settings.rendezvous {
                gate.wait(Some(
                    self.settings.rendezvous_pairs.unwrap_or(DEFAULT_FILE_PAIRS) * 2,
                ))?;
            }
            if index == 0
                && let Some(gate) = &self.settings.slow
            {
                gate.wait(None)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = wait {
            self.metrics.active.fetch_sub(1, Ordering::SeqCst);
            return Err(error);
        }
        if let Some(cancel) = &self.settings.cancel_on_open {
            cancel.store(true, Ordering::SeqCst);
        }
        let bytes = if self.settings.mixed_contents && index % 3 == 1 && self.side == 1 {
            b"different\n".to_vec()
        } else if self.side == 0 {
            b"same\r\n".to_vec()
        } else {
            b"same\n".to_vec()
        };
        Ok(HostReader::remote(ReadStream::new(Reader {
            owner: self.owner,
            bytes: Cursor::new(bytes),
            metrics: self.metrics.clone(),
            fail_finish: self.settings.fail_finish == Some(index),
        })))
    }
}

struct Reader {
    owner: ThreadId,
    bytes: Cursor<Vec<u8>>,
    metrics: Arc<Metrics>,
    fail_finish: bool,
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert_eq!(self.owner, thread::current().id());
        self.bytes.read(buffer)
    }
}

impl RemoteRead for Reader {
    fn finish(self: Box<Self>) -> remotefs::RemoteResult<()> {
        assert_eq!(self.owner, thread::current().id());
        self.metrics.active.fetch_sub(1, Ordering::SeqCst);
        self.metrics.finishes.fetch_add(1, Ordering::SeqCst);
        if self.fail_finish {
            return Err(remotefs::RemoteError::new(
                remotefs::RemoteErrorType::ProtocolError,
            ));
        }
        Ok(())
    }
}

fn factory(
    side: usize,
    settings: Settings,
    metrics: &Arc<Metrics>,
) -> impl Fn() -> Result<Box<dyn CompareFs>, String> + Send + Sync + 'static {
    let metrics = metrics.clone();
    move || {
        let attempt = metrics.connections[side].fetch_add(1, Ordering::SeqCst);
        if settings
            .connection_limit
            .is_some_and(|limit| attempt >= limit)
        {
            return Err("server connection limit".into());
        }
        Ok(Box::new(Fs {
            _not_send: Rc::new(()),
            owner: thread::current().id(),
            side,
            settings: settings.clone(),
            metrics: metrics.clone(),
        }))
    }
}

#[test]
fn four_pairs_overlap_without_exceeding_the_bound_and_connections_are_reused() {
    let metrics = Arc::new(Metrics::default());
    let settings = Settings {
        files: 12,
        rendezvous: Some(Arc::new(Gate::default())),
        mixed_contents: true,
        ..Settings::default()
    };
    let pool = Pool::connect(
        factory(0, settings.clone(), &metrics),
        factory(1, settings, &metrics),
        None,
    )
    .unwrap();
    for _ in 0..3 {
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(rows.len(), 12);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.path, PathBuf::from(format!("file{index:02}")));
            assert_eq!(
                row.status,
                if index % 3 == 1 {
                    Status::Different
                } else {
                    Status::Equal
                }
            );
        }
        assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
    }
    assert_eq!(metrics.peak.load(Ordering::SeqCst), DEFAULT_FILE_PAIRS * 2);
    assert_eq!(metrics.opens.load(Ordering::SeqCst), 72);
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 72);
    for connections in &metrics.connections {
        assert_eq!(connections.load(Ordering::SeqCst), DEFAULT_FILE_PAIRS);
    }
}

#[test]
fn configured_limits_control_actual_parallel_file_pairs() {
    for limit in [1, 2, 8, 16] {
        let metrics = Arc::new(Metrics::default());
        let settings = Settings {
            files: limit + 4,
            rendezvous: Some(Arc::new(Gate::default())),
            rendezvous_pairs: Some(limit),
            ..Settings::default()
        };
        let mut pool = Pool::connect(
            factory(0, settings.clone(), &metrics),
            factory(1, settings, &metrics),
            None,
        )
        .unwrap();
        pool.set_parallelism(Parallelism::try_from(limit).unwrap());
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert!(rows.iter().all(|row| row.status == Status::Equal));
        assert_eq!(metrics.peak.load(Ordering::SeqCst), limit * 2);
        assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.finishes.load(Ordering::SeqCst), (limit + 4) * 2);
        for connections in &metrics.connections {
            assert_eq!(connections.load(Ordering::SeqCst), limit);
        }
    }
}

#[test]
fn changing_limits_on_refresh_reuses_or_releases_extra_connections() {
    let metrics = Arc::new(Metrics::default());
    let settings = Settings {
        files: 24,
        ..Settings::default()
    };
    let mut pool = Pool::connect(
        factory(0, settings.clone(), &metrics),
        factory(1, settings, &metrics),
        None,
    )
    .unwrap();
    for (limit, creations) in [(1, 1), (8, 8), (2, 8), (16, 22), (1, 22)] {
        metrics.peak.store(0, Ordering::SeqCst);
        pool.set_parallelism(Parallelism::try_from(limit).unwrap());
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert!(rows.iter().all(|row| row.status == Status::Equal));
        assert!(metrics.peak.load(Ordering::SeqCst) <= limit * 2);
        assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
        assert_eq!(pool.extra_pairs.lock().unwrap().len(), limit - 1);
        for connections in &metrics.connections {
            assert_eq!(connections.load(Ordering::SeqCst), creations);
        }
    }
}

#[test]
fn extra_connections_are_created_only_when_there_are_enough_file_pairs() {
    for files in 0..=5 {
        let metrics = Arc::new(Metrics::default());
        let settings = Settings {
            files,
            ..Settings::default()
        };
        let pool = Pool::connect(
            factory(0, settings.clone(), &metrics),
            factory(1, settings, &metrics),
            None,
        )
        .unwrap();
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(rows.len(), files);
        assert!(rows.iter().all(|row| row.status == Status::Equal));
        for connections in &metrics.connections {
            assert_eq!(
                connections.load(Ordering::SeqCst),
                files.clamp(1, DEFAULT_FILE_PAIRS)
            );
        }
    }
}

#[test]
fn strong_hash_cache_is_shared_across_lanes_but_still_requires_live_versions() {
    struct VersionedFs {
        inner: Box<dyn CompareFs>,
        side: u8,
        version: Arc<AtomicUsize>,
    }
    impl CompareFs for VersionedFs {
        fn cache_namespace(&self) -> Option<cache::Namespace> {
            Some(cache::namespace(&[self.side]))
        }
        fn fingerprint(&mut self, _: &Path) -> Result<Option<Fingerprint>, String> {
            Ok(Some(Fingerprint {
                kind: Kind::File,
                size: None,
                modified: None,
                version: Some(self.version.load(Ordering::SeqCst).to_string()),
            }))
        }
        fn list(&mut self, path: &Path) -> Result<Vec<(OsString, Entry)>, String> {
            self.inner.list(path)
        }
        fn open(&mut self, path: &Path) -> Result<HostReader, String> {
            self.inner.open(path)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let metrics = Arc::new(Metrics::default());
    let version = Arc::new(AtomicUsize::new(1));
    let make = |side| {
        let inner = factory(
            side,
            Settings {
                files: 12,
                ..Settings::default()
            },
            &metrics,
        );
        let version = version.clone();
        move || -> Result<Box<dyn CompareFs>, String> {
            Ok(Box::new(VersionedFs {
                inner: inner()?,
                side: side as u8,
                version: version.clone(),
            }))
        }
    };
    let pool = Pool::connect(
        make(0),
        make(1),
        Some(directory.path().join("hashes.sqlite3")),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    for revision in 1..=2 {
        version.store(revision, Ordering::SeqCst);
        let rows = pool.scan(&[], &cancel, |_| {}).unwrap();
        assert!(rows.iter().all(|row| row.status == Status::Equal));
        let reads = metrics.opens.load(Ordering::SeqCst);
        assert_eq!(reads, revision * 24);
        let rows = pool.scan(&[], &cancel, |_| {}).unwrap();
        assert!(
            rows.iter()
                .all(|row| row.status == Status::Equal && row.detail.contains("cached hashes"))
        );
        assert_eq!(metrics.opens.load(Ordering::SeqCst), reads);
    }
    assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
}

#[test]
fn fast_results_are_emitted_while_another_pair_is_still_blocked() {
    let metrics = Arc::new(Metrics::default());
    let gate = Arc::new(Gate::default());
    let settings = Settings {
        files: 8,
        slow: Some(gate.clone()),
        ..Settings::default()
    };
    let pool = Pool::connect(
        factory(0, settings.clone(), &metrics),
        factory(1, settings, &metrics),
        None,
    )
    .unwrap();
    let mut observed = false;
    let rows = pool
        .scan(&[], &Arc::new(AtomicBool::new(false)), |event| {
            if let ScanEvent::Rows(rows) = event
                && rows
                    .iter()
                    .any(|row| row.status == Status::Equal && row.path != Path::new("file00"))
            {
                observed = true;
                gate.release();
            }
        })
        .unwrap();
    assert!(observed);
    assert!(rows.iter().all(|row| row.status == Status::Equal));
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 16);
    assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_drains_all_active_streams_and_refresh_reuses_the_pool() {
    let metrics = Arc::new(Metrics::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let settings = Settings {
        files: 12,
        rendezvous: Some(Arc::new(Gate::default())),
        cancel_on_open: Some(cancel.clone()),
        ..Settings::default()
    };
    let pool = Pool::connect(
        factory(0, settings.clone(), &metrics),
        factory(1, settings, &metrics),
        None,
    )
    .unwrap();
    let rows = pool.scan(&[], &cancel, |_| {}).unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Pending));
    assert_eq!(metrics.opens.load(Ordering::SeqCst), 8);
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 8);
    assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
    let rows = pool
        .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Equal));
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 32);
    assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
    for connections in &metrics.connections {
        assert_eq!(connections.load(Ordering::SeqCst), 4);
    }
}

#[test]
fn additional_connection_failure_falls_back_to_the_existing_pair() {
    let metrics = Arc::new(Metrics::default());
    let source = Settings {
        files: 12,
        ..Settings::default()
    };
    let destination = Settings {
        connection_limit: Some(1),
        ..source.clone()
    };
    let pool = Pool::connect(
        factory(0, source, &metrics),
        factory(1, destination, &metrics),
        None,
    )
    .unwrap();
    for _ in 0..2 {
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(rows.len(), 12);
        assert!(rows.iter().all(|row| row.status == Status::Equal));
        assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
    }
    assert!(metrics.peak.load(Ordering::SeqCst) <= 2);
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 48);
}

#[test]
fn one_pair_failure_does_not_hide_other_results_or_leave_streams_active() {
    let metrics = Arc::new(Metrics::default());
    let source = Settings {
        files: 12,
        fail_open: Some(0),
        ..Settings::default()
    };
    let destination = Settings {
        fail_open: None,
        fail_finish: Some(1),
        ..source.clone()
    };
    let pool = Pool::connect(
        factory(0, source, &metrics),
        factory(1, destination, &metrics),
        None,
    )
    .unwrap();
    let rows = pool
        .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
        .unwrap();
    assert_eq!(rows[0].status, Status::Error);
    assert_eq!(rows[1].status, Status::Error);
    assert!(rows[2..].iter().all(|row| row.status == Status::Equal));
    assert_eq!(metrics.finishes.load(Ordering::SeqCst), 23);
    assert_eq!(metrics.active.load(Ordering::SeqCst), 0);
}

#[test]
fn multi_lane_refresh_observes_confirmed_copies_and_external_edits() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    for index in 0..12 {
        let name = format!("file{index:02}");
        std::fs::write(left.path().join(&name), "same\r\n").unwrap();
        std::fs::write(right.path().join(&name), "same\n").unwrap();
    }
    let source = left.path().to_path_buf();
    let destination = right.path().to_path_buf();
    let pool = Pool::connect(
        move || Ok(Box::new(LocalFs::new(source.clone()))),
        move || Ok(Box::new(LocalFs::new(destination.clone()))),
        None,
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    assert!(
        pool.scan(&[], &cancel, |_| {})
            .unwrap()
            .iter()
            .all(|row| row.status == Status::Equal)
    );
    std::fs::write(left.path().join("file00"), "new\n").unwrap();
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Different
    );
    pool.copy_paths(vec!["file00".into()], true, &[], &cancel)
        .unwrap();
    let rows = pool.scan(&[], &cancel, |_| {}).unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Equal));
    assert_eq!(
        std::fs::read(right.path().join("file00")).unwrap(),
        b"new\n"
    );
}
