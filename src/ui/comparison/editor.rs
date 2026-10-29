//! Independently editable endpoint panels using the inherited connection parameters.
use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::Side;
use crate::comparison::connection::{ConnectionSettings, Credentials, Secrets};
use crate::comparison::profiles::{Endpoint, Profile};
use crate::filetransfer::params::{
    AwsS3Params, GenericProtocolParams, GoogleCloudStorageParams, KubeProtocolParams, SmbDialect,
    SmbParams, WebDAVProtocolParams,
};
use crate::filetransfer::{FileTransferParams, FileTransferProtocol, ProtocolParams};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EndpointType {
    Local,
    Url,
    Bookmark,
    Protocol(FileTransferProtocol),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Field {
    Type,
    Folder,
    Location,
    Host,
    Port,
    Username,
    Password,
    Bucket,
    Region,
    HttpEndpoint,
    Profile,
    AccessKey,
    SecretKey,
    SecurityToken,
    SessionToken,
    PathStyle,
    ServiceAccount,
    Namespace,
    ClusterUrl,
    ClientCert,
    ClientKey,
    Uri,
    Share,
    Workgroup,
    Dialect,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Self::Type => "Type · Space/←→ select",
            Self::Folder => "Folder",
            Self::Location => "URL or connection bookmark name",
            Self::Host => "Host / SSH alias",
            Self::Port => "Port (blank = default / SSH config)",
            Self::Username => "Username",
            Self::Password => "Password · saved in OS keychain",
            Self::Bucket => "Bucket",
            Self::Region => "Region",
            Self::HttpEndpoint => "Endpoint URL",
            Self::Profile => "AWS credentials profile",
            Self::AccessKey => "Access key ID",
            Self::SecretKey => "Secret access key · saved in OS keychain",
            Self::SecurityToken => "Security token · session only",
            Self::SessionToken => "Session token · session only",
            Self::PathStyle => "Path-style addressing · Space/←→",
            Self::ServiceAccount => "Service-account JSON file (blank = ADC)",
            Self::Namespace => "Pod namespace (optional)",
            Self::ClusterUrl => "Cluster URL (blank = kube config)",
            Self::ClientCert => "Client certificate file",
            Self::ClientKey => "Client key file",
            Self::Uri => "WebDAV HTTP(S) URI",
            Self::Share => "Share",
            Self::Workgroup => "Workgroup",
            Self::Dialect => "SMB version · Space/←→; SMB1 is insecure",
        }
    }
    fn secret(self) -> bool {
        matches!(
            self,
            Self::Password | Self::SecretKey | Self::SecurityToken | Self::SessionToken
        )
    }
}

pub(super) struct EndpointInput {
    pub kind: EndpointType,
    pub location: String,
    pub fields: BTreeMap<Field, String>,
}

impl Default for EndpointInput {
    fn default() -> Self {
        Self {
            kind: EndpointType::Local,
            location: String::new(),
            fields: BTreeMap::new(),
        }
    }
}

impl EndpointInput {
    pub fn choices() -> Vec<EndpointType> {
        use FileTransferProtocol::*;
        let mut choices = vec![EndpointType::Local];
        choices.extend(
            [
                Sftp,
                Scp,
                Ftp(false),
                Ftp(true),
                AwsS3,
                GoogleCloudStorage,
                Kube,
                WebDAV,
            ]
            .map(EndpointType::Protocol),
        );
        if cfg!(smb) {
            choices.push(EndpointType::Protocol(Smb));
        }
        choices.extend([EndpointType::Url, EndpointType::Bookmark]);
        choices
    }

    fn from_endpoint(endpoint: &Endpoint) -> Self {
        match endpoint {
            Endpoint::Local { path } => Self {
                location: path.display().to_string(),
                ..Self::default()
            },
            Endpoint::Bookmark { name } => Self {
                kind: EndpointType::Bookmark,
                location: name.clone(),
                ..Self::default()
            },
            Endpoint::Remote { url } => match ConnectionSettings::from_url(url) {
                Ok(settings) => Self::from_settings(&settings),
                Err(_) => Self {
                    kind: EndpointType::Url,
                    location: url.clone(),
                    ..Self::default()
                },
            },
            Endpoint::Connection { settings } => Self::from_settings(settings),
        }
    }

