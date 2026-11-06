use std::time::Duration;

use super::*;
use crate::comparison::filesystem::Kind;

// Most fixtures exercise the default (both checks); policy-specific cases below
// call the underlying builder explicitly.
fn row_fields(row: &Row, mode: ComparisonMode) -> Value {
    super::row_fields(row, mode, QuickCheck::default())
}

fn row(size: Option<u64>, a: Option<SystemTime>, b: Option<SystemTime>) -> Row {
    Row {
        path: "folder/file.txt".into(),
        left: Some(Entry {
            kind: Kind::File,
            size,
            modified: a,
        }),
        right: Some(Entry {
            kind: Kind::File,
            size,
            modified: b,
        }),
        status: Status::Pending,
        detail: String::new(),
        aggregate_base: None,
    }
}

#[test]
fn timestamps_keep_exact_nanoseconds_and_the_actual_comparison_precision() {
    for (time, ns, seconds, utc) in [
        (
            UNIX_EPOCH + Duration::from_nanos(999_999_999),
            "999999999",
            "0",
            "1970-01-01T00:00:00.999999999Z",
        ),
        (
            UNIX_EPOCH - Duration::from_nanos(1),
            "-1",
            "-1",
            "1969-12-31T23:59:59.999999999Z",
        ),
        (
            UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789),
            "1700000000123456789",
            "1700000000",
            "2023-11-14T22:13:20.123456789Z",
        ),
    ] {
        let fields = timestamp(time);
        assert_eq!(fields["unix_ns"], ns);
        assert_eq!(fields["compared_seconds"], seconds);
        assert_eq!(fields["utc"], utc);
    }
}

#[test]
fn mismatch_logs_explain_size_mtime_missing_metadata_and_second_boundaries() {
    let a = UNIX_EPOCH + Duration::from_millis(100_900);
    let b = UNIX_EPOCH + Duration::from_secs(101);
    let mut row = row(Some(4), Some(a), Some(b));
    quick::compare(&mut row);
    let fields = row_fields(&row, ComparisonMode::Quick);
    assert_eq!(row.status, Status::Different);
    assert_eq!(fields["reason"], "mtime_differs");
    assert_eq!(fields["mtime_delta_raw_ns_right_minus_left"], "100000000");
    assert_eq!(fields["mtime_delta_compared_seconds_right_minus_left"], "1");
    assert_eq!(fields["mtime_seconds_equal"], false);

    row.right.as_mut().unwrap().modified = Some(a);
    row.right.as_mut().unwrap().size = Some(5);
    quick::compare(&mut row);
    assert_eq!(
        row_fields(&row, ComparisonMode::Quick)["reason"],
        "size_differs"
    );
    row.right.as_mut().unwrap().modified = Some(b);
    quick::compare(&mut row);
    assert_eq!(
        row_fields(&row, ComparisonMode::Quick)["reason"],
        "size_and_mtime_differ"
    );
    row.right.as_mut().unwrap().size = Some(4);
    row.right.as_mut().unwrap().modified = None;
    quick::compare(&mut row);
    let fields = row_fields(&row, ComparisonMode::Quick);
    assert_eq!(fields["reason"], "required_metadata_missing");
    assert!(fields["right"]["modified"].is_null());
    assert_eq!(fields["status"], "unverified");
}

#[test]
fn criteria_diagnostics_ignore_disabled_fields_but_retain_raw_metadata() {
    let mut row = row(
        Some(4),
        Some(UNIX_EPOCH),
        Some(UNIX_EPOCH + Duration::from_secs(1)),
    );
    quick::compare_with(&mut row, QuickCheck::Size);
    let fields = super::row_fields(&row, ComparisonMode::Quick, QuickCheck::Size);
    assert_eq!(fields["reason"], "metadata_match_not_content_verified");
    assert_eq!(fields["status"], "quick_match");
    assert_eq!(fields["quick_check"], "size");
    assert_eq!(fields["timestamp_check_enabled"], false);
    assert_eq!(fields["mtime_seconds_equal"], false); // Recorded but explicitly not tested.
    row.right.as_mut().unwrap().size = Some(5);
    row.right.as_mut().unwrap().modified = Some(UNIX_EPOCH);
    quick::compare_with(&mut row, QuickCheck::Timestamp);
    let fields = super::row_fields(&row, ComparisonMode::Quick, QuickCheck::Timestamp);
    assert_eq!(fields["reason"], "metadata_match_not_content_verified");
    assert_eq!(fields["size_check_enabled"], false);
    assert_eq!(fields["size_equal"], false);
    row.right.as_mut().unwrap().modified = None;
    quick::compare_with(&mut row, QuickCheck::Timestamp);
    assert_eq!(
        super::row_fields(&row, ComparisonMode::Quick, QuickCheck::Timestamp)["reason"],
        "required_metadata_missing"
    );
}

