use crate::{
    SUPPORTED_API_MAJOR, WokCoreClient,
    discovery::{DiscoveryRead, ValidatedDiscovery},
    http::ProtectedHttpError,
};

impl WokCoreClient {
    pub(crate) fn management_discovery(&self) -> Result<ValidatedDiscovery, ManagementError> {
        match self.read_discovery() {
            DiscoveryRead::Missing => Err(ManagementError::Missing),
            DiscoveryRead::Invalid => Err(ManagementError::InvalidRuntime),
            DiscoveryRead::Record(discovery) if discovery.api_major != SUPPORTED_API_MAJOR => {
                Err(ManagementError::Incompatible)
            }
            DiscoveryRead::Record(discovery) => Ok(discovery),
        }
    }
}

pub(crate) fn map_http_error(error: ProtectedHttpError) -> ManagementError {
    match error {
        ProtectedHttpError::Transport => ManagementError::Stopped,
        ProtectedHttpError::Unauthorized => ManagementError::Unauthorized,
        ProtectedHttpError::Forbidden => ManagementError::Forbidden,
        ProtectedHttpError::Conflict => ManagementError::Conflict,
        ProtectedHttpError::InvalidRequest => ManagementError::InvalidInput,
        ProtectedHttpError::InvalidResponse => ManagementError::InvalidResponse,
    }
}

pub(crate) fn push_optional(query: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        query.push((name.to_owned(), value.to_owned()));
    }
}

pub(crate) fn valid_opaque_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && value
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

pub(crate) fn valid_secret_ref(value: &str) -> bool {
    let Some(value) = value.strip_prefix("secret:") else {
        return false;
    };
    let value = value
        .strip_prefix("urn:uuid:")
        .or_else(|| value.strip_prefix('{').and_then(|v| v.strip_suffix('}')))
        .unwrap_or(value);
    let compact = value.replace('-', "");
    compact.len() == 32 && compact.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ManagementError {
    #[error("WokCore runtime metadata is missing")]
    Missing,
    #[error("WokCore is not running")]
    Stopped,
    #[error("WokCore API version is incompatible")]
    Incompatible,
    #[error("WokCore runtime metadata is invalid")]
    InvalidRuntime,
    #[error("WokCore client authorization is required")]
    Unauthorized,
    #[error("WokCore denied the requested capability")]
    Forbidden,
    #[error("WokCore configuration revision changed")]
    Conflict,
    #[error("the management request is invalid")]
    InvalidInput,
    #[error("WokCore returned an invalid management response")]
    InvalidResponse,
}
