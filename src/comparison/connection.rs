//! Reuse termscp's connection schema/conversions, but never persist credential values.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

#[cfg(test)]
#[path = "connection_tests.rs"]
pub(crate) mod tests;

use crate::config::bookmarks::Bookmark;
use crate::filetransfer::{FileTransferParams, FileTransferProtocol, ProtocolParams};

/// Session-only credentials; deliberately not Debug or Serialize.
#[derive(Clone, Default)]
pub struct Secrets {
    pub password: Option<String>,
    pub secret_access_key: Option<String>,
    pub security_token: Option<String>,
    pub session_token: Option<String>,
}

impl Secrets {
    pub fn apply(self, params: &mut ProtocolParams) {
        if let ProtocolParams::AwsS3(params) = params {
            if self.secret_access_key.is_some() {
                params.secret_access_key = self.secret_access_key;
            }
            if self.security_token.is_some() {
                params.security_token = self.security_token;
            }
            if self.session_token.is_some() {
                params.session_token = self.session_token;
            }
        }
        if let Some(secret) = self.password.filter(|value| !value.is_empty()) {
            if let ProtocolParams::Smb(smb) = params {
                smb.password = Some(secret);
            } else {
                params.set_default_secret(secret);
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Credentials {
    pub source: Secrets,
    pub destination: Secrets,
}

/// The private inner bookmark is guaranteed to contain no password or secret key.
/// Keeping the inherited schema means every protocol uses the same parameter mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Bookmark", into = "Bookmark")]
pub struct ConnectionSettings(Bookmark);

impl TryFrom<Bookmark> for ConnectionSettings {
    type Error = String;

    fn try_from(bookmark: Bookmark) -> Result<Self, String> {
        if bookmark.password.is_some()
            || bookmark
                .s3
                .as_ref()
                .is_some_and(|s3| s3.secret_access_key.is_some())
        {
            return Err(
                "Comparison settings cannot contain passwords or secret access keys".into(),
            );
        }
        let settings = Self(bookmark);
        settings.validate()?;
        Ok(settings)
    }
}

impl From<ConnectionSettings> for Bookmark {
    fn from(settings: ConnectionSettings) -> Self {
        settings.0
    }
}

impl ConnectionSettings {
    pub fn from_params(mut transfer: FileTransferParams) -> Result<(Self, Secrets), String> {
        let mut secrets = Secrets::default();
        match &mut transfer.params {
            ProtocolParams::Generic(params) => secrets.password = params.password.take(),
            ProtocolParams::AwsS3(params) => {
                secrets.secret_access_key = params.secret_access_key.take();
                secrets.security_token = params.security_token.take();
                secrets.session_token = params.session_token.take();
            }
            ProtocolParams::Smb(params) => secrets.password = params.password.take(),
            ProtocolParams::WebDAV(params) => {
                secrets.password = Some(std::mem::take(&mut params.password));
            }
            ProtocolParams::GoogleCloudStorage(_) | ProtocolParams::Kube(_) => {}
        }
        transfer.local_path = None;
        // WebDAV uses a required runtime String password; represent absence as None in storage.
        let mut bookmark = Bookmark::from(transfer);
        bookmark.password = None;
        Ok((Self::try_from(bookmark)?, secrets))
    }

    pub fn from_url(value: &str) -> Result<Self, String> {
        let url = super::profiles::remote_url(value)?;
        let decode = |value: &str| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8()
                .map(std::borrow::Cow::into_owned)
                .map_err(|error| error.to_string())
        };
        if matches!(url.scheme(), "http" | "https") {
            http_url(value)?;
            return Self::from_params(
                FileTransferParams::new(
                    FileTransferProtocol::WebDAV,
                    ProtocolParams::WebDAV(crate::filetransfer::params::WebDAVProtocolParams {
                        uri: value.into(),
                        username: String::new(),
                        password: String::new(),
                    }),
                )
                .remote_path(Some("/")),
            )
            .map(|(settings, _)| settings);
        }
        let protocol = match url.scheme() {
            "sftp" => FileTransferProtocol::Sftp,
            "scp" => FileTransferProtocol::Scp,
            "ftp" => FileTransferProtocol::Ftp(false),
            "ftps" => FileTransferProtocol::Ftp(true),
            _ => return Err("Unsupported folder URL".into()),
        };
        let username = if url.username().is_empty() {
            None
        } else {
            Some(decode(url.username())?)
        };
        let root = decode(url.path())?;
        let params = crate::filetransfer::params::GenericProtocolParams::default()
            .address(
                url.host_str()
                    .ok_or("Missing hostname")?
                    .trim_start_matches('[')
                    .trim_end_matches(']'),
            )
            .port(
                url.port()
                    .unwrap_or(if matches!(protocol, FileTransferProtocol::Ftp(_)) {
                        21
                    } else {
                        22
                    }),
            )
            .username(username);
        let (mut settings, _) = Self::from_params(
            FileTransferParams::new(protocol, ProtocolParams::Generic(params))
                .remote_path(Some(if root.is_empty() { "/" } else { &root })),
        )?;
        settings.0.port = url.port(); // None keeps SSH-config/default-port resolution.
        Ok(settings)
    }

    pub fn bookmark(&self) -> &Bookmark {
        &self.0
    }

    pub fn protocol(&self) -> FileTransferProtocol {
        self.0.protocol
    }

    pub fn transfer_params(&self) -> FileTransferParams {
        let mut params = FileTransferParams::from(self.0.clone());
        if let ProtocolParams::Generic(generic) = &mut params.params {
            generic.port =
                self.0
                    .port
                    .unwrap_or(if matches!(params.protocol, FileTransferProtocol::Ftp(_)) {
                        21
                    } else {
                        22
                    });
        }
        if let ProtocolParams::GoogleCloudStorage(gcs) = &mut params.params
            && gcs.endpoint.is_empty()
        {
            gcs.endpoint = crate::filetransfer::params::DEFAULT_GCS_ENDPOINT.into();
        }
        params
    }

    pub fn root(&self) -> PathBuf {
        self.0.remote_path.clone().unwrap_or_else(|| "/".into())
    }

    pub fn validate(&self) -> Result<(), String> {
        let b = &self.0;
        let required = |value: Option<&str>, label: &str| {
            if value.is_none_or(|value| value.trim().is_empty()) {
                Err(format!("Enter {label}"))
            } else {
                Ok(())
            }
        };
        let optional_http = |value: Option<&str>| {
            value
                .filter(|value| !value.is_empty())
                .map(http_url)
                .transpose()
                .map(|_| ())
        };
        match b.protocol {
            FileTransferProtocol::Sftp
            | FileTransferProtocol::Scp
            | FileTransferProtocol::Ftp(_) => {
                required(b.address.as_deref(), "a hostname")?;
                if b.address.as_ref().is_some_and(|host| {
                    host.contains("://")
                        || host.contains('@')
                        || host.contains('/')
                        || host.contains(char::is_whitespace)
                }) {
                    return Err("Host must be a hostname/IP or SSH alias, not a URL".into());
                }
                if b.port == Some(0) {
                    return Err("Port must be between 1 and 65535".into());
                }
            }
            FileTransferProtocol::AwsS3 => {
                let s3 = b.s3.as_ref().ok_or("Missing S3 settings")?;
                required(Some(&s3.bucket), "an S3 bucket")?;
                optional_http(s3.endpoint.as_deref())?;
            }
            FileTransferProtocol::GoogleCloudStorage => {
                let gcs = b.gcs.as_ref().ok_or("Missing GCS settings")?;
                required(Some(&gcs.bucket), "a GCS bucket")?;
                optional_http(Some(&gcs.endpoint))?;
            }
            FileTransferProtocol::Kube => {
                let kube = b.kube.as_ref().ok_or("Missing Kubernetes settings")?;
                optional_http(kube.cluster_url.as_deref())?;
            }
            FileTransferProtocol::Smb => {
                required(b.address.as_deref(), "an SMB hostname")?;
                required(b.smb.as_ref().map(|smb| smb.share.as_str()), "an SMB share")?;
                if b.port == Some(0) {
                    return Err("Port must be between 1 and 65535".into());
                }
            }
            FileTransferProtocol::WebDAV => {
                http_url(b.address.as_deref().ok_or("Enter a WebDAV URI")?)?;
            }
        }
        if b.remote_path
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("Enter a folder (use / for the remote root)".into());
        }
        Ok(())
    }

    /// Human-readable name for native credential permission dialogs.
    pub fn credential_label(&self) -> String {
        let description = self.description();
        let label = match self.0.username.as_deref().filter(|user| !user.is_empty()) {
            Some(user) => format!("{user} · {description}"),
            None => description,
        };
        label.chars().filter(|ch| !ch.is_control()).collect()
    }

    pub fn description(&self) -> String {
        let b = &self.0;
        let address = match b.protocol {
            FileTransferProtocol::AwsS3 => b.s3.as_ref().map(|s3| s3.bucket.as_str()),
            FileTransferProtocol::GoogleCloudStorage => {
                b.gcs.as_ref().map(|gcs| gcs.bucket.as_str())
            }
            FileTransferProtocol::Kube => b
                .kube
                .as_ref()
                .and_then(|kube| kube.namespace.as_deref())
                .or(Some("default")),
            _ => b.address.as_deref(),
        }
        .unwrap_or_default();
        let share = b
            .smb
            .as_ref()
            .map(|smb| format!("/{}", smb.share))
            .unwrap_or_default();
        format!(
            "{} · {address}{share} · {}",
            b.protocol,
            self.root().display()
        )
    }
}

/// For server URLs, never persist inline credentials, signed queries or fragments.
pub fn http_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Use an http:// or https:// server URL".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Server URLs cannot contain credentials, queries or fragments; use separate authentication fields".into());
    }
    Ok(url)
}
