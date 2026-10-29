use super::*;
use crate::filetransfer::params::{
    AwsS3Params, GenericProtocolParams, GoogleCloudStorageParams, KubeProtocolParams, SmbDialect,
    SmbParams, WebDAVProtocolParams,
};

pub(crate) fn fixtures() -> Vec<FileTransferParams> {
    use FileTransferProtocol::*;
    let mut transfers: Vec<_> = [Sftp, Scp, Ftp(false), Ftp(true)]
        .into_iter()
        .map(|protocol| {
            FileTransferParams::new(
                protocol,
                ProtocolParams::Generic(
                    GenericProtocolParams::default()
                        .address("host")
                        .port(2222)
                        .username(Some("user"))
                        .password(Some("password-value")),
                ),
            )
        })
        .collect();
    transfers.extend([
        FileTransferParams::new(
            AwsS3,
            ProtocolParams::AwsS3(
                AwsS3Params::new("bucket", Some("region"), Some("profile"))
                    .endpoint(Some("https://s3.example"))
                    .access_key(Some("access-id"))
                    .secret_access_key(Some("secret-key-value"))
                    .security_token(Some("security-token-value"))
                    .session_token(Some("session-token-value"))
                    .new_path_style(true),
            ),
        ),
        FileTransferParams::new(
            GoogleCloudStorage,
            ProtocolParams::GoogleCloudStorage(
                GoogleCloudStorageParams::new("bucket")
                    .endpoint("http://localhost:4443")
                    .service_account_key(Some("credentials.json")),
            ),
        ),
        FileTransferParams::new(
            Kube,
            ProtocolParams::Kube(KubeProtocolParams {
                namespace: Some("namespace".into()),
                cluster_url: Some("https://kube.example".into()),
                username: Some("user".into()),
                client_cert: Some("cert.crt".into()),
                client_key: Some("key.pem".into()),
            }),
        ),
        FileTransferParams::new(
            WebDAV,
            ProtocolParams::WebDAV(WebDAVProtocolParams {
                uri: "https://dav.example/root".into(),
                username: "user".into(),
                password: "password-value".into(),
            }),
        ),
    ]);
    let smb = SmbParams::new("server", "share")
        .username(Some("user"))
        .password(Some("password-value"))
        .dialect(SmbDialect::Smb3);
    #[cfg(posix)]
    let smb = smb.port(1445).workgroup(Some("workgroup"));
    transfers.push(FileTransferParams::new(Smb, ProtocolParams::Smb(smb)));
    transfers
        .into_iter()
        .map(|transfer| transfer.remote_path(Some("/folder")))
        .collect()
}

#[test]
fn every_protocol_round_trips_all_options_with_credentials_kept_separate() {
    for original in fixtures() {
        let expected = Bookmark::from(original.clone());
        let (settings, secrets) = ConnectionSettings::from_params(original).unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        for secret in [
            "password-value",
            "secret-key-value",
            "security-token-value",
            "session-token-value",
        ] {
            assert!(!json.contains(secret));
        }
        let restored: ConnectionSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, settings);
        let mut transfer = restored.transfer_params();
        assert_eq!(
            transfer.remote_path.as_deref(),
            Some(std::path::Path::new("/folder"))
        );
        assert_eq!(transfer.protocol, expected.protocol);
        secrets.apply(&mut transfer.params);
        if let ProtocolParams::AwsS3(s3) = &transfer.params {
            assert_eq!(s3.security_token.as_deref(), Some("security-token-value"));
            assert_eq!(s3.session_token.as_deref(), Some("session-token-value"));
        }
        assert_eq!(Bookmark::from(transfer), expected);
    }
}

#[test]
fn rejects_inline_secrets_and_invalid_servers_without_disclosing_values() {
    let (settings, _) = ConnectionSettings::from_params(fixtures().remove(0)).unwrap();
    let mut json = serde_json::to_value(&settings).unwrap();
    json["password"] = "do-not-disclose".into();
    let error = serde_json::from_value::<ConnectionSettings>(json)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("do-not-disclose"));
    for uri in [
        "https://u:secret@host/",
        "https://host/?token=secret",
        "file:///tmp",
        "https://host/#secret",
    ] {
        assert!(http_url(uri).is_err());
    }
    assert!(ConnectionSettings::from_url("sftp://u:secret@host/root").is_err());
    assert!(ConnectionSettings::from_url("https://host/root").is_ok());
}

#[test]
fn folder_urls_keep_protocol_username_encoding_and_default_ports() {
    for (url, protocol, port) in [
        ("sftp://u%20ser@host/a%20b", FileTransferProtocol::Sftp, 22),
        (
            "scp://user@host:2200/folder",
            FileTransferProtocol::Scp,
            2200,
        ),
        (
            "ftp://user@host/folder",
            FileTransferProtocol::Ftp(false),
            21,
        ),
        (
            "ftps://user@host:2121/folder",
            FileTransferProtocol::Ftp(true),
            2121,
        ),
    ] {
        let settings = ConnectionSettings::from_url(url).unwrap();
        let transfer = settings.transfer_params();
        assert_eq!(transfer.protocol, protocol);
        assert_eq!(transfer.params.generic_params().unwrap().port, port);
    }
    let settings = ConnectionSettings::from_url("sftp://u%20ser@[::1]/a%20b").unwrap();
    assert_eq!(settings.bookmark().username.as_deref(), Some("u ser"));
    assert_eq!(settings.bookmark().address.as_deref(), Some("::1"));
    assert_eq!(settings.bookmark().port, None);
    assert_eq!(settings.root(), PathBuf::from("/a b"));
}
