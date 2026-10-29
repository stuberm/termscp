//! Read-only recursive comparison, independent of the terminal UI.
pub mod connection;
pub mod credentials;
pub mod filesystem;
pub mod profiles;
#[cfg(test)]
mod stream_tests;
pub mod text;
pub mod worker;

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use filesystem::{CompareFs, Entry, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Equal,
    Different,
    LeftOnly,
    RightOnly,
    TypeConflict,
    Unverified,
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "… pending",
            Self::Equal => "= equal",
            Self::Different => "≠ different",
            Self::LeftOnly => "< left only",
            Self::RightOnly => "> right only",
            Self::TypeConflict => "! type conflict",
            Self::Unverified => "? unverified",
            Self::Error => "! error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub path: PathBuf,
    pub left: Option<Entry>,
    pub right: Option<Entry>,
    pub status: Status,
    pub detail: String,
    /// Original state before descendant aggregation, for in-memory refiltering.
    pub aggregate_base: Option<(Status, String)>,
}

impl Row {
    pub fn is_directory(&self) -> bool {
        self.left
            .as_ref()
            .is_some_and(|entry| entry.kind == Kind::Directory)
            || self
                .right
                .as_ref()
                .is_some_and(|entry| entry.kind == Kind::Directory)
    }

    pub fn is_file_pair(&self) -> bool {
        self.left
            .as_ref()
            .is_some_and(|entry| entry.kind == Kind::File)
            && self
                .right
                .as_ref()
                .is_some_and(|entry| entry.kind == Kind::File)
    }
}

#[derive(Debug)]
pub enum ScanEvent {
    Progress(String),
    Rows(Vec<Row>),
}

#[derive(Default)]
struct Inventory {
    entries: BTreeMap<PathBuf, Entry>,
    errors: BTreeMap<PathBuf, String>,
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn inventory(
    fs: &mut dyn CompareFs,
    ignores: &[String],
    cancel: &AtomicBool,
    emit: &mut impl FnMut(ScanEvent),
    side: &str,
    discovered: &mut BTreeMap<PathBuf, Row>,
) -> Inventory {
    let mut result = Inventory::default();
    let mut directories = vec![PathBuf::new()];
    while let Some(directory) = directories.pop() {
        if cancelled(cancel) {
            break;
        }
        emit(ScanEvent::Progress(format!(
            "Listing {side}: {}",
            directory.display()
        )));
        let mut updates = Vec::new();
        match fs.list(&directory) {
            Ok(entries) => {
                for (name, entry) in entries {
                    // A server-controlled name must be exactly one normal path component.
                    let mut components = Path::new(&name).components();
                    if !matches!(components.next(), Some(Component::Normal(_)))
                        || components.next().is_some()
                        || name.to_string_lossy().contains(['/', '\\'])
                    {
                        result
                            .errors
                            .insert(directory.clone(), "Unsafe directory entry name".into());
                        continue;
                    }
                    let path = directory.join(name);
                    let normalized = path
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    if ignores
                        .iter()
                        .any(|pattern| wildmatch::WildMatch::new(pattern).matches(&normalized))
                    {
                        continue;
                    }
                    if entry.kind == Kind::Directory {
                        directories.push(path.clone());
                    }
                    let row = discovered.entry(path.clone()).or_insert_with(|| Row {
                        path: path.clone(),
                        left: None,
                        right: None,
                        status: Status::Pending,
                        detail: String::new(),
                        aggregate_base: None,
                    });
                    if side == "left" {
                        row.left = Some(entry.clone());
                    } else {
                        row.right = Some(entry.clone());
                    }
                    updates.push(row.clone());
                    if updates.len() == 64 {
                        emit(ScanEvent::Rows(std::mem::take(&mut updates)));
                    }
                    if result.entries.insert(path, entry).is_some() {
                        result
                            .errors
                            .insert(directory.clone(), "Duplicate directory entry name".into());
                    }
                }
            }
            Err(error) => {
                result.errors.insert(directory.clone(), error);
            }
        }
        if let Some(error) = result.errors.get(&directory) {
            let row = discovered.entry(directory.clone()).or_insert_with(|| Row {
                path: directory,
                left: None,
                right: None,
                status: Status::Pending,
                detail: String::new(),
                aggregate_base: None,
            });
            row.status = Status::Error;
            row.detail.clone_from(error);
            updates.push(row.clone());
        }
        if !updates.is_empty() {
            emit(ScanEvent::Rows(updates));
        }
    }
    result
}

fn ancestor_error<'a>(errors: &'a BTreeMap<PathBuf, String>, path: &Path) -> Option<&'a String> {
    path.ancestors().find_map(|ancestor| errors.get(ancestor))
}

