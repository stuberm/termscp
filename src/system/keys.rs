//! ## KeyStorage
//!
//! `keystorage` provides the trait to manipulate to a KeyStorage

// Storages
pub mod keyringstorage;
// ext
use keyring_core::Error as KeyringError;
use thiserror::Error;

/// Reject attempts to disable mandatory, unconditionally compiled native storage.
pub(crate) fn require_native_store(enabled: bool) -> Result<(), String> {
    if !enabled {
        return Err("Native credential storage is required. Remove --wno-keyring; no session-only or file fallback is supported.".into());
    }
    Ok(())
}

/// defines the error type for the `KeyStorage`
#[derive(Debug, Error)]
pub enum KeyStorageError {
    #[error("Key has a bad syntax")]
    BadSyntax,
    #[error("Provider service error")]
    ProviderError,
    #[error("No such key")]
    NoSuchKey,
    #[error("keyring error: {0}")]
    KeyringError(KeyringError),
}

impl From<KeyringError> for KeyStorageError {
    fn from(e: KeyringError) -> Self {
        Self::KeyringError(e)
    }
}

/// this traits provides the methods to communicate and interact with the key storage.
pub trait KeyStorage {
    /// Retrieve key from the key storage.
    /// The key might be acccess through an identifier, which identifies
    /// the key in the storage
    fn get_key(&self, storage_id: &str) -> Result<String, KeyStorageError>;

    /// Set the key into the key storage
    fn set_key(&self, storage_id: &str, key: &str) -> Result<(), KeyStorageError>;

    /// is_supported
    ///
    /// Returns whether the key storage is supported on the host system
    fn is_supported(&self) -> bool;
}

#[cfg(test)]
mod tests {

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn native_storage_is_required_even_without_default_features() {
        assert!(require_native_store(true).is_ok());
        assert!(require_native_store(false).is_err());
    }

    #[test]
    fn test_system_keys_mod_errors() {
        assert_eq!(
            KeyStorageError::BadSyntax.to_string(),
            String::from("Key has a bad syntax")
        );
        assert_eq!(
            KeyStorageError::ProviderError.to_string(),
            String::from("Provider service error")
        );
        assert_eq!(
            KeyStorageError::NoSuchKey.to_string(),
            String::from("No such key")
        );
    }
}
