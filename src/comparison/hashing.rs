//! Full, finalized reads and strict validation around persistent hash reuse.
#[cfg(test)]
#[path = "hashing_tests.rs"]
mod tests;

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};

use sha2::{Digest, Sha256};

use super::cache::{ContentHash, HashCache};
use super::filesystem::{CompareFs, Kind};

#[derive(Debug)]
pub(super) struct VerifiedHash {
    pub hash: ContentHash,
    pub cached: bool,
}

#[derive(Debug)]
pub(super) enum Verification {
    Complete(VerifiedHash),
    Different,
}

enum Probe {
    Cached,
    Bytes(Vec<u8>),
}

enum PeerComparison {
    Equal,
    Different,
    DigestOnly,
}

/// A bounded chunk exchange preserves early mismatch detection without moving
/// protocol readers between threads or downloading whole changed files.
pub(super) struct Peer {
    send: SyncSender<Probe>,
    receive: Receiver<Probe>,
}

impl Peer {
    pub fn pair() -> [Self; 2] {
        let (a_send, a_receive) = mpsc::sync_channel(1);
        let (b_send, b_receive) = mpsc::sync_channel(1);
        [
            Self {
                send: a_send,
                receive: b_receive,
            },
            Self {
                send: b_send,
                receive: a_receive,
            },
        ]
    }

    fn compare(&self, bytes: &[u8]) -> Option<PeerComparison> {
        let _ = self.send.send(Probe::Bytes(bytes.into()));
        Some(match self.receive.recv().ok()? {
            Probe::Cached => PeerComparison::DigestOnly,
            Probe::Bytes(other) if bytes == other => PeerComparison::Equal,
            Probe::Bytes(_) => PeerComparison::Different,
        })
    }
}

#[cfg(test)]
fn verify(
    fs: &mut dyn CompareFs,
    cache: &mut HashCache,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Option<VerifiedHash>, String> {
    Ok(match verify_with_peer(fs, cache, path, cancel, None)? {
        Some(Verification::Complete(hash)) => Some(hash),
        None => None,
        Some(Verification::Different) => unreachable!("No peer was supplied"),
    })
}

pub(super) fn verify_with_peer(
    fs: &mut dyn CompareFs,
    cache: &mut HashCache,
    path: &Path,
    cancel: &AtomicBool,
    peer: Option<Peer>,
) -> Result<Option<Verification>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let before = fs.fingerprint(path)?;
    if before
        .as_ref()
        .is_some_and(|value| value.kind != Kind::File)
    {
        return Err("Not a regular file (or replaced since listing)".into());
    }
    let namespace = fs.cache_namespace();
    if let (Some(namespace), Some(live)) = (&namespace, &before)
        && let Some(hash) = cache.get(namespace, path, live)
    {
        if let Some(peer) = &peer {
            let _ = peer.send.send(Probe::Cached);
        }
        return Ok((!cancel.load(Ordering::Relaxed))
            .then_some(Verification::Complete(VerifiedHash { hash, cached: true })));
    }
    if let Some(namespace) = &namespace
        && before.as_ref().is_some_and(|value| value.version.is_some())
    {
        cache.invalidate(namespace, path);
    }
    let mut reader = fs.open(path)?;
    let result: Result<Option<Verification>, String> = (|| {
        let mut normalized = super::line_endings::Normalized::new(&mut reader);
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut peer = peer;
        loop {
            // Fill each chunk so differing legal short-read sizes cannot imply
            // a mismatch. Both endpoints read their chunks concurrently.
            let mut count = 0;
            while count < buffer.len() {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                match normalized.read(&mut buffer[count..]) {
                    Ok(0) => break,
                    Ok(read) => count += read,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            if let Some(other) = &peer {
                match other.compare(&buffer[..count]) {
                    Some(PeerComparison::Different) => return Ok(Some(Verification::Different)),
                    Some(PeerComparison::Equal) => {}
                    Some(PeerComparison::DigestOnly) => peer = None,
                    None => return Ok(None), // Peer failed/cancelled; still finalize below.
                }
            }
            if count == 0 {
                return Ok(Some(Verification::Complete(VerifiedHash {
                    hash: ContentHash {
                        size: normalized.raw_size,
                        digest: hash.finalize().into(),
                    },
                    cached: false,
                })));
            }
            hash.update(&buffer[..count]);
        }
    })();
    // A transfer is not successful until its protocol finalizer succeeds.
    let finish = reader.finish().map_err(|error| error.to_string());
    finish?;
    let Some(verification) = result? else {
        return Ok(None);
    };
    let after = fs.fingerprint(path)?;
    if before != after
        || after.as_ref().is_some_and(|value| {
            value.kind != Kind::File
                || match &verification {
                    Verification::Complete(hash) => {
                        value.size.is_some_and(|size| size != hash.hash.size)
                    }
                    Verification::Different => false,
                }
        })
    {
        return Err("File changed while verifying; refresh the comparison".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if let (Some(namespace), Verification::Complete(hash)) = (&namespace, &verification) {
        cache.put(namespace, path, after.as_ref(), hash.hash);
    }
    Ok(Some(verification))
}
