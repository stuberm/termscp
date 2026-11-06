use std::ffi::OsString;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

use super::*;
use crate::comparison::cache::{self, Namespace};
use crate::comparison::filesystem::{Entry, Fingerprint, Kind, LocalFs};
use crate::host::HostReader;

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    version: Option<String>,
}

struct TestFs {
    // Intentionally !Send: only the factory crosses the thread boundary.
    _not_send: Rc<()>,
    state: Arc<Mutex<State>>,
    opens: Arc<AtomicUsize>,
    namespace: Namespace,
    rendezvous: Option<Arc<Barrier>>,
    fail_listing: bool,
}

impl CompareFs for TestFs {
    fn cache_namespace(&self) -> Option<Namespace> {
        Some(self.namespace)
    }
    fn fingerprint(&mut self, _: &Path) -> Result<Option<Fingerprint>, String> {
        let state = self.state.lock().unwrap();
        Ok(Some(Fingerprint {
            kind: Kind::File,
            size: Some(state.bytes.len() as u64),
            modified: None,
            version: state.version.clone(),
        }))
    }
    fn list(&mut self, _: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        if let Some(barrier) = &self.rendezvous {
            barrier.wait();
        }
        if self.fail_listing {
            return Err("permission denied".into());
        }
        Ok(vec![(
            "file".into(),
            Entry {
                kind: Kind::File,
                size: Some(self.state.lock().unwrap().bytes.len() as u64),
                modified: None,
            },
        )])
    }
    fn open(&mut self, _: &Path) -> Result<HostReader, String> {
        if let Some(barrier) = &self.rendezvous {
            barrier.wait();
        }
        self.opens.fetch_add(1, Ordering::Relaxed);
        Ok(HostReader::io(Cursor::new(
            self.state.lock().unwrap().bytes.clone(),
        )))
    }
    fn write_bytes(&mut self, _: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if bytes == b"fail" {
            // Simulate a partial remote write whose ETag has not been updated.
            state.bytes = b"bad".to_vec();
            return Err("partial write".into());
        }
        state.bytes = bytes.into();
        state.version = Some("written".into());
        Ok(())
    }
}

fn state() -> Arc<Mutex<State>> {
    Arc::new(Mutex::new(State {
        bytes: b"abc".to_vec(),
        version: Some("v1".into()),
    }))
}

fn factory(
    state: Arc<Mutex<State>>,
    opens: Arc<AtomicUsize>,
    name: &[u8],
) -> impl Fn() -> Result<Box<dyn CompareFs>, String> + Send + Sync + 'static {
    let namespace = cache::namespace(name);
    move || {
        Ok(Box::new(TestFs {
            _not_send: Rc::new(()),
            state: state.clone(),
            opens: opens.clone(),
            namespace,
            rendezvous: None,
            fail_listing: false,
        }))
    }
}

#[test]
fn endpoint_listing_and_reads_run_concurrently_on_exactly_two_reused_owner_threads() {
    let barrier = Arc::new(Barrier::new(2));
    let creations = Arc::new(AtomicUsize::new(0));
    let opens = Arc::new(AtomicUsize::new(0));
    let make = || {
        let barrier = barrier.clone();
        let creations = creations.clone();
        let opens = opens.clone();
        move || -> Result<Box<dyn CompareFs>, String> {
            creations.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(TestFs {
                _not_send: Rc::new(()),
                state: state(),
                opens: opens.clone(),
                namespace: cache::namespace(b"same"),
                rendezvous: Some(barrier.clone()),
                fail_listing: false,
            }))
        }
    };
    let pool = Pool::connect(make(), make(), None).unwrap();
    for _ in 0..3 {
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(rows[0].status, Status::Equal);
    }
    assert_eq!(creations.load(Ordering::Relaxed), 2);
    assert_eq!(opens.load(Ordering::Relaxed), 6);
}

