use std::{fs::File, io::Read, path::PathBuf};

use secrecy::{ExposeSecret, SecretString};

use super::{
    atomic_edit::{
        create_private_directory, private_file, remove_private_file, replace_private_file,
    },
    integrations::ClientKind,
    journal::MutationError,
};

const TOKEN_PREFIX: &str = "wok_proxy_v1_";
const MAX_TOKEN_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug)]
pub(super) struct ClientTokenStore {
    root: PathBuf,
}

impl ClientTokenStore {
    pub(super) fn new(root: PathBuf) -> Result<Self, MutationError> {
        create_private_directory(&root)?;
        Ok(Self::open(root))
    }

    pub(super) fn open(root: PathBuf) -> Self {
        Self { root }
    }

    pub(super) fn write(
        &self,
        client: ClientKind,
        token: &SecretString,
    ) -> Result<(), MutationError> {
        validate_token(token.expose_secret())?;
        create_private_directory(&self.root)?;
        replace_private_file(&self.path(client), token.expose_secret().as_bytes())
    }

    pub(super) fn read(&self, client: ClientKind) -> Result<SecretString, MutationError> {
        let path = self.path(client);
        if !private_file(&path) {
            return Err(MutationError::InvalidRecord);
        }
        let mut file = File::open(path).map_err(|_| MutationError::Io)?;
        let metadata = file.metadata().map_err(|_| MutationError::Io)?;
        if metadata.len() > MAX_TOKEN_BYTES as u64 {
            return Err(MutationError::InvalidRecord);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take((MAX_TOKEN_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| MutationError::Io)?;
        if bytes.len() > MAX_TOKEN_BYTES {
            return Err(MutationError::InvalidRecord);
        }
        let token = String::from_utf8(bytes).map_err(|_| MutationError::InvalidRecord)?;
        validate_token(&token)?;
        Ok(SecretString::from(token))
    }

    pub(super) fn remove(&self, client: ClientKind) -> Result<(), MutationError> {
        remove_private_file(&self.path(client))
    }

    fn path(&self, client: ClientKind) -> PathBuf {
        self.root.join(format!("{}.token", client.as_str()))
    }
}

fn validate_token(token: &str) -> Result<(), MutationError> {
    if token.len() <= TOKEN_PREFIX.len()
        || token.len() > MAX_TOKEN_BYTES
        || !token.starts_with(TOKEN_PREFIX)
        || token.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(MutationError::InvalidRecord);
    }
    Ok(())
}
