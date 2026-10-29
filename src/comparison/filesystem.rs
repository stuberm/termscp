//! Filesystem access for scanning and explicitly confirmed text replacement.
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::host::{HostBridge, HostReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink,
    Special,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub size: Option<u64>,
}

pub trait CompareFs {
    fn list(&mut self, relative: &Path) -> Result<Vec<(OsString, Entry)>, String>;
    fn open(&mut self, relative: &Path) -> Result<HostReader, String>;
    fn write_bytes(&mut self, _relative: &Path, _bytes: &[u8]) -> Result<(), String> {
        Err("Writing is unsupported by this filesystem".into())
    }
}

pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl CompareFs for LocalFs {
    fn write_bytes(&mut self, relative: &Path, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        let path = self.root.join(relative);
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("Target is no longer a regular file".into());
        }
        let permissions = fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions();
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent")?)
            .map_err(|e| e.to_string())?;
        temporary.write_all(bytes).map_err(|e| e.to_string())?;
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(&path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list(&mut self, relative: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        let path = self.root.join(relative);
        // Do not traverse a directory replaced by a symlink since discovery.
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_dir() {
            return Err(format!("{} is no longer a directory", path.display()));
        }
        fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .map(|entry| {
                let entry = entry.map_err(|error| error.to_string())?;
                let metadata =
                    fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
                let kind = if metadata.is_symlink() {
                    Kind::Symlink
                } else if metadata.is_dir() {
                    Kind::Directory
                } else if metadata.is_file() {
                    Kind::File
                } else {
                    Kind::Special
                };
                Ok((
                    entry.file_name(),
                    Entry {
                        kind,
                        size: Some(metadata.len()),
                    },
                ))
            })
            .collect()
    }

    fn open(&mut self, relative: &Path) -> Result<HostReader, String> {
        let path = self.root.join(relative);
        if !fs::symlink_metadata(&path)
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Not a regular file (or replaced by a symlink)".into());
        }
        fs::File::open(path)
            .map(HostReader::io)
            .map_err(|error| error.to_string())
    }
}

pub struct RemoteFs {
    pub host: Box<dyn HostBridge>,
    pub root: PathBuf,
}

impl CompareFs for RemoteFs {
    fn write_bytes(&mut self, relative: &Path, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        let path = self.root.join(relative);
        let file = self.host.stat(&path).map_err(|e| e.to_string())?;
        if !file.is_file() || file.metadata().symlink.is_some() {
            return Err("Target is no longer a regular file".into());
        }
        let mut writer = self
            .host
            .create_file(&path, file.metadata())
            .map_err(|e| e.to_string())?;
        let result = writer.write_all(bytes).map_err(|e| e.to_string());
        let finish = self.host.finalize_write(writer).map_err(|e| e.to_string());
        result.and(finish)
    }

    fn list(&mut self, relative: &Path) -> Result<Vec<(OsString, Entry)>, String> {
        let path = self.root.join(relative);
        let directory = self.host.stat(&path).map_err(|error| error.to_string())?;
        if !directory.is_dir() || directory.metadata().symlink.is_some() {
            return Err(format!("{} is no longer a directory", path.display()));
        }
        self.host
            .list_dir(&path)
            .map_err(|error| error.to_string())
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| {
                        let kind = if file.metadata().symlink.is_some()
                            || file.metadata().file_type == remotefs::fs::FileType::Symlink
                        {
                            Kind::Symlink
                        } else if file.is_dir() {
                            Kind::Directory
                        } else if file.is_file() {
                            Kind::File
                        } else {
                            Kind::Special
                        };
                        // File::name is a display string; preserve native path components where possible.
                        let name = file.path().file_name().unwrap_or_default().to_os_string();
                        (
                            name,
                            Entry {
                                kind,
                                size: file.metadata().size,
                            },
                        )
                    })
                    .collect()
            })
    }

    fn open(&mut self, relative: &Path) -> Result<HostReader, String> {
        let path = self.root.join(relative);
        let file = self.host.stat(&path).map_err(|error| error.to_string())?;
        if !file.is_file() || file.metadata().symlink.is_some() {
            return Err("Not a regular file".into());
        }
        self.host
            .open_file(&path)
            .map_err(|error| error.to_string())
    }
}

impl Drop for RemoteFs {
    fn drop(&mut self) {
        let _ = self.host.disconnect();
    }
}
