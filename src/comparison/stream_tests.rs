//! Protocol-independent coverage for incomplete listings and stream lifecycle.
use std::ffi::OsString;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use remotefs::fs::{ReadStream, RemoteRead};

use super::*;
use crate::host::HostReader;

struct Reader {
    data: Cursor<Vec<u8>>,
    finishes: Arc<AtomicUsize>,
    fail_read: bool,
    fail_finish: bool,
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        let count = buffer.len().min(3);
        self.data.read(&mut buffer[..count])
    }
}

impl RemoteRead for Reader {
    fn finish(self: Box<Self>) -> remotefs::RemoteResult<()> {
        self.finishes.fetch_add(1, Ordering::Relaxed);
        if self.fail_finish {
            Err(remotefs::RemoteError::new(
                remotefs::RemoteErrorType::ProtocolError,
            ))
        } else {
            Ok(())
        }
    }
}

struct FakeFs {
    data: Vec<u8>,
    finishes: Arc<AtomicUsize>,
    fail_read: bool,
    fail_finish: bool,
    fail_open: bool,
}

impl FakeFs {
    fn new(data: &[u8]) -> Self {
        Self {
            data: data.into(),
            finishes: Arc::new(AtomicUsize::new(0)),
            fail_read: false,
            fail_finish: false,
            fail_open: false,
        }
    }
}

impl CompareFs for FakeFs {
    fn list(&mut self, _: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        Ok(vec![(
            "file".into(),
            Entry {
                kind: Kind::File,
                size: None,
            },
        )])
    }

    fn open(&mut self, _: &Path) -> Result<HostReader, String> {
        if self.fail_open {
            return Err("open failed".into());
        }
        Ok(HostReader::remote(ReadStream::new(Reader {
            data: Cursor::new(self.data.clone()),
            finishes: self.finishes.clone(),
            fail_read: self.fail_read,
            fail_finish: self.fail_finish,
        })))
    }
}

#[test]
fn finalizes_streams_on_equality_difference_read_errors_and_finish_errors() {
    for (bytes, fail_read, fail_finish, expected) in [
        (b"abc".as_slice(), false, false, Status::Equal),
        (b"xyz".as_slice(), false, false, Status::Different),
        (b"abc".as_slice(), true, false, Status::Error),
        (b"abc".as_slice(), false, true, Status::Error),
    ] {
        let mut a = FakeFs::new(b"abc");
        let mut b = FakeFs::new(bytes);
        a.fail_read = fail_read;
        a.fail_finish = fail_finish;
        let rows = scan(&mut a, &mut b, &[], &AtomicBool::new(false), |_| {});
        assert_eq!(rows[0].status, expected);
        assert_eq!(a.finishes.load(Ordering::Relaxed), 1);
        assert_eq!(b.finishes.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn finalizes_first_stream_when_second_open_fails_and_both_when_cancelled() {
    let mut a = FakeFs::new(b"abc");
    let mut b = FakeFs::new(b"abc");
    b.fail_open = true;
    assert!(contents_equal(&mut a, &mut b, Path::new("file"), &AtomicBool::new(false)).is_err());
    assert_eq!(a.finishes.load(Ordering::Relaxed), 1);
    b.fail_open = false;
    assert_eq!(
        contents_equal(&mut a, &mut b, Path::new("file"), &AtomicBool::new(true)).unwrap(),
        None
    );
    assert_eq!(a.finishes.load(Ordering::Relaxed), 2);
    assert_eq!(b.finishes.load(Ordering::Relaxed), 1);
}

#[test]
fn text_read_finalizes_even_on_encoding_errors() {
    let mut fs = FakeFs::new(&[0xef, 0xbb, 0xbf, 255]);
    assert!(text::read_text(&mut fs, Path::new("file"), &AtomicBool::new(false)).is_err());
    assert_eq!(fs.finishes.load(Ordering::Relaxed), 1);
}

#[test]
fn cancellation_after_discovery_preserves_pending_directories() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for root in [a.path(), b.path()] {
        std::fs::create_dir(root.join("directory")).unwrap();
        std::fs::write(root.join("directory/file"), b"abc").unwrap();
    }
    let cancel = AtomicBool::new(false);
    let rows = scan(
        &mut filesystem::LocalFs::new(a.path().into()),
        &mut filesystem::LocalFs::new(b.path().into()),
        &[],
        &cancel,
        |event| {
            if let ScanEvent::Rows(rows) = event
                && rows.iter().any(|row| {
                    row.path == Path::new("directory/file")
                        && row.left.is_some()
                        && row.right.is_some()
                })
            {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.status == Status::Pending));
}

#[test]
fn case_sensitive_pairing_and_unknown_sizes_do_not_imply_equality() {
    struct CaseFs {
        name: &'static str,
    }
    impl CompareFs for CaseFs {
        fn list(&mut self, _: &Path) -> Result<Vec<(OsString, Entry)>, String> {
            Ok(vec![(
                self.name.into(),
                Entry {
                    kind: Kind::File,
                    size: None,
                },
            )])
        }
        fn open(&mut self, _: &Path) -> Result<HostReader, String> {
            Err("unavailable".into())
        }
    }
    let mut a = CaseFs { name: "File" };
    let mut b = CaseFs { name: "file" };
    let rows = scan(&mut a, &mut b, &[], &AtomicBool::new(false), |_| {});
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].status, Status::LeftOnly);
    assert_eq!(rows[1].status, Status::RightOnly);
    b.name = "File";
    let rows = scan(&mut a, &mut b, &[], &AtomicBool::new(false), |_| {});
    assert_eq!(rows[0].status, Status::Error);
}

#[test]
fn failed_nested_listing_is_not_mistaken_for_a_missing_file() {
    struct TreeFs {
        fail: bool,
    }
    impl CompareFs for TreeFs {
        fn list(&mut self, path: &Path) -> Result<Vec<(OsString, Entry)>, String> {
            if path.as_os_str().is_empty() {
                Ok(vec![(
                    "directory".into(),
                    Entry {
                        kind: Kind::Directory,
                        size: None,
                    },
                )])
            } else if self.fail {
                Err("permission denied".into())
            } else {
                Ok(vec![(
                    "file".into(),
                    Entry {
                        kind: Kind::File,
                        size: Some(0),
                    },
                )])
            }
        }
        fn open(&mut self, _: &Path) -> Result<HostReader, String> {
            Ok(HostReader::io(Cursor::new(Vec::<u8>::new())))
        }
    }
    let rows = scan(
        &mut TreeFs { fail: false },
        &mut TreeFs { fail: true },
        &[],
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(
        rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>(),
        vec![PathBuf::from("directory"), PathBuf::from("directory/file")]
    );
    assert!(rows.iter().all(|row| row.status == Status::Error));
}
