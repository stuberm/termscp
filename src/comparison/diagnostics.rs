//! Opt-in metadata-only diagnostics, independent of protocol TRACE logging.
//! Never pass profiles, URLs, credentials, remote errors or file contents here.
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use super::filesystem::Entry;
use super::profiles::Endpoint;
use super::settings::QuickCheck;
use super::{ComparisonMode, Row, Status, quick};

pub const ENV: &str = "SIDEMIRROR_COMPARE_LOG";
static LOG: OnceLock<Mutex<SessionLog>> = OnceLock::new();
static IDS: AtomicU64 = AtomicU64::new(1);

struct SessionLog {
    writer: BufWriter<File>,
    sequence: u64,
    failed: bool,
}

impl SessionLog {
    fn open(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(posix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(Self {
            writer: BufWriter::new(options.open(path)?),
            sequence: 0,
            failed: false,
        })
    }

    fn write(&mut self, event: &str, data: Value) -> io::Result<()> {
        self.sequence += 1;
        let record = json!({
            "event": event, "sequence": self.sequence,
            "at_utc": Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
            "data": data,
        });
        serde_json::to_writer(&mut self.writer, &record).map_err(io::Error::other)?;
        self.writer.write_all(b"\n")?;
        // Diagnostic sessions favor complete, immediately readable records over
        // benchmark performance. No fsync and no remote/filesystem content reads.
        self.writer.flush()
    }
}

pub fn init_from_env() -> Result<Option<PathBuf>, String> {
    let Some(path) = std::env::var_os(ENV) else {
        return Ok(None);
    };
    if path.is_empty() {
        return Err(format!("{ENV} must name a new diagnostic file"));
    }
    if LOG.get().is_some() {
        return Err("Comparison diagnostics already initialized".into());
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let mut logger = SessionLog::open(&path).map_err(|error| format!(
        "Cannot create comparison diagnostic file {}: {error}. Choose a new file in an existing directory.", path.display()))?;
    logger.write("session_start", json!({
        "schema_version": 2, "application_version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(), "os": std::env::consts::OS,
        "quick_policy": "per_profile_enabled_metadata_checks; mtime_floored_to_utc_seconds_no_extra_tolerance",
        "privacy": "relative names and metadata only; no contents, hashes, URLs or credentials",
    })).map_err(|error| format!("Cannot initialize comparison diagnostics: {error}"))?;
    LOG.set(Mutex::new(logger))
        .map_err(|_| "Comparison diagnostics already initialized".to_string())?;
    Ok(Some(path))
}

pub fn enabled() -> bool {
    LOG.get().is_some()
}

pub fn new_id() -> u64 {
    if enabled() {
        IDS.fetch_add(1, Ordering::Relaxed)
    } else {
        0
    }
}

/// Lazy fields ensure disabled diagnostics perform no metadata serialization.
pub fn record(event: &str, fields: impl FnOnce() -> Value) {
    let Some(logger) = LOG.get() else {
        return;
    };
    let Ok(mut logger) = logger.lock() else {
        return;
    };
    if logger.failed {
        return;
    }
    if let Err(error) = logger.write(event, fields()) {
        logger.failed = true;
        // IO errors only; never include an underlying comparison/remote error.
        log::warn!("Comparison diagnostic logging stopped after a write failure: {error}");
    }
}

pub fn finish_session(success: bool) -> bool {
    record("session_end", || json!({ "success": success }));
    LOG.get()
        .is_none_or(|logger| logger.lock().is_ok_and(|logger| !logger.failed))
}

/// Intentionally excludes endpoint roots, host/user names, bookmark names and URLs.
pub fn backend(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Local { .. } => "local".into(),
        Endpoint::Bookmark { .. } => "bookmark".into(),
        Endpoint::Connection { settings } => format!("{:?}", settings.protocol()),
        Endpoint::Remote { url } => url::Url::parse(url)
            .ok()
            .map(|url| match url.scheme() {
                "sftp" | "scp" | "ftp" | "ftps" | "s3" | "gcs" | "http" | "https" => {
                    url.scheme().to_owned()
                }
                _ => "remote".into(),
            })
            .unwrap_or_else(|| "remote".into()),
    }
}

pub(super) fn mode_name(mode: ComparisonMode) -> &'static str {
    match mode {
        ComparisonMode::Quick => "quick",
        ComparisonMode::Full => "full",
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Pending => "pending",
        Status::Equal => "equal",
        Status::QuickMatch => "quick_match",
        Status::Different => "different",
        Status::LeftOnly => "left_only",
        Status::RightOnly => "right_only",
        Status::TypeConflict => "type_conflict",
        Status::Unverified => "unverified",
        Status::Error => "error",
    }
}

fn nanos(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    }
}

fn timestamp(time: SystemTime) -> Value {
    let ns = nanos(time);
    let utc = i64::try_from(quick::seconds(time))
        .ok()
        .and_then(|seconds| {
            DateTime::<Utc>::from_timestamp(seconds, ns.rem_euclid(1_000_000_000) as u32)
        })
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Nanos, true));
    // Decimal strings avoid loss of nanosecond precision in JSON consumers.
    json!({ "utc": utc, "unix_ns": ns.to_string(), "compared_seconds": quick::seconds(time).to_string() })
}

fn metadata(entry: &Option<Entry>) -> Value {
    match entry {
        None => Value::Null,
        Some(entry) => json!({
            "kind": format!("{:?}", entry.kind), "size": entry.size,
            "modified": entry.modified.map(timestamp),
        }),
    }
}

