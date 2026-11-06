//! Explicit whole-entry copies, independent of text decoding and diff limits.
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::filesystem::{CompareFs, Entry, Kind};

pub(super) enum Prepared {
    File(File),
    Directory,
}

pub(super) fn validate(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Copy requires a nonempty root-relative path".into());
    }
    Ok(())
}

/// Resolve each component through no-follow listings, including all ancestors.
/// Missing parents mean a missing entry; listing errors never mean absence.
pub(super) fn entry(fs: &mut dyn CompareFs, path: &Path) -> Result<Option<Entry>, String> {
    validate(path)?;
    let mut parent = PathBuf::new();
    for part in path.components() {
        let found = fs
            .list(&parent)?
            .into_iter()
            .find(|(name, _)| name == part.as_os_str());
        let Some((_, entry)) = found else {
            return Ok(None);
        };
        parent.push(part);
        if parent == path {
            return Ok(Some(entry));
        }
        if entry.kind != Kind::Directory {
            return Err(format!("{} is not a regular directory", parent.display()));
        }
    }
    unreachable!()
}

/// Discover before writing, so overlapping roots cannot recursively copy newly
/// created destination directories back into themselves.
pub(super) fn plan(
    fs: &mut dyn CompareFs,
    mut pending: Vec<PathBuf>,
    ignores: &[String],
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, String> {
    let mut visited = BTreeSet::new();
    let mut paths = Vec::new();
    while let Some(path) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Copy cancelled before writing".into());
        }
        if super::is_ignored(&path, ignores) || !visited.insert(path.clone()) {
            continue;
        }
        let source = entry(fs, &path)?
            .ok_or_else(|| format!("Copy source {} is missing", path.display()))?;
        match source.kind {
            Kind::Directory => {
                for (name, _) in fs.list(&path)? {
                    let name_path = Path::new(&name);
                    validate(name_path)?;
                    if name_path.components().count() != 1 {
                        return Err("Invalid directory entry name".into());
                    }
                    pending.push(path.join(name));
                }
            }
            Kind::File => {}
            _ => {
                return Err(format!(
                    "{}: symlinks and special files cannot be copied",
                    path.display()
                ));
            }
        }
        paths.push(path);
    }
    Ok(paths)
}

pub(super) fn prepare(
    fs: &mut dyn CompareFs,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let source = entry(fs, path)?.ok_or("Copy source is missing; refresh the comparison")?;
    match source.kind {
        Kind::Directory => Ok(Prepared::Directory),
        Kind::File => {
            let before = fs.fingerprint(path)?;
            let mut reader = fs.open(path)?;
            let result: Result<File, String> = (|| {
                let mut temporary = tempfile::tempfile().map_err(|error| error.to_string())?;
                let mut buffer = [0; 64 * 1024];
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Copy cancelled; earlier copies are not rolled back".into());
                    }
                    let count = reader
                        .read(&mut buffer)
                        .map_err(|error| error.to_string())?;
                    if count == 0 {
                        break;
                    }
                    temporary
                        .write_all(&buffer[..count])
                        .map_err(|error| error.to_string())?;
                }
                temporary.rewind().map_err(|error| error.to_string())?;
                Ok(temporary)
            })();
            let finish = reader.finish().map_err(|error| error.to_string());
            finish?;
            let file = result?;
            if before != fs.fingerprint(path)? {
                return Err("Source changed while reading; refresh before copying".into());
            }
            Ok(Prepared::File(file))
        }
        _ => Err("Symlinks and special files cannot be copied".into()),
    }
}

pub(super) fn ensure_directory(fs: &mut dyn CompareFs, path: &Path) -> Result<(), String> {
    validate(path)?;
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match entry(fs, &current)? {
            Some(entry) if entry.kind == Kind::Directory => {}
            Some(_) => return Err(format!("{} is not a regular directory", current.display())),
            None => fs.create_directory(&current)?,
        }
    }
    Ok(())
}

pub(super) fn install(fs: &mut dyn CompareFs, path: &Path, file: &mut File) -> Result<(), String> {
    validate(path)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        ensure_directory(fs, parent)?;
    }
    if entry(fs, path)?.is_some_and(|entry| entry.kind != Kind::File) {
        return Err("Destination is not a regular file; type conflicts are not replaced".into());
    }
    fs.copy_file(path, file)
}
