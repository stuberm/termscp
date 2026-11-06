use std::ffi::OsString;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::comparison::filesystem::{CompareFs, Entry, Fingerprint, Kind};
use crate::comparison::parallel::Pool;
use crate::comparison::{ComparisonMode, ScanEvent, finish_scan};
use crate::host::HostReader;

fn entry(size: Option<u64>, modified: Option<SystemTime>) -> Entry {
    Entry {
        kind: Kind::File,
        size,
        modified,
    }
}

fn row(left: Entry, right: Entry) -> Row {
    Row {
        path: "file".into(),
        left: Some(left),
        right: Some(right),
        status: Status::Pending,
        detail: String::new(),
        aggregate_base: None,
    }
}

#[test]
fn metadata_matches_differences_and_unknowns_are_not_content_equality() {
    let time = Some(UNIX_EPOCH + Duration::from_secs(100));
    let later = Some(UNIX_EPOCH + Duration::from_secs(101));
    for (left, right, expected) in [
        (
            entry(Some(4), time),
            entry(Some(4), time),
            Status::QuickMatch,
        ),
        (
            entry(Some(4), time),
            entry(Some(5), time),
            Status::Different,
        ),
        (
            entry(Some(4), time),
            entry(Some(4), later),
            Status::Different,
        ),
        (
            entry(Some(4), None),
            entry(Some(4), time),
            Status::Unverified,
        ),
        (entry(None, time), entry(Some(4), time), Status::Unverified),
        (entry(None, None), entry(None, None), Status::Unverified),
        (
            entry(Some(4), None),
            entry(Some(5), None),
            Status::Different,
        ),
        (entry(None, time), entry(None, later), Status::Different),
    ] {
        let mut row = row(left, right);
        compare(&mut row);
        assert_eq!(row.status, expected, "{}", row.detail);
        assert_ne!(row.status, Status::Equal);
        assert!(row.detail.contains("F5"));
    }
}

#[test]
fn selected_checks_ignore_disabled_differences_and_missing_metadata() {
    use crate::comparison::settings::QuickCheck;
    let time = Some(UNIX_EPOCH + Duration::from_secs(100));
    let later = Some(UNIX_EPOCH + Duration::from_secs(101));
    for (checks, left, right, expected) in [
        (
            QuickCheck::Size,
            entry(Some(4), time),
            entry(Some(4), later),
            Status::QuickMatch,
        ),
        (
            QuickCheck::Size,
            entry(Some(4), None),
            entry(Some(4), None),
            Status::QuickMatch,
        ),
        (
            QuickCheck::Size,
            entry(Some(4), time),
            entry(Some(5), time),
            Status::Different,
        ),
        (
            QuickCheck::Size,
            entry(None, time),
            entry(None, time),
            Status::Unverified,
        ),
        (
            QuickCheck::Size,
            entry(None, time),
            entry(Some(4), later),
            Status::Unverified,
        ),
        (
            QuickCheck::Timestamp,
            entry(Some(4), time),
            entry(Some(5), time),
            Status::QuickMatch,
        ),
        (
            QuickCheck::Timestamp,
            entry(None, time),
            entry(None, time),
            Status::QuickMatch,
        ),
        (
            QuickCheck::Timestamp,
            entry(None, time),
            entry(None, later),
            Status::Different,
        ),
        (
            QuickCheck::Timestamp,
            entry(Some(4), None),
            entry(Some(4), None),
            Status::Unverified,
        ),
        (
            QuickCheck::Timestamp,
            entry(Some(4), None),
            entry(Some(5), time),
            Status::Unverified,
        ),
        (
            QuickCheck::Both,
            entry(Some(4), time),
            entry(Some(5), time),
            Status::Different,
        ),
        (
            QuickCheck::Both,
            entry(Some(4), time),
            entry(Some(4), later),
            Status::Different,
        ),
        (
            QuickCheck::Both,
            entry(Some(4), None),
            entry(Some(4), time),
            Status::Unverified,
        ),
        (
            QuickCheck::Both,
            entry(None, time),
            entry(Some(4), time),
            Status::Unverified,
        ),
    ] {
        let mut row = row(left, right);
        super::compare_with(&mut row, checks);
        assert_eq!(row.status, expected, "{checks:?}: {}", row.detail);
        assert!(row.detail.contains(checks.label()));
        assert_ne!(row.status, Status::Equal);
    }
}

#[test]
fn disabled_quick_checks_never_produce_a_vacuous_metadata_match() {
    use crate::comparison::settings::QuickCheck;
    let mut row = row(
        entry(Some(4), Some(UNIX_EPOCH)),
        entry(Some(4), Some(UNIX_EPOCH)),
    );
    super::compare_with(&mut row, QuickCheck::None);
    assert_eq!(row.status, Status::Unverified);
    assert!(row.detail.contains("full content verification required"));
    assert_eq!(
        ComparisonMode::Quick.for_checks(QuickCheck::None),
        ComparisonMode::Full
    );
    assert_eq!(
        ComparisonMode::Full.for_checks(QuickCheck::None),
        ComparisonMode::Full
    );
}