fn row_fields(row: &Row, mode: ComparisonMode, checks: QuickCheck) -> Value {
    let mode = mode.for_checks(checks);
    let sizes = row
        .left
        .as_ref()
        .and_then(|entry| entry.size)
        .zip(row.right.as_ref().and_then(|entry| entry.size));
    let times = row
        .left
        .as_ref()
        .and_then(|entry| entry.modified)
        .zip(row.right.as_ref().and_then(|entry| entry.modified));
    let size_equal = sizes.map(|(a, b)| a == b);
    let time_equal = times.map(|(a, b)| quick::seconds(a) == quick::seconds(b));
    let reason = if row.is_directory() && row.aggregate_base.is_some() {
        "descendant_summary"
    } else if mode == ComparisonMode::Quick
        && row.is_file_pair()
        && matches!(
            row.status,
            Status::Different | Status::QuickMatch | Status::Unverified
        )
    {
        let size_mismatch = checks.uses_size() && size_equal == Some(false);
        let time_mismatch = checks.uses_timestamp() && time_equal == Some(false);
        if size_mismatch && time_mismatch {
            "size_and_mtime_differ"
        } else if size_mismatch {
            "size_differs"
        } else if time_mismatch {
            "mtime_differs"
        } else if (!checks.uses_size() || size_equal.is_some())
            && (!checks.uses_timestamp() || time_equal.is_some())
        {
            "metadata_match_not_content_verified"
        } else {
            "required_metadata_missing"
        }
    } else {
        match row.status {
            Status::Equal | Status::QuickMatch if row.is_directory() => {
                "directory_descendants_match"
            }
            Status::Equal | Status::Different => "normalized_content_result",
            Status::QuickMatch => "metadata_match_not_content_verified",
            Status::LeftOnly | Status::RightOnly => "entry_missing_on_other_side",
            Status::TypeConflict => "entry_kinds_differ",
            Status::Pending => "not_finished_or_cancelled",
            Status::Unverified => "not_verified",
            Status::Error => "listing_or_verification_error_details_omitted",
        }
    };
    json!({
        "mode": mode_name(mode), "quick_check": checks,
        "size_check_enabled": checks.uses_size(), "timestamp_check_enabled": checks.uses_timestamp(),
        "path": row.path.to_string_lossy(),
        "path_native_hex": row.path.as_os_str().to_str().is_none().then(||
            row.path.as_os_str().as_encoded_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>()),
        "left": metadata(&row.left), "right": metadata(&row.right),
        "status": status_name(row.status), "reason": reason,
        "aggregate_base": row.aggregate_base.as_ref().map(|(status, _)| status_name(*status)),
        "size_equal": size_equal, "mtime_seconds_equal": time_equal,
        "size_delta_right_minus_left": sizes.map(|(a, b)| (i128::from(b) - i128::from(a)).to_string()),
        "mtime_delta_raw_ns_right_minus_left": times.map(|(a, b)| (nanos(b) - nanos(a)).to_string()),
        "mtime_delta_compared_seconds_right_minus_left": times.map(|(a, b)| (quick::seconds(b) - quick::seconds(a)).to_string()),
    })
}

pub struct ScanTrace {
    pool_id: u64,
    scan_id: u64,
    mode: ComparisonMode,
    quick_check: QuickCheck,
    started: Instant,
    complete: bool,
}

impl ScanTrace {
    pub fn begin(
        pool_id: u64,
        mode: ComparisonMode,
        roots: Option<&[PathBuf]>,
        ignores: &[String],
        quick_check: QuickCheck,
    ) -> Option<Self> {
        if !enabled() {
            return None;
        }
        let trace = Self {
            pool_id,
            scan_id: new_id(),
            mode,
            quick_check,
            started: Instant::now(),
            complete: false,
        };
        record("scan_start", || {
            json!({
                "pool_id": pool_id, "scan_id": trace.scan_id, "mode": mode_name(mode),
                "quick_check": quick_check,
                "scope": if roots.is_some() { "copied_paths" } else { "whole_comparison" },
                "roots": roots.map(|paths| paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>()),
                "ignores": ignores,
            })
        });
        Some(trace)
    }

    pub fn row(&self, phase: &str, row: &Row) {
        record("row", || {
            json!({
                "pool_id": self.pool_id, "scan_id": self.scan_id, "phase": phase,
                "result": row_fields(row, self.mode, self.quick_check),
            })
        });
    }

    pub fn finish(&mut self, rows: &[Row], cancelled: bool) {
        for row in rows {
            self.row("final", row);
        }
        record("scan_end", || {
            json!({
                "pool_id": self.pool_id, "scan_id": self.scan_id, "rows": rows.len(),
                "cancelled": cancelled, "elapsed_ms": self.started.elapsed().as_millis(),
            })
        });
        self.complete = true;
    }
}

impl Drop for ScanTrace {
    fn drop(&mut self) {
        if !self.complete {
            record("scan_aborted", || {
                json!({
                    "pool_id": self.pool_id, "scan_id": self.scan_id,
                    "elapsed_ms": self.started.elapsed().as_millis(),
                    "reason": "scan_returned_early_or_failed_details_omitted",
                })
            });
        }
    }
}

pub fn display_rows<'a>(
    request_id: u64,
    mode: ComparisonMode,
    rows: impl IntoIterator<Item = &'a Row>,
    checks: QuickCheck,
) {
    if !enabled() {
        return;
    }
    for row in rows {
        record("display_row", || {
            let mut result = row_fields(row, mode, checks);
            result["reason"] = json!("displayed_snapshot_not_a_new_comparison_decision");
            json!({ "request_id": request_id, "result": result })
        });
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
