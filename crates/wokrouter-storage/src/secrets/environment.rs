use std::ffi::OsString;

use secrecy::SecretString;
use wokrouter_core::secret::{SecretRef, SecretScope};
use zeroize::Zeroize;

use crate::{HeadlessSecretStoreConfig, SecretStore, StorageError};

#[derive(Clone, Debug)]
pub struct EnvironmentSecretStore {
    secret_ref: SecretRef,
    variable_name: String,
}

impl EnvironmentSecretStore {
    pub fn from_config(config: HeadlessSecretStoreConfig) -> Result<Self, StorageError> {
        let HeadlessSecretStoreConfig::Environment {
            secret_ref,
            variable_name,
        } = config
        else {
            return Err(StorageError::InvalidHeadlessSecretStoreConfig);
        };
        if variable_name.is_empty() || variable_name.contains('=') || variable_name.contains('\0') {
            return Err(StorageError::InvalidHeadlessSecretStoreConfig);
        }
        Ok(Self {
            secret_ref,
            variable_name,
        })
    }
}

#[async_trait::async_trait]
impl SecretStore for EnvironmentSecretStore {
    async fn put(
        &self,
        _scope: &SecretScope,
        _value: SecretString,
    ) -> Result<SecretRef, StorageError> {
        Err(StorageError::ReadOnlySecretStore)
    }

    async fn get(&self, secret_ref: &SecretRef) -> Result<SecretString, StorageError> {
        if secret_ref != &self.secret_ref {
            return Err(StorageError::SecretNotFound);
        }
        let value = std::env::var_os(&self.variable_name).ok_or(StorageError::SecretNotFound)?;
        secret_from_os_string(value)
    }

    async fn delete(&self, secret_ref: &SecretRef) -> Result<(), StorageError> {
        if secret_ref != &self.secret_ref {
            return Ok(());
        }
        Err(StorageError::ReadOnlySecretStore)
    }
}

fn secret_from_os_string(value: OsString) -> Result<SecretString, StorageError> {
    match value.into_string() {
        Ok(value) => Ok(SecretString::from(value)),
        Err(value) => {
            let mut bytes = value.into_encoded_bytes();
            bytes.zeroize();
            Err(StorageError::InvalidSecretEncoding)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::secret_from_os_string;
    use crate::StorageError;

    #[cfg(unix)]
    #[test]
    fn non_unicode_environment_value_is_rejected_without_formatting_its_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let value = std::ffi::OsString::from_vec(vec![0xff]);

        assert!(matches!(
            secret_from_os_string(value),
            Err(StorageError::InvalidSecretEncoding)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn non_unicode_environment_value_is_rejected_without_formatting_its_bytes() {
        use std::os::windows::ffi::OsStringExt;

        let value = std::ffi::OsString::from_wide(&[0xd800]);

        assert!(matches!(
            secret_from_os_string(value),
            Err(StorageError::InvalidSecretEncoding)
        ));
    }
}