    fn from_settings(settings: &ConnectionSettings) -> Self {
        let mut input = Self {
            kind: EndpointType::Protocol(settings.protocol()),
            location: settings.root().display().to_string(),
            ..Self::default()
        };
        let b = settings.bookmark();
        let mut put = |id, value: Option<&str>| {
            if let Some(value) = value {
                input.fields.insert(id, value.into());
            }
        };
        put(Field::Host, b.address.as_deref());
        put(Field::Uri, b.address.as_deref());
        put(Field::Username, b.username.as_deref());
        if let Some(port) = b.port {
            input.fields.insert(Field::Port, port.to_string());
        }
        if let Some(s3) = &b.s3 {
            input.fields.insert(Field::Bucket, s3.bucket.clone());
            for (id, value) in [
                (Field::Region, &s3.region),
                (Field::HttpEndpoint, &s3.endpoint),
                (Field::Profile, &s3.profile),
                (Field::AccessKey, &s3.access_key),
            ] {
                if let Some(value) = value {
                    input.fields.insert(id, value.clone());
                }
            }
            input.fields.insert(
                Field::PathStyle,
                if s3.new_path_style.unwrap_or(false) {
                    "Yes"
                } else {
                    "No"
                }
                .into(),
            );
        }
        if let Some(gcs) = &b.gcs {
            input.fields.insert(Field::Bucket, gcs.bucket.clone());
            input
                .fields
                .insert(Field::HttpEndpoint, gcs.endpoint.clone());
            if let Some(path) = &gcs.service_account_key {
                input.fields.insert(Field::ServiceAccount, path.clone());
            }
        }
        if let Some(kube) = &b.kube {
            for (id, value) in [
                (Field::Namespace, &kube.namespace),
                (Field::ClusterUrl, &kube.cluster_url),
                (Field::Username, &kube.username),
                (Field::ClientCert, &kube.client_cert),
                (Field::ClientKey, &kube.client_key),
            ] {
                if let Some(value) = value {
                    input.fields.insert(id, value.clone());
                }
            }
        }
        if let Some(smb) = &b.smb {
            input.fields.insert(Field::Share, smb.share.clone());
            if let Some(workgroup) = &smb.workgroup {
                input.fields.insert(Field::Workgroup, workgroup.clone());
            }
            input.fields.insert(
                Field::Dialect,
                match smb.dialect.unwrap_or_default() {
                    SmbDialect::Auto => "Auto",
                    SmbDialect::Smb1 => "SMB1 (insecure)",
                    SmbDialect::Smb2 => "SMB2",
                    SmbDialect::Smb3 => "SMB3",
                }
                .into(),
            );
        }
        input
    }

    pub fn field_ids(&self) -> Vec<Field> {
        use Field::*;
        use FileTransferProtocol::*;
        let mut fields = vec![Type];
        fields.extend(match self.kind {
            EndpointType::Local => vec![Folder],
            EndpointType::Url | EndpointType::Bookmark => vec![Location, Password],
            EndpointType::Protocol(Sftp | Scp | Ftp(_)) => {
                vec![Host, Port, Username, Password, Folder]
            }
            EndpointType::Protocol(AwsS3) => vec![
                Bucket,
                Region,
                HttpEndpoint,
                Profile,
                AccessKey,
                SecretKey,
                SecurityToken,
                SessionToken,
                PathStyle,
                Folder,
            ],
            EndpointType::Protocol(GoogleCloudStorage) => {
                vec![Bucket, HttpEndpoint, ServiceAccount, Folder]
            }
            EndpointType::Protocol(Kube) => vec![
                Namespace, ClusterUrl, Username, ClientCert, ClientKey, Folder,
            ],
            EndpointType::Protocol(WebDAV) => vec![Uri, Username, Password, Folder],
            EndpointType::Protocol(Smb) => {
                let mut fields = vec![Host];
                if cfg!(posix) {
                    fields.push(Port);
                }
                fields.extend([Username, Password, Share]);
                if cfg!(posix) {
                    fields.extend([Workgroup, Dialect]);
                }
                fields.push(Folder);
                fields
            }
        });
        fields
    }