#[test]
fn configured_pool_rechecks_only_enabled_metadata_and_full_scans_ignore_criteria() {
    use crate::comparison::settings::QuickCheck;
    let (mut pool, counters) = pool(
        vec![("file".into(), entry(Some(4), Some(UNIX_EPOCH)))],
        vec![(
            "file".into(),
            entry(Some(4), Some(UNIX_EPOCH + Duration::from_secs(1))),
        )],
        false,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    for (checks, expected) in [
        (QuickCheck::Both, Status::Different),
        (QuickCheck::Size, Status::QuickMatch),
        (QuickCheck::Timestamp, Status::Different),
    ] {
        pool.set_quick_check(checks);
        let rows = pool
            .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |_| {})
            .unwrap();
        assert!(rows.iter().all(|row| row.status == expected)); // Includes directory summary.
        let rows = pool
            .scan_paths(ComparisonMode::Quick, &["folder/file".into()], &[], &cancel)
            .unwrap();
        assert!(rows.iter().all(|row| row.status == expected));
        assert_eq!(counters.opens.load(Ordering::Relaxed), 0);
        assert_eq!(counters.fingerprints.load(Ordering::Relaxed), 0);
        assert_eq!(counters.connections.load(Ordering::Relaxed), 2);
    }
    pool.set_quick_check(QuickCheck::Size);
    let rows = pool
        .scan_with_mode(ComparisonMode::Full, &[], &cancel, |_| {})
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Different)); // same-size changed bytes
    assert_eq!(counters.opens.load(Ordering::Relaxed), 2);
    pool.set_quick_check(QuickCheck::None);
    for scoped in [false, true] {
        let rows = if scoped {
            pool.scan_paths(ComparisonMode::Quick, &["folder/file".into()], &[], &cancel)
        } else {
            pool.scan_with_mode(ComparisonMode::Quick, &[], &cancel, |_| {})
        }
        .unwrap();
        assert!(rows.iter().all(|row| row.status == Status::Different));
        assert_eq!(
            counters.opens.load(Ordering::Relaxed),
            if scoped { 6 } else { 4 }
        );
    }
}

#[test]
fn full_comparison_keeps_crlf_equivalence_with_any_quick_criteria() {
    use crate::comparison::filesystem::LocalFs;
    use crate::comparison::settings::QuickCheck;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for (root, bytes) in [(a.path(), b"same\r\n".as_slice()), (b.path(), b"same\n")] {
        std::fs::write(root.join("file"), bytes).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("file"))
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(100))
            .unwrap();
    }
    let a = a.path().to_path_buf();
    let b = b.path().to_path_buf();
    let mut pool = Pool::connect(
        move || Ok(Box::new(LocalFs::new(a.clone()))),
        move || Ok(Box::new(LocalFs::new(b.clone()))),
        None,
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    for check in [
        QuickCheck::None,
        QuickCheck::Both,
        QuickCheck::Size,
        QuickCheck::Timestamp,
    ] {
        pool.set_quick_check(check);
        let rows = pool
            .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |_| {})
            .unwrap();
        assert_eq!(
            rows[0].status,
            if check == QuickCheck::None {
                Status::Equal
            } else if check == QuickCheck::Timestamp {
                Status::QuickMatch
            } else {
                Status::Different
            }
        );
        let rows = pool
            .scan_with_mode(ComparisonMode::Full, &[], &cancel, |_| {})
            .unwrap();
        assert_eq!(rows[0].status, Status::Equal);
    }
}

#[test]
fn timestamps_use_whole_utc_seconds_without_extra_tolerance() {
    for (a, b, expected) in [
        (
            UNIX_EPOCH,
            UNIX_EPOCH + Duration::from_millis(999),
            Status::QuickMatch,
        ),
        (
            UNIX_EPOCH,
            UNIX_EPOCH + Duration::from_secs(1),
            Status::Different,
        ),
        (
            UNIX_EPOCH - Duration::from_millis(1),
            UNIX_EPOCH - Duration::from_secs(1),
            Status::QuickMatch,
        ),
        (
            UNIX_EPOCH - Duration::from_millis(1),
            UNIX_EPOCH,
            Status::Different,
        ),
    ] {
        let mut row = row(entry(Some(4), Some(a)), entry(Some(4), Some(b)));
        compare(&mut row);
        assert_eq!(row.status, expected);
    }
}

#[derive(Default)]
struct Counters {
    connections: AtomicUsize,
    opens: AtomicUsize,
    fingerprints: AtomicUsize,
}

struct Fixture {
    entries: Vec<(OsString, Entry)>,
    data: &'static [u8],
    counters: Arc<Counters>,
    fail: bool,
}

impl CompareFs for Fixture {
    fn list(&mut self, relative: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        if self.fail {
            return Err("Listing denied".into());
        }
        if relative.as_os_str().is_empty() {
            Ok(vec![(
                "folder".into(),
                Entry {
                    kind: Kind::Directory,
                    size: None,
                    modified: None,
                },
            )])
        } else {
            Ok(self.entries.clone())
        }
    }

