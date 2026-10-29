//! Mandatory password/default-secret storage directly in the native OS keychain.
//! No serialized credentials, file fallback, or temporary cloud tokens.
use super::connection::Secrets;
use super::profiles::Endpoint;
use crate::system::{KeyStorage, KeyStorageError};
use sha2::{Digest, Sha256};

fn storage_ids(endpoint: &Endpoint) -> Result<(String, String), String> {
    // Exact, unambiguous endpoint identity: includes protocol, user, host and root.
    // Never reuse credentials after settings change. Profile renames retain access.
    let normalized;
    let endpoint = if let Endpoint::Remote { url } = endpoint {
        normalized = Endpoint::Connection {
            settings: Box::new(super::connection::ConnectionSettings::from_url(url)?),
        };
        &normalized
    } else {
        endpoint
    };
    let identity = serde_json::to_string(endpoint).map_err(|_| "Invalid credential identity")?;
    // Keep all settings in the identity without exposing their serialized form in
    // native permission dialogs. The suffix also distinguishes similar labels.
    let digest = Sha256::digest(identity.as_bytes());
    let mut fingerprint_bytes = [0_u8; 16];
    fingerprint_bytes.copy_from_slice(&digest[..16]);
    let fingerprint = u128::from_be_bytes(fingerprint_bytes);
    let label = match endpoint {
        Endpoint::Connection { settings } => settings.credential_label(),
        _ => endpoint.description(),
    };
    Ok((
        format!("sidemirror password for {label} [{fingerprint:032x}]"),
        format!("sidemirror.comparison.password:{identity}"),
    ))
}

pub fn supplied_secret(secrets: &Secrets) -> Option<&str> {
    secrets
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            secrets
                .secret_access_key
                .as_deref()
                .filter(|value| !value.is_empty())
        })
}

pub fn restore(
    storage: &dyn KeyStorage,
    endpoint: &Endpoint,
    secrets: &mut Secrets,
) -> Result<(), String> {
    if !endpoint.needs_secret_prompt() || supplied_secret(secrets).is_some() {
        return Ok(());
    }
    let (id, legacy_id) = storage_ids(endpoint)?;
    let result = match storage.get_key(&id) {
        Err(KeyStorageError::NoSuchKey) => match storage.get_key(&legacy_id) {
            Ok(secret) => {
                // The first legacy read may still show the old native prompt.
                // Leave the original intact if migration fails.
                remember(storage, endpoint, &secret)?;
                Ok(secret)
            }
            result => result,
        },
        result => result,
    };
    match result {
        Ok(secret) => secrets.password = Some(secret),
        Err(KeyStorageError::NoSuchKey) => {}
        Err(_) => return Err("Cannot read OS keychain; unlock or configure the native credential store and retry. No fallback is supported".into()),
    }
    Ok(())
}

