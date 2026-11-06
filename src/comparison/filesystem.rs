//! Filesystem access for scanning and explicitly confirmed copying.
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::cache::{self, Namespace};
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
    /// Listing metadata for quick comparisons, never a content-cache validator.
    pub modified: Option<SystemTime>,
}

/// Metadata can detect an interrupted/changing read, but is not proof of equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Only a strong identifier of the actual bytes; never mtime or a weak ETag.
    pub version: Option<String>,
}

pub trait CompareFs {
    fn cache_namespace(&self) -> Option<Namespace> {
        None
    }
    fn fingerprint(&mut self, _relative: &Path) -> Result<Option<Fingerprint>, String> {
        Ok(None)
    }
    fn list(&mut self, relative: &Path) -> Result<Vec<(OsString, Entry)>, String>;
    fn open(&mut self, relative: &Path) -> Result<HostReader, String>;
    fn write_bytes(&mut self, _relative: &Path, _bytes: &[u8]) -> Result<(), String> {
        Err("Writing is unsupported by this filesystem".into())
    }
    fn create_directory(&mut self, _relative: &Path) -> Result<(), String> {
        Err("Creating directories is unsupported by this filesystem".into())
    }
    fn copy_file(&mut self, _relative: &Path, _file: &mut fs::File) -> Result<(), String> {
        Err("Copying files is unsupported by this filesystem".into())
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
    fn cache_namespace(&self) -> Option<Namespace> {
        Some(cache::namespace(self.root.as_os_str().as_encoded_bytes()))
    }

    fn fingerprint(&mut self, relative: &Path) -> Result<Option<Fingerprint>, String> {
        let metadata =
            fs::symlink_metadata(self.root.join(relative)).map_err(|error| error.to_string())?;
        Ok(Some(Fingerprint {
            kind: if metadata.is_file() {
                Kind::File
            } else {
                Kind::Special
            },
            size: Some(metadata.len()),
            modified: metadata.modified().ok(),
            // Portable filesystem metadata has no trustworthy content version.
            version: None,
        }))
    }

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

    fn create_directory(&mut self, relative: &Path) -> Result<(), String> {
        fs::create_dir(self.root.join(relative)).map_err(|error| error.to_string())
    }

    fn copy_file(&mut self, relative: &Path, file: &mut fs::File) -> Result<(), String> {
        let path = self.root.join(relative);
        let permissions = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
            Ok(_) => return Err("Target is no longer a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent")?)
            .map_err(|error| error.to_string())?;
        std::io::copy(file, &mut temporary).map_err(|error| error.to_string())?;
        if let Some(permissions) = &permissions {
            temporary
                .as_file()
                .set_permissions(permissions.clone())
                .map_err(|error| error.to_string())?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        if permissions.is_some() {
            temporary
                .persist(&path)
                .map_err(|error| error.to_string())?;
        } else {
            temporary
                .persist_noclobber(&path)
                .map_err(|error| error.to_string())?;
        }
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
                        modified: metadata.modified().ok(),
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

pub(super) type VersionProbe = Box<dyn FnMut(&Path) -> Result<Option<String>, String>>;

pub struct RemoteFs {
    pub host: Box<dyn HostBridge>,
    pub root: PathBuf,
    pub(super) namespace: Option<Namespace>,
    pub(super) version_probe: Option<VersionProbe>,
}

impl CompareFs for RemoteFs {
    fn cache_namespace(&self) -> Option<Namespace> {
        self.namespace
    }

    fn fingerprint(&mut self, relative: &Path) -> Result<Option<Fingerprint>, String> {
        let path = self.root.join(relative);
        let file = self.host.stat(&path).map_err(|error| error.to_string())?;
        let metadata = file.metadata();
        let version = match &mut self.version_probe {
            Some(probe) => probe(&path)?,
            None => None,
        };
        Ok(Some(Fingerprint {
            kind: if file.is_file() && metadata.symlink.is_none() {
                Kind::File
            } else {
                Kind::Special
            },
            size: metadata.size,
            modified: metadata.modified,
            version,
        }))
    }

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

    fn create_directory(&mut self, relative: &Path) -> Result<(), String> {
        self.host
            .mkdir(&self.root.join(relative))
            .map_err(|error| error.to_string())
    }

    fn copy_file(&mut self, relative: &Path, source: &mut fs::File) -> Result<(), String> {
        let path = self.root.join(relative);
        let mut metadata = match super::copy::entry(self, relative)? {
            Some(entry) if entry.kind == Kind::File => {
                let file = self.host.stat(&path).map_err(|error| error.to_string())?;
                if !file.is_file() || file.metadata().symlink.is_some() {
                    return Err("Target is no longer a regular file".into());
                }
                file.metadata().clone()
            }
            Some(_) => return Err("Target is no longer a regular file".into()),
            None => remotefs::fs::Metadata::default(),
        };
        metadata.size = Some(source.metadata().map_err(|error| error.to_string())?.len());
        let mut writer = self
            .host
            .create_file(&path, &metadata)
            .map_err(|error| error.to_string())?;
        let result = std::io::copy(source, &mut writer).map_err(|error| error.to_string());
        let finish = self
            .host
            .finalize_write(writer)
            .map_err(|error| error.to_string());
        finish?;
        result.map(|_| ())
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
                                modified: file.metadata().modified,
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
