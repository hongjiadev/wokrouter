use keyring::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

const TOKEN_SERVICE: &str = "dev.wokrouter.wokcore";
const TOKEN_ACCOUNT: &str = "wokrouter.desktop";

#[async_trait::async_trait]
pub trait WokCoreTokenVault: Send + Sync {
    async fn load(&self) -> Result<Option<SecretString>, TokenVaultError>;

    async fn store(&self, token: SecretString) -> Result<(), TokenVaultError>;

    async fn delete(&self) -> Result<(), TokenVaultError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeWokCoreTokenVault;

impl NativeWokCoreTokenVault {
    pub const fn new() -> Self {
        Self
    }

    fn entry() -> Result<Entry, TokenVaultError> {
        Entry::new(TOKEN_SERVICE, TOKEN_ACCOUNT).map_err(map_keyring_error)
    }
}

#[async_trait::async_trait]
impl WokCoreTokenVault for NativeWokCoreTokenVault {
    async fn load(&self) -> Result<Option<SecretString>, TokenVaultError> {
        match Self::entry()?.get_password() {
            Ok(token) => Ok(Some(SecretString::from(token))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(error)),
        }
    }

    async fn store(&self, token: SecretString) -> Result<(), TokenVaultError> {
        Self::entry()?
            .set_password(token.expose_secret())
            .map_err(map_keyring_error)
    }

    async fn delete(&self) -> Result<(), TokenVaultError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

fn map_keyring_error(error: KeyringError) -> TokenVaultError {
    match error {
        KeyringError::NoDefaultStore
        | KeyringError::NoStorageAccess(_)
        | KeyringError::PlatformFailure(_) => TokenVaultError::Unavailable,
        KeyringError::BadEncoding(mut bytes) => {
            bytes.zeroize();
            TokenVaultError::InvalidEncoding
        }
        KeyringError::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            TokenVaultError::InvalidEncoding
        }
        _ => TokenVaultError::BackendFailure,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TokenVaultError {
    #[error("the native credential service for the WokCore token is unavailable")]
    Unavailable,
    #[error("the WokCore token credential backend failed")]
    BackendFailure,
    #[error("the stored WokCore token has invalid encoding")]
    InvalidEncoding,
}

#[cfg(test)]
mod tests {
    use super::{
        NativeWokCoreTokenVault, TOKEN_ACCOUNT, TOKEN_SERVICE, TokenVaultError, map_keyring_error,
    };

    #[test]
    fn native_vault_has_one_fixed_credential_and_no_plaintext_fallback() {
        assert_eq!(TOKEN_SERVICE, "dev.wokrouter.wokcore");
        assert_eq!(TOKEN_ACCOUNT, "wokrouter.desktop");
        assert_eq!(std::mem::size_of::<NativeWokCoreTokenVault>(), 0);
    }

    #[test]
    fn unavailable_native_service_stays_explicit() {
        assert_eq!(
            map_keyring_error(keyring::Error::NoDefaultStore),
            TokenVaultError::Unavailable
        );
    }
}