/// Call only after authenticating successfully. User-entered overrides replace old passwords.
pub fn remember(storage: &dyn KeyStorage, endpoint: &Endpoint, secret: &str) -> Result<(), String> {
    storage
        .set_key(&storage_ids(endpoint)?.0, secret)
        .map_err(|_| {
            "Connected, but could not save password in OS keychain; no file fallback was used"
                .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<String, String>>);
    impl KeyStorage for Memory {
        fn get_key(&self, id: &str) -> Result<String, KeyStorageError> {
            self.0
                .borrow()
                .get(id)
                .cloned()
                .ok_or(KeyStorageError::NoSuchKey)
        }
        fn set_key(&self, id: &str, secret: &str) -> Result<(), KeyStorageError> {
            self.0.borrow_mut().insert(id.into(), secret.into());
            Ok(())
        }
        fn is_supported(&self) -> bool {
            true
        }
    }

    #[test]
    fn credentials_are_endpoint_scoped_and_explicit_overrides_win() {
        let storage = Memory::default();
        let endpoint = Endpoint::from_input("sftp://alice@server/root").unwrap();
        remember(&storage, &endpoint, "private-password").unwrap();
        let mut secrets = Secrets::default();
        restore(&storage, &endpoint, &mut secrets).unwrap();
        assert_eq!(secrets.password.as_deref(), Some("private-password"));
        let typed = Endpoint::Connection {
            settings: Box::new(
                super::super::connection::ConnectionSettings::from_url("sftp://alice@server/root")
                    .unwrap(),
            ),
        };
        let mut typed_secret = Secrets::default();
        restore(&storage, &typed, &mut typed_secret).unwrap();
        assert_eq!(typed_secret.password.as_deref(), Some("private-password"));
        for other in [
            "sftp://bob@server/root",
            "sftp://alice@other/root",
            "sftp://alice@server/other",
            "ftp://alice@server/root",
        ] {
            let mut secrets = Secrets::default();
            restore(
                &storage,
                &Endpoint::from_input(other).unwrap(),
                &mut secrets,
            )
            .unwrap();
            assert!(secrets.password.is_none());
        }
        let mut override_secret = Secrets {
            password: Some("override".into()),
            ..Secrets::default()
        };
        restore(&storage, &endpoint, &mut override_secret).unwrap();
        assert_eq!(override_secret.password.as_deref(), Some("override"));
        assert!(
            !serde_json::to_string(&endpoint)
                .unwrap()
                .contains("private-password")
        );
    }

    #[test]
    fn credential_names_are_readable_and_legacy_passwords_are_migrated() {
        let endpoint = Endpoint::from_input("ftp://wuh@172.27.102.3/pimred").unwrap();
        let (id, legacy_id) = storage_ids(&endpoint).unwrap();
        assert!(id.contains("sidemirror password for wuh"));
        assert!(id.contains("FTP"));
        assert!(id.contains("172.27.102.3"));
        assert!(!id.contains('{'));
        assert!(!id.contains("null"));
        assert_eq!(id, storage_ids(&endpoint).unwrap().0);

        let storage = Memory::default();
        storage.set_key(&legacy_id, "old-password").unwrap();
        let mut secrets = Secrets::default();
        restore(&storage, &endpoint, &mut secrets).unwrap();
        assert_eq!(secrets.password.as_deref(), Some("old-password"));
        assert_eq!(storage.get_key(&id).unwrap(), "old-password");

        storage.set_key(&id, "new-password").unwrap();
        let mut secrets = Secrets::default();
        restore(&storage, &endpoint, &mut secrets).unwrap();
        assert_eq!(secrets.password.as_deref(), Some("new-password"));
    }

    #[test]
    fn temporary_cloud_tokens_are_not_selected_for_persistence() {
        let secrets = Secrets {
            security_token: Some("temporary".into()),
            session_token: Some("temporary".into()),
            ..Secrets::default()
        };
        assert!(supplied_secret(&secrets).is_none());
    }

    #[test]
    fn s3_restores_default_secret_without_persisting_temporary_tokens() {
        use super::super::connection::ConnectionSettings;
        use crate::filetransfer::{
            FileTransferParams, FileTransferProtocol, ProtocolParams, params::AwsS3Params,
        };
        let (settings, supplied) = ConnectionSettings::from_params(
            FileTransferParams::new(
                FileTransferProtocol::AwsS3,
                ProtocolParams::AwsS3(
                    AwsS3Params::new("bucket", Some("region"), None)
                        .secret_access_key(Some("private-key"))
                        .session_token(Some("temporary-token")),
                ),
            )
            .remote_path(Some("/prefix")),
        )
        .unwrap();
        let endpoint = Endpoint::Connection {
            settings: Box::new(settings.clone()),
        };
        let storage = Memory::default();
        remember(&storage, &endpoint, supplied_secret(&supplied).unwrap()).unwrap();
        assert!(
            storage
                .0
                .borrow()
                .values()
                .all(|value| value == "private-key")
        );
        let mut restored = Secrets::default();
        restore(&storage, &endpoint, &mut restored).unwrap();
        let mut transfer = settings.transfer_params();
        restored.apply(&mut transfer.params);
        let ProtocolParams::AwsS3(params) = transfer.params else {
            panic!("expected S3");
        };
        assert_eq!(params.secret_access_key.as_deref(), Some("private-key"));
        assert!(params.session_token.is_none());
        assert!(params.security_token.is_none());
    }

    #[test]
    fn keychain_errors_are_explicit_and_do_not_disclose_provider_data() {
        struct Locked;
        impl KeyStorage for Locked {
            fn get_key(&self, _: &str) -> Result<String, KeyStorageError> {
                Err(KeyStorageError::ProviderError)
            }
            fn set_key(&self, _: &str, _: &str) -> Result<(), KeyStorageError> {
                Err(KeyStorageError::ProviderError)
            }
            fn is_supported(&self) -> bool {
                false
            }
        }
        let endpoint = Endpoint::from_input("sftp://server/root").unwrap();
        assert!(
            restore(&Locked, &endpoint, &mut Secrets::default())
                .unwrap_err()
                .contains("OS keychain")
        );
        assert!(
            remember(&Locked, &endpoint, "secret")
                .unwrap_err()
                .contains("no file fallback")
        );
    }
}
