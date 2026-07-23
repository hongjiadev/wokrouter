use keyring::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use wokrouter_core::secret::{SecretRef, SecretScope};
use zeroize::Zeroize;

use crate::{SecretStore, StorageError};

const NATIVE_SERVICE_NAME: &str = "dev.wokrouter.credentials";

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeSecretStore;

impl NativeSecretStore {
    pub const fn new() -> Self {
        Self
    }

    fn entry(secret_ref: &SecretRef) -> Result<Entry, StorageError> {
        Entry::new(NATIVE_SERVICE_NAME, secret_ref.as_str()).map_err(map_keyring_error)
    }
}

#[async_trait::async_trait]
impl SecretStore for NativeSecretStore {
    async fn put(
        &self,
        _scope: &SecretScope,
        value: SecretString,
    ) -> Result<SecretRef, StorageError> {
        let secret_ref = SecretRef::new();
        Self::entry(&secret_ref)?
            .set_password(value.expose_secret())
            .map_err(map_keyring_error)?;
        Ok(secret_ref)
    }

    async fn get(&self, secret_ref: &SecretRef) -> Result<SecretString, StorageError> {
        let value = Self::entry(secret_ref)?
            .get_password()
            .map_err(map_keyring_error)?;
        Ok(SecretString::from(value))
    }

    async fn delete(&self, secret_ref: &SecretRef) -> Result<(), StorageError> {
        match Self::entry(secret_ref)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

fn map_keyring_error(error: KeyringError) -> StorageError {
    match error {
        KeyringError::NoEntry => StorageError::SecretNotFound,
        KeyringError::NoDefaultStore
        | KeyringError::NoStorageAccess(_)
        | KeyringError::PlatformFailure(_) => StorageError::CredentialServiceUnavailable,
        KeyringError::BadEncoding(mut bytes) => {
            bytes.zeroize();
            StorageError::InvalidSecretEncoding
        }
        KeyringError::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            StorageError::InvalidSecretEncoding
        }
        _ => StorageError::SecretBackendFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::{NATIVE_SERVICE_NAME, NativeSecretStore, map_keyring_error};
    use crate::StorageError;

    #[test]
    fn unavailable_native_service_never_selects_a_headless_fallback() {
        let error = map_keyring_error(keyring::Error::NoDefaultStore);

        assert!(matches!(error, StorageError::CredentialServiceUnavailable));
    }

    #[test]
    fn native_store_uses_only_the_required_service() {
        assert_eq!(NATIVE_SERVICE_NAME, "dev.wokrouter.credentials");
        assert_eq!(std::mem::size_of::<NativeSecretStore>(), 0);
    }
}
