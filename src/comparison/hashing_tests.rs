use std::ffi::OsString;
use std::io::{self, Cursor, Read};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::UNIX_EPOCH;

use remotefs::fs::{ReadStream, RemoteRead};

use super::*;
use crate::comparison::cache::{self, Namespace};
use crate::comparison::filesystem::Fingerprint;
use crate::host::HostReader;

struct Reader {
    bytes: Cursor<Vec<u8>>,
    finishes: Arc<AtomicUsize>,
    fail_read: bool,
    fail_finish: bool,
    cancel: Option<Arc<AtomicBool>>,
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        let count = buffer.len().min(2); // Real streams may return short reads.
        self.bytes.read(&mut buffer[..count])
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
    bytes: Vec<u8>,
    version: Option<String>,
    opens: usize,
    stats: usize,
    finishes: Arc<AtomicUsize>,
    fail_read: bool,
    fail_finish: bool,
    fail_stat: bool,
    change_after_read: bool,
    cancel_on_read: Option<Arc<AtomicBool>>,
}

impl Fs {
    fn new(version: Option<&str>) -> Self {
        Self {
            bytes: b"abc".to_vec(),
            version: version.map(str::to_owned),
            opens: 0,
            stats: 0,
            finishes: Arc::default(),
            fail_read: false,
            fail_finish: false,
            fail_stat: false,
            change_after_read: false,
            cancel_on_read: None,
        }
    }
}

impl CompareFs for Fs {
    fn cache_namespace(&self) -> Option<Namespace> {
        Some(cache::namespace(b"test"))
    }
    fn fingerprint(&mut self, _: &Path) -> Result<Option<Fingerprint>, String> {
        self.stats += 1;
        if self.fail_stat {
            return Err("stat failed".into());
        }
        if self.change_after_read && self.stats > 1 {
            self.version = Some("changed".into());
        }
        Ok(Some(Fingerprint {
            kind: Kind::File,
            size: Some(self.bytes.len() as u64),
            modified: Some(UNIX_EPOCH),
            version: self.version.clone(),
        }))
    }
    fn list(
        &mut self,
        _: &Path,
    ) -> Result<Vec<(OsString, crate::comparison::filesystem::Entry)>, String> {
        unreachable!()
    }
    fn open(&mut self, _: &Path) -> Result<HostReader, String> {
        self.opens += 1;
        Ok(HostReader::remote(ReadStream::new(Reader {
            bytes: Cursor::new(self.bytes.clone()),
            finishes: self.finishes.clone(),
            fail_read: self.fail_read,
            fail_finish: self.fail_finish,
            cancel: self.cancel_on_read.clone(),
        })))
    }
}

#[test]
fn chunk_normalization_keeps_sha256_and_raw_sizes_compatible() {
    let mut fs = Fs::new(None);
    fs.bytes = b"line\r\n".repeat(20_000);
    fs.bytes.extend_from_slice(b"last\r");
    let mut normalized = b"line\n".repeat(20_000);
    normalized.extend_from_slice(b"last\r");
    let hash = verify(
        &mut fs,
        &mut HashCache::default(),
        Path::new("file"),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        hash.hash.digest,
        <[u8; 32]>::from(Sha256::digest(&normalized))
    );
    assert_eq!(hash.hash.size, fs.bytes.len() as u64);
    assert_eq!(fs.stats, 2);
    assert_eq!(fs.finishes.load(Ordering::Relaxed), 1);
}

#[test]
fn strong_versions_reuse_only_fully_read_finalized_hashes() {
    let directory = tempfile::tempdir().unwrap();
    let mut cache = HashCache::open(Some(&directory.path().join("hashes.sqlite3")));
    let mut fs = Fs::new(Some("v1"));
    let cancel = AtomicBool::new(false);
    let first = verify(&mut fs, &mut cache, Path::new("file"), &cancel)
        .unwrap()
        .unwrap();
    assert!(!first.cached);
    assert_eq!(first.hash.digest, <[u8; 32]>::from(Sha256::digest(b"abc")));
    assert_eq!(fs.finishes.load(Ordering::Relaxed), 1);
    let second = verify(&mut fs, &mut cache, Path::new("file"), &cancel)
        .unwrap()
        .unwrap();
    assert!(second.cached);
    assert_eq!(fs.opens, 1);
    fs.version = Some("v2".into());
    fs.bytes = b"xyz".to_vec(); // Same length and timestamp, new strong version.
    let third = verify(&mut fs, &mut cache, Path::new("file"), &cancel)
        .unwrap()
        .unwrap();
    assert!(!third.cached);
    assert_ne!(first.hash, third.hash);
    fs.fail_stat = true;
    assert!(verify(&mut fs, &mut cache, Path::new("file"), &cancel).is_err());
}

#[test]
fn same_size_and_mtime_never_skip_live_reads_without_a_strong_version() {
    let directory = tempfile::tempdir().unwrap();
    let mut cache = HashCache::open(Some(&directory.path().join("hashes.sqlite3")));
    let mut fs = Fs::new(None);
    let cancel = AtomicBool::new(false);
    let first = verify(&mut fs, &mut cache, Path::new("file"), &cancel)
        .unwrap()
        .unwrap();
    fs.bytes = b"xyz".to_vec();
    let second = verify(&mut fs, &mut cache, Path::new("file"), &cancel)
        .unwrap()
        .unwrap();
    assert!(!second.cached);
    assert_ne!(first.hash, second.hash);
    assert_eq!(fs.opens, 2);
}

#[test]
fn read_finish_cancellation_and_mid_read_changes_cannot_publish_a_cached_hash() {
    for scenario in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = HashCache::open(Some(&directory.path().join("hashes.sqlite3")));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut fs = Fs::new(Some("v1"));
        match scenario {
            0 => fs.fail_read = true,
            1 => fs.fail_finish = true,
            2 => fs.cancel_on_read = Some(cancel.clone()),
            _ => fs.change_after_read = true,
        }
        let result = verify(&mut fs, &mut cache, Path::new("file"), &cancel);
        if scenario == 2 {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
        assert_eq!(fs.finishes.load(Ordering::Relaxed), 1);
        let live = fs.fingerprint(Path::new("file")).unwrap().unwrap();
        assert_eq!(
            cache.get(&fs.cache_namespace().unwrap(), Path::new("file"), &live),
            None
        );
    }
}
