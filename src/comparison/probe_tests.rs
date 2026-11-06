//! Early mismatch and peer failure must never deadlock or cache partial reads.
use std::ffi::OsString;
use std::io::{self, Cursor, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use remotefs::fs::{ReadStream, RemoteRead};

use super::Status;
use super::filesystem::{CompareFs, Entry, Kind};
use super::parallel::Pool;
use crate::host::HostReader;

struct Reader {
    cursor: Cursor<Vec<u8>>,
    count: Arc<AtomicUsize>,
    finishes: Arc<AtomicUsize>,
    fail_read: bool,
    fail_finish: bool,
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        // Different short-read lengths on the two sides are legal.
        let length = buffer.len().min(1237);
        let count = self.cursor.read(&mut buffer[..length])?;
        self.count.fetch_add(count, Ordering::Relaxed);
        Ok(count)
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

struct Fs {
    data: Vec<u8>,
    count: Arc<AtomicUsize>,
    finishes: Arc<AtomicUsize>,
    fail_open: bool,
    fail_read: bool,
    fail_finish: bool,
}

impl CompareFs for Fs {
    fn list(&mut self, _: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        Ok(vec![(
            "file".into(),
            Entry {
                kind: Kind::File,
                size: Some(self.data.len() as u64),
                modified: None,
            },
        )])
    }
    fn open(&mut self, _: &Path) -> Result<HostReader, String> {
        if self.fail_open {
            return Err("open failed".into());
        }
        Ok(HostReader::remote(ReadStream::new(Reader {
            cursor: Cursor::new(self.data.clone()),
            count: self.count.clone(),
            finishes: self.finishes.clone(),
            fail_read: self.fail_read,
            fail_finish: self.fail_finish,
        })))
    }
}

#[test]
fn matching_prefixes_do_not_imply_equality_and_equal_files_are_read_completely() {
    for different in [false, true] {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let finishes = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let make = |side: usize| {
            let count = counts[side].clone();
            let finishes = finishes[side].clone();
            move || -> Result<Box<dyn CompareFs>, String> {
                let mut data = vec![0; 128 * 1024];
                if different && side == 1 {
                    *data.last_mut().unwrap() = 1;
                }
                Ok(Box::new(Fs {
                    data,
                    count: count.clone(),
                    finishes: finishes.clone(),
                    fail_open: false,
                    fail_read: false,
                    fail_finish: false,
                }))
            }
        };
        let pool = Pool::connect(make(0), make(1), None).unwrap();
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(
            rows[0].status,
            if different {
                Status::Different
            } else {
                Status::Equal
            }
        );
        for side in 0..2 {
            assert_eq!(counts[side].load(Ordering::Relaxed), 128 * 1024);
            assert_eq!(finishes[side].load(Ordering::Relaxed), 1);
        }
    }
}

#[test]
fn mismatches_stop_in_the_first_differing_chunk_and_both_streams_are_finalized() {
    for failure in 0..4 {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let finishes = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let make = |side: usize| {
            let count = counts[side].clone();
            let finishes = finishes[side].clone();
            move || -> Result<Box<dyn CompareFs>, String> {
                Ok(Box::new(Fs {
                    // A shared first chunk must not force downloading the rest
                    // of a large file once the next chunk differs.
                    data: {
                        let mut data = vec![0; 2 * 1024 * 1024];
                        if side == 1 {
                            data[70 * 1024] = 1;
                        }
                        data
                    },
                    count: count.clone(),
                    finishes: finishes.clone(),
                    fail_open: side == 0 && failure == 1,
                    fail_read: side == 0 && failure == 2,
                    fail_finish: side == 0 && failure == 3,
                }))
            }
        };
        let pool = Pool::connect(make(0), make(1), None).unwrap();
        let rows = pool
            .scan(&[], &Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(
            rows[0].status,
            if failure == 0 {
                Status::Different
            } else {
                Status::Error
            }
        );
        for side in 0..2 {
            let count = counts[side].load(Ordering::Relaxed);
            // Normalization buffers up to 8 KiB of raw input ahead.
            assert!(count <= 128 * 1024 + 8 * 1024);
            if failure == 0 {
                // BufReader may prefetch beyond the two compared chunks,
                // especially when the underlying reader returns short reads.
                assert!(count >= 128 * 1024);
            }
            assert_eq!(
                finishes[side].load(Ordering::Relaxed),
                usize::from(!(side == 0 && failure == 1))
            );
        }
    }
}
