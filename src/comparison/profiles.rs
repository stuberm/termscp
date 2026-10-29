//! Saved comparison sessions. Both endpoints are typed; secrets never belong in profiles.
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use super::connection::ConnectionSettings;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Endpoint {
    Local { path: PathBuf },
    Remote { url: String },
    Bookmark { name: String },
    Connection { settings: Box<ConnectionSettings> },
}

impl Endpoint {
    pub fn from_input(value: &str) -> Result<Self, String> {
        let endpoint = if let Some(name) = value.strip_prefix("bookmark:") {
            Self::Bookmark { name: name.into() }
        } else if value.contains("://") {
            Self::Remote { url: value.into() }
        } else {
            Self::Local { path: value.into() }
        };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Local { path } if path.as_os_str().is_empty() => {
                Err("Select a local folder".into())
            }
            Self::Local { .. } => Ok(()),
            Self::Remote { url } => remote_url(url).map(|_| ()),
            Self::Bookmark { name } if name.trim().is_empty() => {
                Err("Select a connection bookmark".into())
            }
            Self::Bookmark { .. } => Ok(()),
            Self::Connection { settings } => settings.validate(),
        }
    }

    fn normalized(mut self) -> Result<Self, String> {
        self.validate()?;
        if let Self::Local { path } = &mut self {
            *path = absolute_directory(path)?;
        }
        Ok(self)
    }

    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Local { .. })
    }

    pub fn needs_secret_prompt(&self) -> bool {
        !matches!(self, Self::Local { .. })
            && !matches!(self,
            Self::Connection { settings } if matches!(settings.protocol(),
                crate::filetransfer::FileTransferProtocol::GoogleCloudStorage | crate::filetransfer::FileTransferProtocol::Kube))
    }

    pub fn location(&self) -> String {
        match self {
            Self::Local { path } => path.display().to_string(),
            Self::Remote { url } => url.clone(),
            Self::Bookmark { name } => format!("bookmark:{name}"),
            Self::Connection { settings } => settings.description(),
        }
    }

    pub fn description(&self) -> String {
        if let Self::Connection { settings } = self {
            return settings.description();
        }
        format!(
            "{} · {}",
            if self.is_remote() { "Remote" } else { "Local" },
            self.location()
        )
    }
}

/// A comparison session, not a connection bookmark. There is no transfer view selector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub source: Endpoint,
    pub destination: Endpoint,
    #[serde(default)]
    pub ignores: Vec<String>,
}

impl Profile {
    pub fn new(
        name: String,
        source: &str,
        destination: &str,
        ignores: Vec<String>,
    ) -> Result<Self, String> {
        Self::from_endpoints(
            name,
            Endpoint::from_input(source)?,
            Endpoint::from_input(destination)?,
            ignores,
        )
    }

