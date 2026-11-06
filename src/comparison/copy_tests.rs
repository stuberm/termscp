use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::filesystem::LocalFs;
use super::parallel::Pool;

fn pool(left: &std::path::Path, right: &std::path::Path) -> Pool {
    let left = left.to_path_buf();
    let right = right.to_path_buf();
    Pool::connect(
        move || Ok(Box::new(LocalFs::new(left.clone()))),
        move || Ok(Box::new(LocalFs::new(right.clone()))),
        None,
    )
    .unwrap()
}

#[test]
fn copies_binary_files_missing_targets_nested_folders_and_honors_ignores() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(left.path().join("folder/empty")).unwrap();
    let binary = vec![0xa5; 6 * 1024 * 1024]; // Above the text diff size limit.
    std::fs::write(left.path().join("folder/binary"), &binary).unwrap();
    std::fs::write(left.path().join("folder/ignored"), "private").unwrap();
    std::fs::write(left.path().join("paired"), b"source\r\n\0").unwrap();
    std::fs::write(right.path().join("paired"), "old").unwrap();
    std::fs::write(right.path().join("extra"), "keep").unwrap();
    let pool = pool(left.path(), right.path());
    let count = pool
        .copy_paths(
            vec!["folder".into(), "folder/binary".into(), "paired".into()],
            true,
            &["folder/ignored".into()],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(count, 4); // Folder, empty directory, binary, paired; no duplicates.
    assert_eq!(
        std::fs::read(right.path().join("folder/binary")).unwrap(),
        binary
    );
    assert!(right.path().join("folder/empty").is_dir());
    assert!(!right.path().join("folder/ignored").exists());
    assert_eq!(
        std::fs::read(right.path().join("paired")).unwrap(),
        b"source\r\n\0"
    );
    assert_eq!(
        std::fs::read_to_string(right.path().join("extra")).unwrap(),
        "keep"
    );
    std::fs::write(right.path().join("paired"), "reverse").unwrap();
    pool.copy_paths(
        vec!["paired".into()],
        false,
        &[],
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(left.path().join("paired")).unwrap(),
        "reverse"
    );
}

#[test]
fn remote_adapter_creates_and_overwrites_files_via_the_host_bridge() {
    use super::filesystem::RemoteFs;
    use crate::host::Localhost;
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::create_dir(left.path().join("folder")).unwrap();
    std::fs::write(left.path().join("folder/file"), b"binary\0\r\n").unwrap();
    let source = left.path().to_path_buf();
    let target = right.path().to_path_buf();
    let pool = Pool::connect(
        move || {
            Ok(Box::new(RemoteFs {
                host: Box::new(Localhost::new(source.clone()).unwrap()),
                root: source.clone(),
                namespace: None,
                version_probe: None,
            }))
        },
        move || {
            Ok(Box::new(RemoteFs {
                host: Box::new(Localhost::new(target.clone()).unwrap()),
                root: target.clone(),
                namespace: None,
                version_probe: None,
            }))
        },
        None,
    )
    .unwrap();
    pool.copy_paths(
        vec!["folder".into()],
        true,
        &[],
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(right.path().join("folder/file")).unwrap(),
        b"binary\0\r\n"
    );
    std::fs::write(right.path().join("folder/file"), "reverse").unwrap();
    pool.copy_paths(
        vec!["folder/file".into()],
        false,
        &[],
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(left.path().join("folder/file")).unwrap(),
        "reverse"
    );
}

#[test]
fn refuses_conflicts_missing_sources_invalid_paths_and_cancelled_operations() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::write(left.path().join("file"), "source").unwrap();
    std::fs::create_dir(right.path().join("file")).unwrap();
    std::fs::create_dir(left.path().join("folder")).unwrap();
    std::fs::write(right.path().join("folder"), "keep").unwrap();
    let pool = pool(left.path(), right.path());
    for path in ["file", "folder", "missing", "../escape", ""] {
        assert!(
            pool.copy_paths(
                vec![PathBuf::from(path)],
                true,
                &[],
                &Arc::new(AtomicBool::new(false))
            )
            .is_err(),
            "{path}"
        );
    }
    assert!(right.path().join("file").is_dir());
    assert_eq!(
        std::fs::read_to_string(right.path().join("folder")).unwrap(),
        "keep"
    );
    assert!(
        pool.copy_paths(
            vec!["file".into()],
            true,
            &[],
            &Arc::new(AtomicBool::new(true))
        )
        .is_err()
    );
}

#[test]
fn discovery_finishes_before_writes_even_with_overlapping_roots() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    std::fs::write(root.path().join("sub/file"), "original").unwrap();
    let pool = pool(root.path(), &root.path().join("sub"));
    pool.copy_paths(
        vec!["sub".into()],
        true,
        &[],
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("sub/sub/file")).unwrap(),
        "original"
    );
    assert!(!root.path().join("sub/sub/sub").exists());
}

#[cfg(posix)]
#[test]
fn refuses_source_destination_and_ancestor_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("file"), "untouched").unwrap();
    std::fs::write(left.path().join("file"), "source").unwrap();
    symlink(outside.path().join("file"), right.path().join("file")).unwrap();
    symlink(outside.path(), left.path().join("link")).unwrap();
    std::fs::create_dir(left.path().join("dir")).unwrap();
    std::fs::write(left.path().join("dir/file"), "source").unwrap();
    symlink(outside.path(), right.path().join("dir")).unwrap();
    let pool = pool(left.path(), right.path());
    for path in ["file", "link", "link/file", "dir/file"] {
        assert!(
            pool.copy_paths(
                vec![path.into()],
                true,
                &[],
                &Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
    }
    assert_eq!(
        std::fs::read_to_string(outside.path().join("file")).unwrap(),
        "untouched"
    );
    std::fs::remove_file(right.path().join("file")).unwrap();
    std::fs::write(right.path().join("file"), "old").unwrap();
    std::fs::set_permissions(
        right.path().join("file"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    pool.copy_paths(
        vec!["file".into()],
        true,
        &[],
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(right.path().join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}
