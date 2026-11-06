use super::*;
use crate::comparison::filesystem::Kind;

fn fingerprint(version: Option<&str>) -> Fingerprint {
    Fingerprint {
        kind: Kind::File,
        size: Some(3),
        modified: Some(UNIX_EPOCH),
        version: version.map(str::to_owned),
    }
}

fn hash() -> ContentHash {
    ContentHash {
        size: 3,
        digest: Sha256::digest(b"abc").into(),
    }
}

#[test]
fn persists_hashes_but_requires_live_strong_version_size_endpoint_and_path() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hashes.sqlite3");
    let endpoint = namespace(b"host/user/root");
    let path = Path::new("file");
    let live = fingerprint(Some("strong:v1"));
    {
        let mut cache = HashCache::open(Some(&database));
        cache.put(&endpoint, path, Some(&live), hash());
    }
    let mut cache = HashCache::open(Some(&database));
    assert_eq!(cache.get(&endpoint, path, &live), Some(hash()));
    assert_eq!(cache.get(&endpoint, path, &fingerprint(None)), None);
    assert_eq!(
        cache.get(&endpoint, path, &fingerprint(Some("strong:v2"))),
        None
    );
    assert_eq!(cache.get(&namespace(b"other/root"), path, &live), None);
    assert_eq!(cache.get(&endpoint, Path::new("other"), &live), None);
    let mut resized = live.clone();
    resized.size = Some(4);
    assert_eq!(cache.get(&endpoint, path, &resized), None);
    cache.invalidate(&endpoint, path);
    assert_eq!(cache.get(&endpoint, path, &live), None);
}

#[test]
fn metadata_only_observations_can_never_be_cache_hits() {
    let directory = tempfile::tempdir().unwrap();
    let mut cache = HashCache::open(Some(&directory.path().join("hashes.sqlite3")));
    let endpoint = namespace(b"local/root");
    let live = fingerprint(None);
    cache.put(&endpoint, Path::new("file"), Some(&live), hash());
    let count: i64 = cache
        .connection
        .as_ref()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM file_hashes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "Do not add database writes for uncacheable backends"
    );
    assert_eq!(cache.get(&endpoint, Path::new("file"), &live), None);
    assert_eq!(
        cache.get(&endpoint, Path::new("file"), &fingerprint(Some(""))),
        None
    );
}

#[test]
fn unavailable_corrupt_future_and_locked_databases_are_disposable() {
    let directory = tempfile::tempdir().unwrap();
    assert!(HashCache::open(Some(directory.path())).connection.is_none());
    let corrupt = directory.path().join("corrupt.sqlite3");
    std::fs::write(&corrupt, b"not a database").unwrap();
    assert!(HashCache::open(Some(&corrupt)).connection.is_none());
    let future = directory.path().join("future.sqlite3");
    let connection = Connection::open(&future).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 99")
        .unwrap();
    assert!(HashCache::open(Some(&future)).connection.is_none());
    let database = directory.path().join("locked.sqlite3");
    let mut cache = HashCache::open(Some(&database));
    let lock = Connection::open(&database).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    cache.put(
        &namespace(b"root"),
        Path::new("file"),
        Some(&fingerprint(Some("v1"))),
        hash(),
    );
    assert!(cache.connection.is_none());
    lock.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn malformed_hashes_do_not_become_hits_and_old_observations_are_pruned() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hashes.sqlite3");
    let mut cache = HashCache::open(Some(&database));
    let endpoint = namespace(b"root");
    let path = Path::new("file");
    let live = fingerprint(Some("v1"));
    cache.put(&endpoint, path, Some(&live), hash());
    let connection = cache.connection.as_ref().unwrap();
    connection
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
        UPDATE file_hashes SET sha256 = X'01';",
        )
        .unwrap();
    assert_eq!(cache.get(&endpoint, path, &live), None);
    cache.put(&endpoint, path, Some(&live), hash());
    cache
        .connection
        .as_ref()
        .unwrap()
        .execute("UPDATE file_hashes SET verified_at = 0", [])
        .unwrap();
    drop(cache);
    assert_eq!(
        HashCache::open(Some(&database)).get(&endpoint, path, &live),
        None
    );
}

#[test]
fn cache_pruning_bounds_the_number_of_retained_files() {
    let directory = tempfile::tempdir().unwrap();
    let cache = HashCache::open(Some(&directory.path().join("hashes.sqlite3")));
    let connection = cache.connection.as_ref().unwrap();
    connection.execute(
        "WITH RECURSIVE files(n) AS (VALUES(1) UNION ALL SELECT n + 1 FROM files WHERE n < 100005)
         INSERT INTO file_hashes(endpoint, path, validator, size, sha256, verified_at)
         SELECT zeroblob(32), CAST(n AS BLOB), NULL, zeroblob(8), zeroblob(32), ?1 FROM files",
        [now()],
    ).unwrap();
    HashCache::prune(connection).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM file_hashes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 100000);
}

#[cfg(posix)]
#[test]
fn cache_keys_preserve_non_utf8_paths() {
    use std::os::unix::ffi::OsStrExt;
    let a = Path::new(std::ffi::OsStr::from_bytes(b"a\xff"));
    let b = Path::new(std::ffi::OsStr::from_bytes(b"a\xfe"));
    assert_eq!(a.to_string_lossy(), b.to_string_lossy());
    assert_ne!(path_key(a), path_key(b));
    assert_ne!(
        namespace(a.as_os_str().as_encoded_bytes()),
        namespace(b.as_os_str().as_encoded_bytes())
    );
}