#[test]
fn directory_logs_expose_aggregation_bases_without_comparing_directory_mtimes() {
    let mut parent = row(
        None,
        Some(UNIX_EPOCH),
        Some(UNIX_EPOCH + Duration::from_secs(3600)),
    );
    parent.path = "folder".into();
    parent.left.as_mut().unwrap().kind = Kind::Directory;
    parent.right.as_mut().unwrap().kind = Kind::Directory;
    parent.status = Status::QuickMatch;
    let mut child = row(
        Some(4),
        Some(UNIX_EPOCH),
        Some(UNIX_EPOCH + Duration::from_secs(1)),
    );
    quick::compare(&mut child);
    let mut rows = vec![parent, child];
    crate::comparison::aggregate(&mut rows);
    let parent = row_fields(&rows[0], ComparisonMode::Quick);
    assert_eq!(parent["reason"], "descendant_summary");
    assert_eq!(parent["aggregate_base"], "quick_match");
    assert_eq!(parent["status"], "different");
    assert_eq!(
        row_fields(&rows[1], ComparisonMode::Quick)["reason"],
        "mtime_differs"
    );
}

#[test]
fn endpoint_labels_omit_urls_roots_bookmark_names_and_credentials() {
    let labels = [
        backend(&Endpoint::Remote {
            url: "sftp://private-user:SECRET@private-host/private-root?token=TOKEN".into(),
        }),
        backend(&Endpoint::Bookmark {
            name: "private-bookmark-SECRET".into(),
        }),
        backend(&Endpoint::Local {
            path: "/private-root/SECRET".into(),
        }),
        backend(&Endpoint::Connection {
            settings: Box::new(
                crate::comparison::connection::ConnectionSettings::from_url(
                    "sftp://private-user@private-host/private-root",
                )
                .unwrap(),
            ),
        }),
    ];
    for label in labels {
        for private in ["private", "SECRET", "TOKEN", "sftp://", "?token"] {
            assert!(!label.contains(private), "{label}");
        }
    }
}

#[test]
fn jsonl_is_flushed_escaped_private_and_never_overwrites_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let mut log = SessionLog::open(&path).unwrap();
    let mut row = row(Some(4), Some(UNIX_EPOCH), Some(UNIX_EPOCH));
    row.path = "folder/line\nbreak\u{1b}.txt".into();
    row.detail = "sftp://user:SECRET@server/?token=TOKEN".into();
    row.aggregate_base = Some((Status::Error, "SECRET TOKEN".into()));
    quick::compare(&mut row);
    row.detail = "sftp://user:SECRET@server/?token=TOKEN".into();
    log.write("row", row_fields(&row, ComparisonMode::Quick))
        .unwrap();
    log.write("session_end", json!({"success": true})).unwrap();
    let text = std::fs::read_to_string(&path).unwrap(); // Still open: flush must have happened.
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains("line\\nbreak\\u001b.txt"));
    for private in ["SECRET", "TOKEN", "sftp://"] {
        assert!(!text.contains(private));
    }
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[0]["sequence"], 1);
    assert_eq!(records[1]["sequence"], 2);
    assert_eq!(records[0]["data"]["path"], "folder/line\nbreak\u{1b}.txt");
    assert!(SessionLog::open(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    #[cfg(posix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    assert!(SessionLog::open(&directory.path().join("missing-parent/log.jsonl")).is_err());
}

#[cfg(posix)]
#[test]
fn existing_symlinks_are_not_followed_and_non_utf8_paths_keep_lossless_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("private.txt");
    let link = directory.path().join("log.jsonl");
    std::fs::write(&target, "do not truncate").unwrap();
    symlink(&target, &link).unwrap();
    assert!(SessionLog::open(&link).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not truncate");
    let mut row = row(Some(1), None, None);
    row.path = OsString::from_vec(b"name-\xff".to_vec()).into();
    assert_eq!(
        row_fields(&row, ComparisonMode::Quick)["path_native_hex"],
        "6e616d652dff"
    );
}

#[test]
fn enabled_session_captures_real_quick_and_full_scans_in_an_isolated_process() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "comparison::diagnostics::tests::diagnostic_child_process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env(ENV, &path)
        .env("SIDEMIRROR_DIAGNOSTIC_FIXTURE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("DO_NOT_LOG_FILE_CONTENTS"));
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.first().unwrap()["event"], "session_start");
    assert_eq!(records.last().unwrap()["event"], "session_end");
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record["sequence"], index + 1);
    }
    let decisions: Vec<_> = records
        .iter()
        .filter(|record| {
            record["event"] == "row"
                && record["data"]["phase"] == "final"
                && record["data"]["result"]["path"] == "file"
        })
        .map(|record| &record["data"]["result"])
        .collect();
    assert_eq!(records.first().unwrap()["data"]["schema_version"], 2);
    assert_eq!(decisions.len(), 6);
    assert_eq!(decisions[0]["mode"], "quick");
    assert_eq!(decisions[0]["status"], "different");
    assert_eq!(decisions[0]["reason"], "mtime_differs");
    assert_eq!(
        decisions[0]["mtime_delta_compared_seconds_right_minus_left"],
        "1"
    );
    assert_eq!(decisions[1]["mode"], "full");
    assert_eq!(decisions[1]["status"], "equal");
    assert_eq!(decisions[2]["quick_check"], "size");
    assert_eq!(decisions[2]["status"], "quick_match");
    assert_eq!(
        decisions[2]["reason"],
        "metadata_match_not_content_verified"
    );
    assert_eq!(decisions[3]["quick_check"], "timestamp");
    assert_eq!(decisions[3]["status"], "different");
    assert_eq!(decisions[3]["reason"], "mtime_differs");
    assert_eq!(decisions[4]["quick_check"], "none");
    assert_eq!(decisions[4]["mode"], "full");
    assert_eq!(decisions[4]["status"], "equal");
    assert_eq!(decisions[4]["reason"], "normalized_content_result");
    assert_eq!(decisions[4]["size_check_enabled"], false);
    assert_eq!(decisions[4]["timestamp_check_enabled"], false);
    assert_eq!(decisions[5]["quick_check"], "both");
    assert_eq!(decisions[5]["mode"], "quick");
    assert_eq!(decisions[5]["status"], "different");
    assert!(
        records
            .iter()
            .any(|record| record["event"] == "profile_open"
                && record["data"]["source_backend"] == "local")
    );
    assert!(
        records
            .iter()
            .any(|record| record["event"] == "display_row")
    );
}

