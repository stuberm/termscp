//! Disposable, strict per-endpoint SHA-256 cache. Timestamps are never validators.
#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::filesystem::Fingerprint;

pub(super) type Namespace = [u8; 32];

// SQLite permits only one WAL writer. Serialize same-process setup/writes so
// parallel verification lanes do not exhaust the short busy timeout and disable
// each other's caches. Live reads, hashing and cache lookups remain concurrent.
// The busy handler still covers contention with other processes.
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ContentHash {
    pub size: u64,
    pub digest: [u8; 32],
}

/// Hash native bytes, not lossy display strings (including on Windows).
/// The platform and schema prefix prevent cross-platform cache-key ambiguity.
pub(super) fn namespace(identity: &[u8]) -> Namespace {
    let mut hash = Sha256::new();
    hash.update(b"sidemirror-hash-cache-crlf-v2\0");
    hash.update(std::env::consts::OS.as_bytes());
    hash.update([0]);
    hash.update(identity);
    hash.finalize().into()
}

fn path_key(path: &Path) -> [u8; 32] {
    Sha256::digest(path.as_os_str().as_encoded_bytes()).into()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[derive(Default)]
pub(super) struct HashCache {
    connection: Option<Connection>,
    writes: usize,
}

impl HashCache {
    pub fn open(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let _write = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = (|| {
            let connection = Connection::open(path)?;
            connection.busy_timeout(Duration::from_millis(100))?;
            let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            if !matches!(version, 0 | 1) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            connection.execute_batch(
                "PRAGMA trusted_schema = OFF;
                 PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 CREATE TABLE IF NOT EXISTS file_hashes (
                    endpoint BLOB NOT NULL,
                    path BLOB NOT NULL,
                    validator TEXT,
                    size BLOB NOT NULL CHECK(length(size) = 8),
                    sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
                    verified_at INTEGER NOT NULL,
                    PRIMARY KEY(endpoint, path)
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS file_hashes_verified_at
                    ON file_hashes(verified_at);
                 PRAGMA user_version = 1;",
            )?;
            Self::prune(&connection)?;
            Ok(connection)
        })();
        match result {
            Ok(connection) => Self {
                connection: Some(connection),
                writes: 0,
            },
            Err(error) => {
                log::warn!("Comparison hash cache unavailable; using live reads: {error}");
                Self::default()
            }
        }
    }

    fn prune(connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "DELETE FROM file_hashes WHERE verified_at < ?1",
            [now().saturating_sub(30 * 24 * 60 * 60)],
        )?;
        connection.execute(
            "DELETE FROM file_hashes WHERE (endpoint, path) IN (
                SELECT endpoint, path FROM file_hashes
                ORDER BY verified_at DESC LIMIT -1 OFFSET 100000
             )",
            [],
        )?;
        Ok(())
    }

    /// A live strong validator is mandatory, even for observations made seconds ago.
    pub fn get(
        &mut self,
        endpoint: &Namespace,
        path: &Path,
        live: &Fingerprint,
    ) -> Option<ContentHash> {
        let validator = live
            .version
            .as_deref()
            .filter(|version| !version.is_empty())?;
        let connection = self.connection.as_ref()?;
        let result = connection
            .query_row(
                "SELECT size, sha256 FROM file_hashes
                 WHERE endpoint = ?1 AND path = ?2 AND validator = ?3",
                params![endpoint.as_slice(), path_key(path).as_slice(), validator],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional();
        match result {
            Ok(Some((size, digest))) => {
                let size = u64::from_be_bytes(size.try_into().ok()?);
                if live.size.is_some_and(|live| live != size) {
                    return None;
                }
                Some(ContentHash {
                    size,
                    digest: digest.try_into().ok()?,
                })
            }
            Ok(None) => None,
            Err(error) => {
                self.disable(error);
                None
            }
        }
    }

    /// Store only complete, finalized reads, never partial/error/cancelled streams.
    pub fn put(
        &mut self,
        endpoint: &Namespace,
        path: &Path,
        fingerprint: Option<&Fingerprint>,
        hash: ContentHash,
    ) {
        // Persisting observations we can never safely reuse only adds I/O to
        // strict local/SFTP/FTP scans. No strong validator means no cache entry.
        let Some(validator) = fingerprint
            .and_then(|value| value.version.as_deref())
            .filter(|version| !version.is_empty())
        else {
            return;
        };
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        let _write = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = connection.execute(
            "INSERT INTO file_hashes(endpoint, path, validator, size, sha256, verified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(endpoint, path) DO UPDATE SET
                validator = excluded.validator, size = excluded.size,
                sha256 = excluded.sha256, verified_at = excluded.verified_at",
            params![
                endpoint.as_slice(),
                path_key(path).as_slice(),
                validator,
                hash.size.to_be_bytes().as_slice(),
                hash.digest.as_slice(),
                now(),
            ],
        );
        if let Err(error) = result {
            self.disable(error);
            return;
        }
        self.writes += 1;
        if self.writes.is_multiple_of(128)
            && let Err(error) = Self::prune(connection)
        {
            self.disable(error);
        }
    }

    pub fn invalidate(&mut self, endpoint: &Namespace, path: &Path) {
        let _write = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(connection) = &self.connection
            && let Err(error) = connection.execute(
                "DELETE FROM file_hashes WHERE endpoint = ?1 AND path = ?2",
                params![endpoint.as_slice(), path_key(path).as_slice()],
            )
        {
            self.disable(error);
        }
    }

    fn disable(&mut self, error: rusqlite::Error) {
        log::warn!("Comparison hash cache disabled; using live reads: {error}");
        self.connection = None;
    }
}