    fn fingerprint(&mut self, _: &Path) -> Result<Option<Fingerprint>, String> {
        self.counters.fingerprints.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    fn open(&mut self, _: &Path) -> Result<HostReader, String> {
        self.counters.opens.fetch_add(1, Ordering::Relaxed);
        Ok(HostReader::io(Cursor::new(self.data)))
    }
}

fn pool(a: Vec<(OsString, Entry)>, b: Vec<(OsString, Entry)>, fail: bool) -> (Pool, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let left = counters.clone();
    let right = counters.clone();
    let pool = Pool::connect(
        move || {
            left.connections.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Fixture {
                entries: a.clone(),
                data: b"abcd",
                counters: left.clone(),
                fail: false,
            }))
        },
        move || {
            right.connections.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Fixture {
                entries: b.clone(),
                data: b"abce",
                counters: right.clone(),
                fail,
            }))
        },
        None,
    )
    .unwrap();
    (pool, counters)
}

fn files(count: usize) -> Vec<(OsString, Entry)> {
    (0..count)
        .map(|index| {
            (
                format!("file{index:03}").into(),
                entry(Some(4), Some(UNIX_EPOCH)),
            )
        })
        .collect()
}

#[test]
fn quick_scans_only_list_and_never_expand_content_lanes_or_probe_fingerprints() {
    let (pool, counters) = pool(files(100), files(100), false);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut streamed = Vec::new();
    let rows = pool
        .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |event| {
            if let ScanEvent::Rows(rows) = event {
                streamed.extend(rows);
            }
        })
        .unwrap();
    assert_eq!(rows.len(), 101);
    assert!(rows.iter().all(|row| row.status == Status::QuickMatch));
    assert!(streamed.iter().any(|row| row.status == Status::QuickMatch));
    assert_eq!(counters.connections.load(Ordering::Relaxed), 2);
    assert_eq!(counters.opens.load(Ordering::Relaxed), 0);
    assert_eq!(counters.fingerprints.load(Ordering::Relaxed), 0);
    // The full scan must ignore prior metadata matches, including on reused connections.
    let rows = pool
        .scan_with_mode(ComparisonMode::Full, &[], &cancel, |_| {})
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Different));
    assert_eq!(counters.opens.load(Ordering::Relaxed), 200);
    assert!(counters.fingerprints.load(Ordering::Relaxed) > 0);
    // A later quick refresh does not accidentally retain the full scan's mode/results.
    let rows = pool
        .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |_| {})
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::QuickMatch));
    assert_eq!(counters.opens.load(Ordering::Relaxed), 200);
}

#[test]
fn quick_scan_aggregates_uncertainty_and_presence_without_reading_contents() {
    let mut a = files(1);
    let mut b = files(1);
    a.push(("missing-time".into(), entry(Some(4), None)));
    b.push(("missing-time".into(), entry(Some(4), None)));
    a.push(("left-only".into(), entry(Some(4), None)));
    b.push(("right-only".into(), entry(Some(4), None)));
    a.push((
        "link".into(),
        Entry {
            kind: Kind::Symlink,
            size: None,
            modified: None,
        },
    ));
    b.push((
        "link".into(),
        Entry {
            kind: Kind::Symlink,
            size: None,
            modified: None,
        },
    ));
    let (pool, counters) = pool(a, b, false);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut rows = pool
        .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |_| {})
        .unwrap();
    let status = |path: &str| {
        rows.iter()
            .find(|row| row.path == Path::new(path))
            .unwrap()
            .status
    };
    assert_eq!(status("folder"), Status::Unverified);
    assert_eq!(status("folder/missing-time"), Status::Unverified);
    assert_eq!(status("folder/left-only"), Status::LeftOnly);
    assert_eq!(status("folder/right-only"), Status::RightOnly);
    assert_eq!(status("folder/link"), Status::Unverified);
    // Ignore/refilter aggregation must restore the original quick-match directory base.
    rows.retain(|row| {
        !matches!(
            row.path.file_name().unwrap().to_str(),
            Some("missing-time" | "link" | "left-only" | "right-only")
        )
    });
    finish_scan(&mut rows, &cancel);
    assert!(rows.iter().all(|row| row.status == Status::QuickMatch));
    let rows = pool
        .scan_with_mode(ComparisonMode::Quick, &["folder".into()], &cancel, |_| {})
        .unwrap();
    assert!(rows.is_empty());
    assert_eq!(counters.opens.load(Ordering::Relaxed), 0);
}

#[test]
fn quick_scans_preserve_listing_errors_and_cancelled_inventory() {
    let (pool, _) = pool(files(1), files(1), true);
    let rows = pool
        .scan_with_mode(
            ComparisonMode::Quick,
            &[],
            &Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
    assert!(rows.iter().all(|row| row.status == Status::Error));
    let cancel = Arc::new(AtomicBool::new(false));
    let rows = pool
        .scan_with_mode(ComparisonMode::Quick, &[], &cancel, |event| {
            if matches!(event, ScanEvent::Rows(_)) {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
    assert!(rows.iter().all(|row| !row.status.is_match()));
}