    pub fn from_endpoints(
        name: String,
        source: Endpoint,
        destination: Endpoint,
        ignores: Vec<String>,
    ) -> Result<Self, String> {
        let profile = Self {
            name,
            source: source
                .normalized()
                .map_err(|error| format!("Source: {error}"))?,
            destination: destination
                .normalized()
                .map_err(|error| format!("Destination: {error}"))?,
            ignores,
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn has_remote(&self) -> bool {
        self.source.is_remote() || self.destination.is_remote()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Enter a comparison name".into());
        }
        self.source
            .validate()
            .map_err(|error| format!("Source: {error}"))?;
        self.destination
            .validate()
            .map_err(|error| format!("Destination: {error}"))
    }
}

pub fn remote_url(address: &str) -> Result<Url, String> {
    let url = Url::parse(address).map_err(|error| error.to_string())?;
    if !matches!(
        url.scheme(),
        "sftp" | "scp" | "ftp" | "ftps" | "http" | "https"
    ) || url.host_str().is_none()
    {
        return Err("Use sftp://, scp://, ftp://, ftps:// or http(s):// with a hostname".into());
    }
    if url.password().is_some() {
        return Err(
            "Passwords must not appear in profiles or URLs; use a bookmark or --password-prompt"
                .into(),
        );
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Remote folder URLs cannot contain a query or fragment".into());
    }
    if matches!(url.scheme(), "http" | "https") {
        super::connection::http_url(address)?;
    }
    Ok(url)
}

pub fn absolute_directory(path: &Path) -> Result<PathBuf, String> {
    let expanded = shellexpand::tilde(&path.to_string_lossy()).into_owned();
    let path = fs::canonicalize(&expanded).map_err(|error| format!("{expanded}: {error}"))?;
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    Ok(path)
}

#[derive(Serialize, Deserialize)]
struct Document<T> {
    version: u32,
    profiles: BTreeMap<String, T>,
    #[serde(default)]
    folders: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct LegacyProfile {
    name: String,
    left: PathBuf,
    right: String,
    #[serde(default)]
    ignores: Vec<String>,
}

pub struct ProfileStore {
    path: PathBuf,
    pub profiles: BTreeMap<String, Profile>,
    pub folders: BTreeMap<String, Vec<String>>,
}

impl ProfileStore {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let mut folders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let profiles = if path.exists() {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).map_err(|error| error.to_string())?)
                    .map_err(|error| format!("Invalid comparisons file: {error}"))?;
            if let Some(saved) = value.get("folders") {
                folders =
                    serde_json::from_value(saved.clone()).map_err(|error| error.to_string())?;
            }
            match value.get("version").and_then(serde_json::Value::as_u64) {
                Some(1) => {
                    let document: Document<LegacyProfile> =
                        serde_json::from_value(value).map_err(|error| error.to_string())?;
                    document
                        .profiles
                        .into_iter()
                        .map(|(name, old)| {
                            // Loading is offline: do not canonicalize local roots or connect to servers.
                            Ok((
                                name,
                                Profile {
                                    name: old.name,
                                    source: Endpoint::Local { path: old.left },
                                    destination: Endpoint::from_input(&old.right)?,
                                    ignores: old.ignores,
                                },
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>, String>>()?
                }
                Some(2 | 3) => {
                    serde_json::from_value::<Document<Profile>>(value)
                        .map_err(|error| error.to_string())?
                        .profiles
                }
                _ => return Err("Unsupported comparison profile version".into()),
            }
        } else {
            BTreeMap::new()
        };
        for (name, profile) in &profiles {
            profile.validate()?;
            if name != &profile.name {
                return Err("Comparison name does not match storage key".into());
            }
        }
        let mut members = std::collections::BTreeSet::new();
        for (folder, names) in &folders {
            Self::validate_folder(folder)?;
            for name in names {
                if !profiles.contains_key(name) || !members.insert(name) {
                    return Err("Invalid comparison folder membership".into());
                }
            }
        }
        Ok(Self {
            path,
            profiles,
            folders,
        })
    }

    /// Save a session (also used when migrating version-1 data on the next write).
    pub fn save(&mut self, profile: Profile) -> Result<(), String> {
        profile.validate()?;
        let mut profiles = self.profiles.clone();
        profiles.insert(profile.name.clone(), profile);
        self.commit(profiles)
    }

    /// Updates the same session, including an atomic rename, without duplicate profiles.
    pub fn update(&mut self, original: &str, profile: Profile) -> Result<(), String> {
        profile.validate()?;
        if !self.profiles.contains_key(original) {
            return Err("The saved comparison no longer exists".into());
        }
        if profile.name != original && self.profiles.contains_key(&profile.name) {
            return Err("That comparison name already exists".into());
        }
        let mut profiles = self.profiles.clone();
        profiles.remove(original);
        profiles.insert(profile.name.clone(), profile.clone());
        let mut folders = self.folders.clone();
        for names in folders.values_mut() {
            for name in names {
                if name == original {
                    *name = profile.name.clone();
                }
            }
        }
        self.commit_with_folders(profiles, folders)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        let mut profiles = self.profiles.clone();
        profiles.remove(name);
        self.commit(profiles)
    }

    fn validate_folder(name: &str) -> Result<(), String> {
        if name.trim().is_empty()
            || name != name.trim()
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(
                "Enter a single-level folder name without slashes or surrounding spaces".into(),
            );
        }
        Ok(())
    }

    pub fn create_folder(&mut self, name: String) -> Result<(), String> {
        Self::validate_folder(&name)?;
        if self.folders.contains_key(&name) {
            return Err("That folder already exists".into());
        }
        let mut folders = self.folders.clone();
        folders.insert(name, Vec::new());
        self.commit_with_folders(self.profiles.clone(), folders)
    }

    /// Remove only the organization folder; its profiles return to the root list.
    pub fn delete_folder(&mut self, name: &str) -> Result<(), String> {
        let mut folders = self.folders.clone();
        if folders.remove(name).is_none() {
            return Err("That folder no longer exists".into());
        }
        self.commit_with_folders(self.profiles.clone(), folders)
    }

    pub fn folder_of(&self, name: &str) -> Option<&str> {
        self.folders
            .iter()
            .find(|(_, names)| names.iter().any(|member| member == name))
            .map(|(folder, _)| folder.as_str())
    }

    pub fn move_profile(&mut self, name: &str, folder: Option<&str>) -> Result<(), String> {
        if !self.profiles.contains_key(name) {
            return Err("The saved comparison no longer exists".into());
        }
        if let Some(folder) = folder
            && !self.folders.contains_key(folder)
        {
            return Err("Unknown folder; create it with f first".into());
        }
        let mut folders = self.folders.clone();
        for names in folders.values_mut() {
            names.retain(|member| member != name);
        }
        if let Some(folder) = folder {
            folders
                .get_mut(folder)
                .expect("validated folder")
                .push(name.to_string());
        }
        self.commit_with_folders(self.profiles.clone(), folders)
    }

    fn commit(&mut self, profiles: BTreeMap<String, Profile>) -> Result<(), String> {
        self.commit_with_folders(profiles, self.folders.clone())
    }

    fn commit_with_folders(
        &mut self,
        profiles: BTreeMap<String, Profile>,
        mut folders: BTreeMap<String, Vec<String>>,
    ) -> Result<(), String> {
        for names in folders.values_mut() {
            names.retain(|name| profiles.contains_key(name));
            names.sort();
        }
        let document = Document {
            version: 3,
            profiles,
            folders,
        };
        let bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        let parent = self.path.parent().ok_or("Invalid comparisons path")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temporary
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.to_string())?;
        self.profiles = document.profiles;
        self.folders = document.folders;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_version_one_without_touching_roots_and_preserves_all_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("comparisons.json");
        let original = serde_json::json!({ "version": 1, "profiles": {
            "existing": { "name": "existing", "left": "offline-local", "right": "sftp://user@host/path" },
            "other": { "name": "other", "left": "another-offline-folder", "right": "bookmark:server", "ignores": [".git"] }
        }});
        fs::write(&path, original.to_string()).unwrap();
        let mut store = ProfileStore::load(path.clone()).unwrap();
        assert_eq!(
            store.profiles["existing"].source,
            Endpoint::Local {
                path: "offline-local".into()
            }
        );
        assert_eq!(
            store.profiles["other"].destination,
            Endpoint::Bookmark {
                name: "server".into()
            }
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
            1
        );
        let profile = store.profiles["existing"].clone();
        store.save(profile.clone()).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 3);
        assert!(saved["profiles"]["existing"].get("left").is_none());
        assert_eq!(ProfileStore::load(path).unwrap().profiles, store.profiles);
        assert_eq!(store.profiles.len(), 2);
    }