    fn text(&self, field: Field) -> String {
        self.fields.get(&field).cloned().unwrap_or_default()
    }
    fn optional(&self, field: Field) -> Option<String> {
        self.fields
            .get(&field)
            .filter(|value| !value.is_empty())
            .cloned()
    }

    pub fn value(&self, field: Field) -> String {
        let text = match field {
            Field::Type => match self.kind {
                EndpointType::Local => "< Local >".into(),
                EndpointType::Url => "< URL shortcut >".into(),
                EndpointType::Bookmark => "< Connection bookmark >".into(),
                EndpointType::Protocol(protocol) => format!(
                    "< {protocol} >{}",
                    if protocol == FileTransferProtocol::Smb && !cfg!(smb) {
                        " · unavailable in this build"
                    } else {
                        ""
                    }
                ),
            },
            Field::Location | Field::Folder => self.location.clone(),
            Field::PathStyle => self.optional(field).unwrap_or_else(|| "No".into()),
            Field::Dialect => self.optional(field).unwrap_or_else(|| "Auto".into()),
            _ => self.text(field),
        };
        if field.secret() {
            "•".repeat(text.chars().count())
        } else {
            text
        }
    }

    pub fn rows(&self) -> Vec<(&'static str, String)> {
        self.field_ids()
            .into_iter()
            .map(|field| (field.label(), self.value(field)))
            .collect()
    }

    fn change_type(&mut self, kind: EndpointType) {
        if kind != self.kind {
            // Never carry a password/token to a different protocol or connection type.
            for field in [
                Field::Password,
                Field::SecretKey,
                Field::SecurityToken,
                Field::SessionToken,
            ] {
                self.fields.remove(&field);
            }
            self.fields.remove(&Field::Port);
            self.kind = kind;
            if matches!(kind, EndpointType::Protocol(_)) && self.location.is_empty() {
                self.location = "/".into();
            }
        }
    }