#[test]
fn cache_hits_require_fresh_versions_and_writes_invalidate_before_reuse() {
    let directory = tempfile::tempdir().unwrap();
    let source = state();
    let destination = state();
    let a_opens = Arc::new(AtomicUsize::new(0));
    let b_opens = Arc::new(AtomicUsize::new(0));
    let pool = Pool::connect(
        factory(source.clone(), a_opens.clone(), b"source"),
        factory(destination.clone(), b_opens.clone(), b"destination"),
        Some(directory.path().join("hashes.sqlite3")),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Equal
    );
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Equal
    );
    assert_eq!(a_opens.load(Ordering::Relaxed), 1);
    assert_eq!(b_opens.load(Ordering::Relaxed), 1);
    {
        let mut state = destination.lock().unwrap();
        state.bytes = b"xyz".to_vec();
        state.version = Some("v2".into());
    }
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Different
    );
    assert_eq!(a_opens.load(Ordering::Relaxed), 1);
    assert_eq!(b_opens.load(Ordering::Relaxed), 2);
    // Text reads must bypass the hash cache, even with unchanged versions.
    let (a, b) = pool.read_pair(Path::new("file"), &cancel).unwrap();
    assert_eq!(a.bytes, b"abc");
    assert_eq!(b.bytes, b"xyz");
    pool.write(1, Path::new("file"), b"abc".to_vec()).unwrap();
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Equal
    );
    assert_eq!(b_opens.load(Ordering::Relaxed), 4);
    assert!(pool.write(1, Path::new("file"), b"fail".to_vec()).is_err());
    assert_eq!(
        pool.scan(&[], &cancel, |_| {}).unwrap()[0].status,
        Status::Different
    );
    assert_eq!(b_opens.load(Ordering::Relaxed), 5);
}

#[test]
fn fresh_inventory_observes_additions_deletions_and_same_size_edits() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for directory in [a.path(), b.path()] {
        std::fs::write(directory.join("file"), b"abc").unwrap();
    }
    let a_root = a.path().to_path_buf();
    let b_root = b.path().to_path_buf();
    let pool = Pool::connect(
        move || Ok(Box::new(LocalFs::new(a_root.clone()))),
        move || Ok(Box::new(LocalFs::new(b_root.clone()))),
        Some(a.path().join("cache.sqlite3")),
    )
    .unwrap();
    // The database is inside the source only for this fixture; explicitly ignore it.
    let ignores = vec!["cache.sqlite3*".into()];
    let cancel = Arc::new(AtomicBool::new(false));
    assert_eq!(
        pool.scan(&ignores, &cancel, |_| {}).unwrap()[0].status,
        Status::Equal
    );
    let mtime = filetime::FileTime::from_last_modification_time(
        &std::fs::metadata(b.path().join("file")).unwrap(),
    );
    std::fs::write(b.path().join("file"), b"xyz").unwrap();
    filetime::set_file_mtime(b.path().join("file"), mtime).unwrap();
    assert_eq!(
        pool.scan(&ignores, &cancel, |_| {}).unwrap()[0].status,
        Status::Different
    );
    std::fs::remove_file(b.path().join("file")).unwrap();
    std::fs::write(b.path().join("added"), b"new").unwrap();
    let rows = pool.scan(&ignores, &cancel, |_| {}).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, Path::new("added"));
    assert_eq!(rows[0].status, Status::RightOnly);
    assert_eq!(rows[1].status, Status::LeftOnly);
}

#[test]
fn cancellation_and_failed_listings_do_not_imply_equality_or_missing_files() {
    let make = |fail_listing| {
        move || -> Result<Box<dyn CompareFs>, String> {
            Ok(Box::new(TestFs {
                _not_send: Rc::new(()),
                state: state(),
                opens: Arc::default(),
                namespace: cache::namespace(b"test"),
                rendezvous: None,
                fail_listing,
            }))
        }
    };
    let pool = Pool::connect(make(false), make(false), None).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let rows = pool
        .scan(&[], &cancel, |event| {
            if matches!(event, ScanEvent::Rows(_)) {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Pending));
    let pool = Pool::connect(make(false), make(true), None).unwrap();
    let rows = pool
        .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Error));
}
