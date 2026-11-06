//! Scoped copy rechecks must never traverse or read unrelated file trees.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::filesystem::{CompareFs, Entry, Fingerprint, LocalFs};
use super::parallel::Pool;
use super::*;
use crate::host::HostReader;

#[derive(Default)]
struct Calls {
    connections: AtomicUsize,
    lists: Mutex<Vec<PathBuf>>,
    opens: Mutex<Vec<PathBuf>>,
}

struct Tracked {
    fs: LocalFs,
    calls: Arc<Calls>,
}

impl CompareFs for Tracked {
    fn list(&mut self, path: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        assert!(
            !path.starts_with("unrelated"),
            "unrelated recursive listing"
        );
        self.calls.lists.lock().unwrap().push(path.into());
        self.fs.list(path)
    }
    fn fingerprint(&mut self, path: &Path) -> Result<Option<Fingerprint>, String> {
        self.fs.fingerprint(path)
    }
    fn open(&mut self, path: &Path) -> Result<HostReader, String> {
        assert!(!path.starts_with("unrelated"), "unrelated content read");
        self.calls.opens.lock().unwrap().push(path.into());
        self.fs.open(path)
    }
}

fn pool(a: &Path, b: &Path) -> (Pool, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let make = |root: &Path| {
        let root = root.to_path_buf();
        let calls = calls.clone();
        move || -> Result<Box<dyn CompareFs>, String> {
            calls.connections.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Tracked {
                fs: LocalFs::new(root.clone()),
                calls: calls.clone(),
            }))
        }
    };
    (Pool::connect(make(a), make(b), None).unwrap(), calls)
}

#[test]
fn a_single_file_recheck_reads_only_that_file_and_inspects_parent_metadata() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for root in [a.path(), b.path()] {
        std::fs::create_dir_all(root.join("selected/nested")).unwrap();
        std::fs::create_dir(root.join("unrelated")).unwrap();
        std::fs::write(root.join("selected/nested/sibling"), "do not inspect").unwrap();
        std::fs::write(root.join("unrelated/file"), "do not inspect").unwrap();
    }
    std::fs::write(a.path().join("selected/nested/file"), "same\r\n").unwrap();
    std::fs::write(b.path().join("selected/nested/file"), "same\n").unwrap();
    let (pool, calls) = pool(a.path(), b.path());
    let rows = pool
        .scan_paths(
            ComparisonMode::Full,
            &["selected/nested/file".into(), "selected/nested/file".into()],
            &[],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>(),
        ["selected", "selected/nested", "selected/nested/file"].map(PathBuf::from)
    );
    assert!(rows.iter().all(|row| row.status == Status::Equal));
    assert_eq!(
        *calls.opens.lock().unwrap(),
        vec![PathBuf::from("selected/nested/file"); 2]
    );
    assert_eq!(calls.connections.load(Ordering::Relaxed), 2);
    let rows = pool
        .scan_paths(
            ComparisonMode::Quick,
            &["selected/nested/file".into()],
            &[],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(rows.last().unwrap().status, Status::Different); // Raw sizes differ.
    assert_eq!(calls.opens.lock().unwrap().len(), 2); // Quick recheck doesn't read.
}

#[test]
fn a_folder_recheck_is_limited_to_selected_subtrees_and_honors_ignores() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for root in [a.path(), b.path()] {
        std::fs::create_dir_all(root.join("selected/nested")).unwrap();
        std::fs::create_dir(root.join("unrelated")).unwrap();
        std::fs::write(root.join("selected/nested/file"), "same").unwrap();
        std::fs::write(root.join("selected/ignored"), "ignored").unwrap();
        std::fs::write(root.join("unrelated/file"), "do not inspect").unwrap();
    }
    let (pool, calls) = pool(a.path(), b.path());
    let rows = pool
        .scan_paths(
            ComparisonMode::Full,
            &["selected".into(), "selected/nested".into()],
            &["selected/ignored".into()],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.status == Status::Equal));
    assert_eq!(calls.opens.lock().unwrap().len(), 2);
    let lists = calls.lists.lock().unwrap();
    assert_eq!(
        lists
            .iter()
            .filter(|path| path.as_path() == Path::new("selected/nested"))
            .count(),
        2
    );
}

#[test]
fn scoped_errors_cancellation_and_unsafe_paths_never_reuse_equal_results() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(a.path().join("file"), "a").unwrap();
    std::fs::write(b.path().join("file"), "b").unwrap();
    let (pool, calls) = pool(a.path(), b.path());
    assert!(
        pool.scan_paths(
            ComparisonMode::Full,
            &["../escape".into()],
            &[],
            &Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    assert!(calls.lists.lock().unwrap().is_empty());
    let rows = pool
        .scan_paths(
            ComparisonMode::Full,
            &["file".into()],
            &[],
            &Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
    assert!(rows.is_empty());
    assert!(calls.opens.lock().unwrap().is_empty());
    std::fs::remove_dir_all(b.path()).unwrap();
    let rows = pool
        .scan_paths(
            ComparisonMode::Full,
            &["file".into()],
            &[],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(rows[0].status, Status::Error);
    assert!(calls.opens.lock().unwrap().is_empty());
    // An ancestor replaced by a file must not be opened as a content candidate.
    std::fs::create_dir(b.path()).unwrap();
    for root in [a.path(), b.path()] {
        std::fs::write(root.join("parent"), "not a directory").unwrap();
    }
    let rows = pool
        .scan_paths(
            ComparisonMode::Full,
            &["parent/file".into()],
            &[],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Error));
    assert!(calls.opens.lock().unwrap().is_empty());
}
