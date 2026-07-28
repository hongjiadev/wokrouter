use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use wokrouter_storage::{TokenVaultError, WokCoreTokenVault};

#[derive(Default)]
struct MemoryTokenVault {
    token: Mutex<Option<SecretString>>,
}

#[async_trait::async_trait]
impl WokCoreTokenVault for MemoryTokenVault {
    async fn load(&self) -> Result<Option<SecretString>, TokenVaultError> {
        let token = self
            .token
            .lock()
            .map_err(|_| TokenVaultError::BackendFailure)?;
        Ok(token
            .as_ref()
            .map(|value| SecretString::from(value.expose_secret().to_owned())))
    }

    async fn store(&self, token: SecretString) -> Result<(), TokenVaultError> {
        *self
            .token
            .lock()
            .map_err(|_| TokenVaultError::BackendFailure)? = Some(token);
        Ok(())
    }

    async fn delete(&self) -> Result<(), TokenVaultError> {
        self.token
            .lock()
            .map_err(|_| TokenVaultError::BackendFailure)?
            .take();
        Ok(())
    }
}

#[tokio::test]
async fn token_vault_contract_never_requires_plaintext_metadata() {
    let vault = MemoryTokenVault::default();
    assert!(vault.load().await.unwrap().is_none());

    vault
        .store(SecretString::from("synthetic-token".to_owned()))
        .await
        .unwrap();
    let loaded = vault.load().await.unwrap().unwrap();
    assert_eq!(loaded.expose_secret(), "synthetic-token");

    vault.delete().await.unwrap();
    assert!(vault.load().await.unwrap().is_none());
}

#[test]
fn token_vault_errors_are_stable_and_secret_free() {
    let error = TokenVaultError::BackendFailure;
    assert_eq!(
        error.to_string(),
        "the WokCore token credential backend failed"
    );
    assert_eq!(format!("{error:?}"), "BackendFailure");
}