    fn key(&mut self, field: Field, key: KeyEvent) {
        let rotate = matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
        );
        if field == Field::Type {
            if rotate {
                let choices = Self::choices();
                let index = choices
                    .iter()
                    .position(|kind| kind == &self.kind)
                    .unwrap_or(0);
                let next = if key.code == KeyCode::Left {
                    (index + choices.len() - 1) % choices.len()
                } else {
                    (index + 1) % choices.len()
                };
                self.change_type(choices[next]);
            } else {
                match key.code {
                    KeyCode::Char('l') => self.change_type(EndpointType::Local),
                    KeyCode::Char('r') => {
                        self.change_type(EndpointType::Protocol(FileTransferProtocol::Sftp))
                    }
                    KeyCode::Char('u') => self.change_type(EndpointType::Url),
                    KeyCode::Char('b') => self.change_type(EndpointType::Bookmark),
                    _ => {}
                }
            }
            return;
        }
        if matches!(field, Field::PathStyle | Field::Dialect) {
            if rotate {
                let choices = if field == Field::PathStyle {
                    vec!["No", "Yes"]
                } else {
                    vec!["Auto", "SMB1 (insecure)", "SMB2", "SMB3"]
                };
                let current = self.value(field);
                let index = choices
                    .iter()
                    .position(|value| *value == current)
                    .unwrap_or(0);
                let next = if key.code == KeyCode::Left {
                    (index + choices.len() - 1) % choices.len()
                } else {
                    (index + 1) % choices.len()
                };
                self.fields.insert(field, choices[next].into());
            }
            return;
        }
        let text = if matches!(field, Field::Folder | Field::Location) {
            &mut self.location
        } else {
            self.fields.entry(field).or_default()
        };
        edit_text(text, key);
    }

    fn connection(
        &self,
        protocol: FileTransferProtocol,
    ) -> Result<(ConnectionSettings, Secrets), String> {
        use Field::*;
        let port = self
            .optional(Port)
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| "Port must be between 1 and 65535".to_string())
            })
            .transpose()?;
        if port == Some(0) {
            return Err("Port must be between 1 and 65535".into());
        }
        let params = match protocol {
            FileTransferProtocol::Sftp
            | FileTransferProtocol::Scp
            | FileTransferProtocol::Ftp(_) => ProtocolParams::Generic(
                GenericProtocolParams::default()
                    .address(self.text(Host))
                    .port(
                        port.unwrap_or(if matches!(protocol, FileTransferProtocol::Ftp(_)) {
                            21
                        } else {
                            22
                        }),
                    )
                    .username(self.optional(Username))
                    .password(self.optional(Password)),
            ),
            FileTransferProtocol::AwsS3 => ProtocolParams::AwsS3(
                AwsS3Params::new(
                    self.text(Bucket),
                    self.optional(Region),
                    self.optional(Profile),
                )
                .endpoint(self.optional(HttpEndpoint))
                .access_key(self.optional(AccessKey))
                .secret_access_key(self.optional(SecretKey))
                .security_token(self.optional(SecurityToken))
                .session_token(self.optional(SessionToken))
                .new_path_style(self.value(PathStyle) == "Yes"),
            ),
            FileTransferProtocol::GoogleCloudStorage => ProtocolParams::GoogleCloudStorage(
                GoogleCloudStorageParams::new(self.text(Bucket))
                    .endpoint(self.optional(HttpEndpoint).unwrap_or_else(|| {
                        crate::filetransfer::params::DEFAULT_GCS_ENDPOINT.into()
                    }))
                    .service_account_key(self.optional(ServiceAccount)),
            ),
            FileTransferProtocol::Kube => ProtocolParams::Kube(KubeProtocolParams {
                namespace: self.optional(Namespace),
                cluster_url: self.optional(ClusterUrl),
                username: self.optional(Username),
                client_cert: self.optional(ClientCert),
                client_key: self.optional(ClientKey),
            }),
            FileTransferProtocol::WebDAV => ProtocolParams::WebDAV(WebDAVProtocolParams {
                uri: self.text(Uri),
                username: self.text(Username),
                password: self.text(Password),
            }),
            FileTransferProtocol::Smb => {
                let params = SmbParams::new(self.text(Host), self.text(Share))
                    .username(self.optional(Username))
                    .password(self.optional(Password))
                    .dialect(match self.value(Dialect).as_str() {
                        "SMB1 (insecure)" => SmbDialect::Smb1,
                        "SMB2" => SmbDialect::Smb2,
                        "SMB3" => SmbDialect::Smb3,
                        _ => SmbDialect::Auto,
                    });
                #[cfg(posix)]
                let params = params
                    .port(port.unwrap_or(445))
                    .workgroup(self.optional(Workgroup));
                ProtocolParams::Smb(params)
            }
        };
        let (settings, secrets) = ConnectionSettings::from_params(
            FileTransferParams::new(protocol, params).remote_path(Some(&self.location)),
        )?;
        let mut bookmark = settings.bookmark().clone();
        if matches!(
            protocol,
            FileTransferProtocol::Sftp | FileTransferProtocol::Scp | FileTransferProtocol::Ftp(_)
        ) {
            bookmark.port = port;
        }
        if protocol == FileTransferProtocol::Smb {
            // Retain POSIX-only saved settings even when editing on Windows.
            bookmark.port = port;
            if let Some(smb) = &mut bookmark.smb {
                smb.workgroup = self.optional(Workgroup);
            }
        }
        Ok((ConnectionSettings::try_from(bookmark)?, secrets))
    }

    fn endpoint(&self) -> Result<Endpoint, String> {
        match self.kind {
            EndpointType::Local => {
                let endpoint = Endpoint::Local {
                    path: self.location.clone().into(),
                };
                endpoint.validate()?;
                Ok(endpoint)
            }
            EndpointType::Bookmark => {
                let endpoint = Endpoint::Bookmark {
                    name: self
                        .location
                        .strip_prefix("bookmark:")
                        .unwrap_or(&self.location)
                        .into(),
                };
                endpoint.validate()?;
                Ok(endpoint)
            }
            EndpointType::Url => {
                let endpoint = Endpoint::from_input(&self.location)?;
                if matches!(endpoint, Endpoint::Remote { .. }) {
                    Ok(endpoint)
                } else {
                    Err(
                        "Enter a remote folder URL; choose Local or Bookmark for other locations"
                            .into(),
                    )
                }
            }
            EndpointType::Protocol(protocol) => {
                self.connection(protocol)
                    .map(|(settings, _)| Endpoint::Connection {
                        settings: Box::new(settings),
                    })
            }
        }
    }

    fn secrets(&self) -> Secrets {
        if let EndpointType::Protocol(FileTransferProtocol::AwsS3) = self.kind {
            Secrets {
                secret_access_key: self.optional(Field::SecretKey),
                security_token: self.optional(Field::SecurityToken),
                session_token: self.optional(Field::SessionToken),
                ..Secrets::default()
            }
        } else {
            Secrets {
                password: self.optional(Field::Password),
                ..Secrets::default()
            }
        }
    }
}