/// Reads and finalizes both streams even on cancellation or I/O failure.
fn contents_equal(
    left: &mut dyn CompareFs,
    right: &mut dyn CompareFs,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Option<bool>, String> {
    let mut a = left.open(path)?;
    let mut b = match right.open(path) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = a.finish();
            return Err(error);
        }
    };
    let result = (|| {
        let mut a_buffer = [0; 64 * 1024];
        let mut b_buffer = [0; 64 * 1024];
        loop {
            if cancelled(cancel) {
                return Ok(None);
            }
            // Fill each chunk; Read::read may return different short-read lengths.
            let a_len = read_chunk(&mut a, &mut a_buffer).map_err(|error| error.to_string())?;
            let b_len = read_chunk(&mut b, &mut b_buffer).map_err(|error| error.to_string())?;
            if a_len != b_len || a_buffer[..a_len] != b_buffer[..b_len] {
                return Ok(Some(false));
            }
            if a_len == 0 {
                return Ok(Some(true));
            }
        }
    })();
    let a_finish = a.finish().map_err(|error| error.to_string());
    let b_finish = b.finish().map_err(|error| error.to_string());
    a_finish?;
    b_finish?;
    result
}

fn read_chunk(reader: &mut impl Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut len = 0;
    while len < buffer.len() {
        match reader.read(&mut buffer[len..]) {
            Ok(0) => break,
            Ok(count) => len += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(len)
}

/// Returns a complete snapshot, or a conservative partial snapshot if cancelled.
/// No metadata-only match is classified as equal for regular files.
pub fn scan(
    left: &mut dyn CompareFs,
    right: &mut dyn CompareFs,
    ignores: &[String],
    cancel: &AtomicBool,
    mut emit: impl FnMut(ScanEvent),
) -> Vec<Row> {
    let mut discovered = BTreeMap::new();
    let a = inventory(left, ignores, cancel, &mut emit, "left", &mut discovered).errors;
    let b = inventory(right, ignores, cancel, &mut emit, "right", &mut discovered).errors;
    let mut rows: Vec<Row> = discovered.into_values().collect();
    let mut updates = Vec::new();
    let mut last_update = Instant::now();
    for row in &mut rows {
        if let Some(error) = ancestor_error(&a, &row.path).or_else(|| ancestor_error(&b, &row.path))
        {
            row.status = Status::Error;
            row.detail = error.clone();
        } else if cancelled(cancel) {
            row.status = Status::Pending;
        } else {
            row.status = match (&row.left, &row.right) {
                (Some(_), None) => Status::LeftOnly,
                (None, Some(_)) => Status::RightOnly,
                (Some(a), Some(b)) if a.kind != b.kind => Status::TypeConflict,
                (Some(a), Some(_)) if a.kind == Kind::Directory => Status::Pending,
                (Some(a), Some(_)) if a.kind != Kind::File => {
                    row.detail = "Symlinks and special files are not followed or verified".into();
                    Status::Unverified
                }
                (Some(a), Some(b)) if a.size.zip(b.size).is_some_and(|(a, b)| a != b) => {
                    Status::Different
                }
                (Some(_), Some(_)) => {
                    emit(ScanEvent::Progress(format!(
                        "Verifying {}",
                        row.path.display()
                    )));
                    match contents_equal(left, right, &row.path, cancel) {
                        Ok(Some(true)) => Status::Equal,
                        Ok(Some(false)) => Status::Different,
                        Ok(None) => Status::Pending,
                        Err(error) => {
                            row.detail = error;
                            Status::Error
                        }
                    }
                }
                (None, None) => Status::Error,
            };
        }
        updates.push(row.clone());
        if updates.len() == 64 || last_update.elapsed() >= Duration::from_millis(100) {
            emit(ScanEvent::Rows(std::mem::take(&mut updates)));
            last_update = Instant::now();
        }
    }
    emit(ScanEvent::Rows(updates));
    if !cancelled(cancel) {
        for row in &mut rows {
            if row.status == Status::Pending
                && row
                    .left
                    .as_ref()
                    .is_some_and(|entry| entry.kind == Kind::Directory)
                && row
                    .right
                    .as_ref()
                    .is_some_and(|entry| entry.kind == Kind::Directory)
            {
                row.status = Status::Equal;
            }
        }
    }
    aggregate(&mut rows);
    rows
}

/// Match a row or any ancestor directory against root-relative ignore globs.
pub fn is_ignored(path: &Path, ignores: &[String]) -> bool {
    if ignores.is_empty() {
        return false;
    }
    path.ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .any(|path| {
            let normalized = path
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            ignores
                .iter()
                .any(|pattern| wildmatch::WildMatch::new(pattern).matches(&normalized))
        })
}

pub fn aggregate(rows: &mut [Row]) {
    // Recalculate from each row's own result, not its previous descendant summary.
    for row in rows.iter_mut() {
        if let Some((status, detail)) = row.aggregate_base.take() {
            row.status = status;
            row.detail = detail;
        }
    }
    // Children precede parents; propagate uncertainty instead of claiming equality.
    let indices: BTreeMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.path.clone(), index))
        .collect();
    for index in (0..rows.len()).rev() {
        let status = rows[index].status;
        if status == Status::Equal {
            continue;
        }
        if let Some(parent) = rows[index]
            .path
            .parent()
            .and_then(|path| indices.get(path))
            .copied()
            && rows[parent].is_directory()
            && matches!(
                rows[parent].status,
                Status::Equal | Status::Different | Status::Pending | Status::Unverified
            )
        {
            if rows[parent].aggregate_base.is_none() {
                rows[parent].aggregate_base =
                    Some((rows[parent].status, rows[parent].detail.clone()));
            }
            rows[parent].status = match (rows[parent].status, status) {
                (_, Status::Error) => Status::Error,
                (_, Status::Pending) | (Status::Pending, _) => Status::Pending,
                (_, Status::Unverified) | (Status::Unverified, _) => Status::Unverified,
                _ => Status::Different,
            };
            rows[parent].detail =
                "Contains changed, unverified, or inaccessible descendants".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filesystem::LocalFs;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn run(a: &Path, b: &Path) -> Vec<Row> {
        scan(
            &mut LocalFs::new(a.to_path_buf()),
            &mut LocalFs::new(b.to_path_buf()),
            &[],
            &AtomicBool::new(false),
            |_| {},
        )
    }

    #[test]
    fn compares_bytes_presence_types_and_empty_directories() {
        let (a, b) = fixture();
        for dir in [a.path(), b.path()] {
            fs::create_dir(dir.join("empty")).unwrap();
            fs::create_dir(dir.join("nested")).unwrap();
            fs::write(dir.join("same"), b"same").unwrap();
        }
        fs::write(a.path().join("nested/file"), b"abcd").unwrap();
        fs::write(b.path().join("nested/file"), b"abce").unwrap();
        fs::write(a.path().join("left"), b"").unwrap();
        fs::write(b.path().join("right"), b"").unwrap();
        fs::write(a.path().join("conflict"), b"").unwrap();
        fs::create_dir(b.path().join("conflict")).unwrap();
        let rows = run(a.path(), b.path());
        let status = |path: &str| {
            rows.iter()
                .find(|row| row.path == Path::new(path))
                .unwrap()
                .status
        };
        assert_eq!(status("same"), Status::Equal);
        assert_eq!(status("empty"), Status::Equal);
        assert_eq!(status("nested/file"), Status::Different);
        assert_eq!(status("nested"), Status::Different);
        assert_eq!(status("left"), Status::LeftOnly);
        assert_eq!(status("right"), Status::RightOnly);
        assert_eq!(status("conflict"), Status::TypeConflict);
    }

    #[test]
    fn missing_side_directory_is_traversable_and_ignored_directories_are_pruned() {
        let (a, b) = fixture();
        fs::create_dir_all(a.path().join("only/sub")).unwrap();
        fs::write(a.path().join("only/sub/file"), "x").unwrap();
        assert!(
            run(a.path(), b.path())
                .iter()
                .all(|row| row.status == Status::LeftOnly)
        );
        let rows = scan(
            &mut LocalFs::new(a.path().into()),
            &mut LocalFs::new(b.path().into()),
            &["only".into()],
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn listing_failure_never_becomes_equal_or_only_on_other_side() {
        let (a, b) = fixture();
        fs::write(a.path().join("file"), "x").unwrap();
        let rows = run(a.path(), &b.path().join("missing"));
        assert!(rows.iter().all(|row| row.status == Status::Error));
    }

    #[test]
    fn cancelled_scan_never_claims_equality() {
        let (a, b) = fixture();
        fs::write(a.path().join("file"), "x").unwrap();
        let cancel = AtomicBool::new(false);
        let rows = scan(
            &mut LocalFs::new(a.path().into()),
            &mut LocalFs::new(b.path().into()),
            &[],
            &cancel,
            |event| {
                if matches!(event, ScanEvent::Progress(_)) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(rows.iter().all(|row| row.status == Status::Pending));
    }

    #[test]
    fn listings_stream_merged_pending_rows_before_recursive_inventory_finishes() {
        let (a, b) = fixture();
        for root in [a.path(), b.path()] {
            fs::create_dir(root.join("nested")).unwrap();
            fs::write(root.join("nested/file"), b"same").unwrap();
        }
        let mut left_seen = false;
        let mut paired_seen = false;
        let mut saw_left_descent = false;
        let mut saw_right_descent = false;
        let rows = scan(
            &mut LocalFs::new(a.path().into()),
            &mut LocalFs::new(b.path().into()),
            &[],
            &AtomicBool::new(false),
            |event| match event {
                ScanEvent::Rows(rows) => {
                    for row in rows {
                        if row.path == Path::new("nested") {
                            if row.left.is_some() && row.right.is_none() {
                                assert_eq!(row.status, Status::Pending);
                                left_seen = true;
                            }
                            if row.left.is_some() && row.right.is_some() && !saw_right_descent {
                                assert_eq!(row.status, Status::Pending);
                                paired_seen = true;
                            }
                        }
                    }
                }
                ScanEvent::Progress(message) => {
                    if message == "Listing left: nested" {
                        assert!(left_seen, "left root must render before descending");
                        saw_left_descent = true;
                    }
                    if message == "Listing right: nested" {
                        assert!(paired_seen, "right root must merge before descending");
                        saw_right_descent = true;
                    }
                }
            },
        );
        assert!(saw_left_descent && saw_right_descent);
        assert!(rows.iter().all(|row| row.status == Status::Equal));
    }

    #[test]
    fn cancelling_after_first_listing_keeps_discovered_rows_uncertain() {
        let (a, b) = fixture();
        fs::write(a.path().join("file"), "same").unwrap();
        fs::write(b.path().join("file"), "same").unwrap();
        let cancel = AtomicBool::new(false);
        let rows = scan(
            &mut LocalFs::new(a.path().into()),
            &mut LocalFs::new(b.path().into()),
            &[],
            &cancel,
            |event| {
                if matches!(event, ScanEvent::Rows(_)) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Pending);
        assert!(rows[0].right.is_none());
    }

    #[test]
    fn handles_short_reads() {
        struct Short(std::io::Cursor<Vec<u8>>);
        impl Read for Short {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let length = buffer.len().min(2);
                self.0.read(&mut buffer[..length])
            }
        }
        let mut reader = Short(std::io::Cursor::new(b"abcdef".to_vec()));
        let mut buffer = [0; 10];
        assert_eq!(read_chunk(&mut reader, &mut buffer).unwrap(), 6);
        assert_eq!(&buffer[..6], b"abcdef");
    }

    #[cfg(posix)]
    #[test]
    fn symlinks_are_not_followed_including_broken_links() {
        let (a, b) = fixture();
        for root in [a.path(), b.path()] {
            std::os::unix::fs::symlink(root, root.join("cycle")).unwrap();
            std::os::unix::fs::symlink(root.join("missing"), root.join("broken")).unwrap();
        }
        let rows = run(a.path(), b.path());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.status == Status::Unverified));
    }
}