    #[test]
    fn supports_all_endpoint_combinations_and_rejects_secrets_on_either_side() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().to_str().unwrap();
        for source in [local, "sftp://source/path", "bookmark:source"] {
            for destination in [local, "ftps://destination/path", "bookmark:destination"] {
                let profile = Profile::new("test".into(), source, destination, vec![]).unwrap();
                let json = serde_json::to_vec(&profile).unwrap();
                assert_eq!(serde_json::from_slice::<Profile>(&json).unwrap(), profile);
            }
        }
        for (source, destination) in [
            ("sftp://u:secret@host/path", local),
            (local, "ftp://u:secret@host/path"),
        ] {
            assert!(Profile::new("test".into(), source, destination, vec![]).is_err());
        }
        assert!(remote_url("unsupported://host/path").is_err());
    }

    #[test]
    fn updates_same_session_and_renames_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut store = ProfileStore::load(path.clone()).unwrap();
        let mut profile = Profile::new(
            "test".into(),
            "sftp://source/path",
            dir.path().to_str().unwrap(),
            vec![],
        )
        .unwrap();
        store.save(profile.clone()).unwrap();
        profile.source = Endpoint::Remote {
            url: "sftp://new-source/path".into(),
        };
        profile.ignores = vec!["*.tmp".into()];
        store.update("test", profile.clone()).unwrap();
        assert_eq!(store.profiles.len(), 1);
        assert_eq!(
            ProfileStore::load(path.clone()).unwrap().profiles["test"],
            profile
        );
        profile.name = "renamed".into();
        store.update("test", profile.clone()).unwrap();
        assert!(!store.profiles.contains_key("test"));
        assert_eq!(store.profiles.len(), 1);
        assert_eq!(
            ProfileStore::load(path.clone()).unwrap().profiles["renamed"],
            profile
        );
        store.delete("renamed").unwrap();
        assert!(ProfileStore::load(path).unwrap().profiles.is_empty());
    }

    #[test]
    fn duplicate_rename_and_write_failures_preserve_original_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut store = ProfileStore::load(path.clone()).unwrap();
        let first = Profile::new(
            "first".into(),
            "sftp://source/path",
            "sftp://destination/path",
            vec![],
        )
        .unwrap();
        store.save(first.clone()).unwrap();
        let mut second = first.clone();
        second.name = "second".into();
        store.save(second.clone()).unwrap();
        assert!(store.update("first", second).is_err());
        assert_eq!(store.profiles["first"], first);
        let old = fs::read(path).unwrap();
        store.path = dir.path().join("not-a-file");
        fs::create_dir(&store.path).unwrap();
        let mut changed = first.clone();
        changed.name = "changed".into();
        assert!(store.update("first", changed).is_err());
        assert_eq!(store.profiles["first"], first);
        assert_eq!(fs::read(dir.path().join("profiles.json")).unwrap(), old);
    }

    #[test]
    fn loads_version_two_offline_and_saves_all_protocols_as_version_three() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("profiles.json");
        fs::write(&path, serde_json::json!({"version": 2, "profiles": {"old": {"name": "old",
            "source": {"kind": "local", "path": "unavailable-folder"},
            "destination": {"kind": "remote", "url": "ftp://offline/folder"}, "ignores": [".git"] }}}).to_string()).unwrap();
        let mut store = ProfileStore::load(path.clone()).unwrap();
        for transfer in super::super::connection::tests::fixtures() {
            let name = transfer.protocol.to_string();
            let (settings, _) = ConnectionSettings::from_params(transfer).unwrap();
            let endpoint = Endpoint::Connection {
                settings: Box::new(settings),
            };
            store
                .save(Profile::from_endpoints(name, endpoint.clone(), endpoint, vec![]).unwrap())
                .unwrap();
        }
        assert_eq!(
            ProfileStore::load(path.clone()).unwrap().profiles,
            store.profiles
        );
        let contents = fs::read_to_string(path).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&contents).unwrap()["version"],
            3
        );
        assert_eq!(store.profiles["old"].ignores, vec![".git"]);
        for secret in [
            "password-value",
            "secret-key-value",
            "security-token-value",
            "session-token-value",
        ] {
            assert!(!contents.contains(secret));
        }
    }

    #[test]
    fn rejects_unknown_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        fs::write(&path, r#"{"version":99,"profiles":{}}"#).unwrap();
        assert!(ProfileStore::load(path).is_err());
    }
}