fn edit_text(text: &mut String, key: KeyEvent) {
    match key.code {
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            text.push(character)
        }
        _ => {}
    }
}

pub(super) struct Editor {
    pub name: String,
    pub source: EndpointInput,
    pub destination: EndpointInput,
    pub ignores: String,
    pub field: usize,
    pub original: Option<String>,
}

impl Editor {
    pub fn new(profile: Option<&Profile>, original: Option<String>) -> Self {
        Self {
            name: profile
                .map(|profile| profile.name.clone())
                .unwrap_or_default(),
            source: profile
                .map(|profile| EndpointInput::from_endpoint(&profile.source))
                .unwrap_or_default(),
            destination: profile
                .map(|profile| EndpointInput::from_endpoint(&profile.destination))
                .unwrap_or_default(),
            ignores: profile
                .map(|profile| profile.ignores.join("; "))
                .unwrap_or_default(),
            field: 0,
            original,
        }
    }

    pub fn ignore_index(&self) -> usize {
        1 + self.source.field_ids().len() + self.destination.field_ids().len()
    }

    pub fn selected(&self, side: Side) -> Option<usize> {
        let start = if side == Side::Source {
            1
        } else {
            1 + self.source.field_ids().len()
        };
        let length = if side == Side::Source {
            self.source.field_ids().len()
        } else {
            self.destination.field_ids().len()
        };
        if self.field >= start && self.field < start + length {
            Some(self.field - start)
        } else {
            None
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        let count = self.ignore_index() + 1;
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.field = (self.field + 1) % count,
            KeyCode::BackTab | KeyCode::Up => self.field = (self.field + count - 1) % count,
            KeyCode::F(1) => self.field = 0,
            KeyCode::F(2) => self.field = 1,
            KeyCode::F(3) => self.field = 1 + self.source.field_ids().len(),
            _ => {
                if self.field == 0 {
                    edit_text(&mut self.name, key);
                } else if self.field == self.ignore_index() {
                    edit_text(&mut self.ignores, key);
                } else if let Some(index) = self.selected(Side::Source) {
                    self.source.key(self.source.field_ids()[index], key);
                } else if let Some(index) = self.selected(Side::Destination) {
                    self.destination
                        .key(self.destination.field_ids()[index], key);
                }
            }
        }
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            source: self.source.secrets(),
            destination: self.destination.secrets(),
        }
    }

    pub fn profile(&self) -> Result<Profile, String> {
        let source = self
            .source
            .endpoint()
            .map_err(|error| format!("Source: {error}"))?;
        let destination = self
            .destination
            .endpoint()
            .map_err(|error| format!("Destination: {error}"))?;
        let ignores = self
            .ignores
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Profile::from_endpoints(self.name.trim().into(), source, destination, ignores)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(editor: &mut Editor, code: KeyCode) {
        editor.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn independently_selects_and_configures_remote_protocols_without_saving_secrets() {
        let mut editor = Editor::new(None, None);
        editor.name = "servers".into();
        press(&mut editor, KeyCode::F(2));
        press(&mut editor, KeyCode::Char('r'));
        editor.source.fields.insert(Field::Host, "source".into());
        editor
            .source
            .fields
            .insert(Field::Password, "source-secret".into());
        press(&mut editor, KeyCode::F(3));
        press(&mut editor, KeyCode::Char('r'));
        press(&mut editor, KeyCode::Right); // SCP is independent from source SFTP.
        editor
            .destination
            .fields
            .insert(Field::Host, "destination".into());
        editor
            .destination
            .fields
            .insert(Field::Password, "destination-secret".into());
        let profile = editor.profile().unwrap();
        let Endpoint::Connection { settings } = &profile.source else {
            panic!("source settings");
        };
        assert_eq!(settings.protocol(), FileTransferProtocol::Sftp);
        let Endpoint::Connection { settings } = &profile.destination else {
            panic!("destination settings");
        };
        assert_eq!(settings.protocol(), FileTransferProtocol::Scp);
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("source-secret") && !json.contains("destination-secret"));
        assert_eq!(
            editor.credentials().source.password.as_deref(),
            Some("source-secret")
        );
        assert_eq!(
            editor.credentials().destination.password.as_deref(),
            Some("destination-secret")
        );
        let rendered = editor
            .source
            .rows()
            .into_iter()
            .map(|(_, value)| value)
            .collect::<String>();
        assert!(!rendered.contains("source-secret"));
        let reopened = Editor::new(Some(&profile), Some(profile.name.clone()));
        assert!(reopened.credentials().source.password.is_none());
        assert_eq!(reopened.profile().unwrap(), profile);
    }

    #[test]
    fn new_local_comparison_normalizes_both_folders() {
        let folder = tempfile::tempdir().unwrap();
        let mut editor = Editor::new(None, None);
        editor.name = "local".into();
        editor.source.location = folder.path().display().to_string();
        editor.destination.location = editor.source.location.clone();
        let profile = editor.profile().unwrap();
        assert!(!profile.has_remote());
        assert_eq!(profile.source, profile.destination);
    }

    #[test]
    fn every_protocol_editor_preserves_options_and_exposes_matching_fields() {
        for transfer in crate::comparison::connection::tests::fixtures() {
            let (settings, _) = ConnectionSettings::from_params(transfer).unwrap();
            let endpoint = Endpoint::Connection {
                settings: Box::new(settings),
            };
            let profile =
                Profile::from_endpoints("test".into(), endpoint.clone(), endpoint, vec![]).unwrap();
            let editor = Editor::new(Some(&profile), Some("test".into()));
            assert_eq!(editor.profile().unwrap(), profile);
            let rows = editor.source.rows();
            assert_eq!(rows.len(), editor.source.field_ids().len());
            assert!(rows.iter().any(|(label, _)| *label == "Folder"));
            assert!(editor.credentials().source.password.is_none());
            assert!(editor.credentials().source.secret_access_key.is_none());
        }
        let profile = Profile::new(
            "URL".into(),
            "sftp://user@alias/folder",
            "ftps://user@other/folder",
            vec![],
        )
        .unwrap();
        let editor = Editor::new(Some(&profile), Some(profile.name.clone()));
        assert!(editor.source.field_ids().contains(&Field::Host));
        assert_eq!(editor.source.text(Field::Username), "user");
        assert_eq!(editor.source.text(Field::Port), "");
    }

    #[test]
    fn keyboard_navigation_reaches_every_field_and_switching_protocol_drops_secrets() {
        let mut editor = Editor::new(None, None);
        for choice in EndpointInput::choices() {
            editor.source.change_type(choice);
            let count = editor.ignore_index() + 1;
            editor.field = 0;
            for expected in 0..count {
                assert_eq!(editor.field, expected);
                press(&mut editor, KeyCode::Tab);
            }
            assert_eq!(editor.field, 0);
        }
        editor.source.kind = EndpointType::Protocol(FileTransferProtocol::AwsS3);
        editor
            .source
            .fields
            .insert(Field::SecretKey, "secret".into());
        editor.field = 1;
        press(&mut editor, KeyCode::Right);
        assert!(!editor.source.fields.contains_key(&Field::SecretKey));
    }
}