#[test]
#[ignore = "fixture run only by the isolated-process diagnostics regression"]
fn diagnostic_child_process_fixture() {
    if std::env::var_os("SIDEMIRROR_DIAGNOSTIC_FIXTURE").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    use crate::comparison::connection::Credentials;
    use crate::comparison::profiles::Profile;
    use crate::comparison::settings::Parallelism;
    use crate::comparison::worker::{Event, Worker};
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    init_from_env().unwrap().unwrap();
    for (root, seconds) in [(a.path(), 100), (b.path(), 101)] {
        std::fs::write(root.join("file"), "DO_NOT_LOG_FILE_CONTENTS\n").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("file"))
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }
    let mut worker = Worker::start(
        Profile::new(
            "fixture".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
        Credentials::default(),
        false,
        1,
        Parallelism::default(),
    );
    for (id, mode, checks, expected) in [
        (
            1,
            ComparisonMode::Quick,
            QuickCheck::Both,
            Status::Different,
        ),
        (2, ComparisonMode::Full, QuickCheck::Both, Status::Equal),
        (
            3,
            ComparisonMode::Quick,
            QuickCheck::Size,
            Status::QuickMatch,
        ),
        (
            4,
            ComparisonMode::Quick,
            QuickCheck::Timestamp,
            Status::Different,
        ),
        (5, ComparisonMode::Quick, QuickCheck::None, Status::Equal),
        (
            6,
            ComparisonMode::Quick,
            QuickCheck::Both,
            Status::Different,
        ),
    ] {
        if id != 1 {
            worker
                .refresh(id, vec![], Parallelism::default(), mode, checks)
                .unwrap();
        }
        loop {
            if let Event::Finished {
                id: observed, rows, ..
            } = worker.events.recv_timeout(Duration::from_secs(10)).unwrap()
            {
                assert_eq!(observed, id);
                assert_eq!(rows[0].status, expected);
                display_rows(id, mode, &rows, checks);
                break;
            }
        }
    }
    drop(worker);
    assert!(finish_session(true));
}

#[test]
fn disabled_diagnostics_never_evaluate_fields_or_create_log_files() {
    assert!(!enabled()); // Tests intentionally never initialize the process-global sink.
    record("disabled", || {
        panic!("disabled diagnostics evaluated their fields")
    });
    assert_eq!(new_id(), 0);
    assert!(ScanTrace::begin(0, ComparisonMode::Quick, None, &[], QuickCheck::default()).is_none());
    assert!(finish_session(true));
}
