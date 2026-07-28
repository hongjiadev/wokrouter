use std::{
    fmt,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::SecretString;
use serde::Deserialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time::timeout,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_AUTHORIZATION_STDOUT_BYTES: usize = 64 * 1024;
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(15);
const CLIENT_ID: &str = "wokrouter.desktop";
const REQUIRED_SCOPES: [&str; 9] = [
    "service.read",
    "service.control",
    "providers.read",
    "providers.write",
    "clients.manage",
    "sessions.read",
    "usage.read",
    "diagnostics.read",
    "diagnostics.export",
];
const AUTHORIZATION_ARGUMENTS: [&str; 22] = [
    "authorize",
    "--client",
    CLIENT_ID,
    "--scope",
    REQUIRED_SCOPES[0],
    "--scope",
    REQUIRED_SCOPES[1],
    "--scope",
    REQUIRED_SCOPES[2],
    "--scope",
    REQUIRED_SCOPES[3],
    "--scope",
    REQUIRED_SCOPES[4],
    "--scope",
    REQUIRED_SCOPES[5],
    "--scope",
    REQUIRED_SCOPES[6],
    "--scope",
    REQUIRED_SCOPES[7],
    "--scope",
    REQUIRED_SCOPES[8],
    "--json",
];

pub struct WokCoreAuthorizer {
    executable: PathBuf,
}

impl WokCoreAuthorizer {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub async fn authorize(&self) -> Result<SecretString, AuthorizationError> {
        let mut command = Command::new(&self.executable);
        command
            .args(AUTHORIZATION_ARGUMENTS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        configure_child_window(&mut command);

        let mut child = command.spawn().map_err(map_spawn_error)?;
        let mut stdout = child.stdout.take().ok_or(AuthorizationError::Failed)?;
        let execution = timeout(AUTHORIZATION_TIMEOUT, async {
            let output = read_bounded(&mut stdout).await?;
            let status = child.wait().await.map_err(|_| AuthorizationError::Failed)?;
            Ok((status, output))
        })
        .await;

        match execution {
            Ok(Ok((status, output))) => parse_success(status, output),
            Ok(Err(error)) => {
                terminate(&mut child).await;
                Err(error)
            }
            Err(_) => {
                terminate(&mut child).await;
                Err(AuthorizationError::TimedOut)
            }
        }
    }
}

impl fmt::Debug for WokCoreAuthorizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WokCoreAuthorizer")
            .finish_non_exhaustive()
    }
}

#[cfg(windows)]
fn configure_child_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_child_window(_command: &mut Command) {}

async fn read_bounded(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Zeroizing<Vec<u8>>, AuthorizationError> {
    let mut output = Zeroizing::new(Vec::with_capacity(4096));
    let mut chunk = Zeroizing::new([0_u8; 8192]);
    loop {
        let read = reader
            .read(chunk.as_mut())
            .await
            .map_err(|_| AuthorizationError::Failed)?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > MAX_AUTHORIZATION_STDOUT_BYTES {
            return Err(AuthorizationError::OutputTooLarge);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

async fn terminate(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn map_spawn_error(error: std::io::Error) -> AuthorizationError {
    match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            AuthorizationError::Unavailable
        }
        _ => AuthorizationError::Failed,
    }
}

fn parse_success(
    status: ExitStatus,
    output: Zeroizing<Vec<u8>>,
) -> Result<SecretString, AuthorizationError> {
    if !status.success() {
        return Err(AuthorizationError::Failed);
    }
    parse_authorization(&output)
}

fn parse_authorization(output: &[u8]) -> Result<SecretString, AuthorizationError> {
    let mut response = serde_json::from_slice::<AuthorizationWire>(output)
        .map_err(|_| AuthorizationError::InvalidResponse)?;
    if response.client_id != CLIENT_ID
        || !valid_token_id(&response.token_id)
        || !valid_scopes(&response.scopes)
        || !valid_token(&response.token)
    {
        return Err(AuthorizationError::InvalidResponse);
    }
    Ok(SecretString::from(std::mem::take(&mut response.token)))
}

fn valid_token_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_scopes(scopes: &[String]) -> bool {
    if scopes.len() != REQUIRED_SCOPES.len() {
        return false;
    }
    let mut seen = [false; REQUIRED_SCOPES.len()];
    for scope in scopes {
        let Some(index) = REQUIRED_SCOPES
            .iter()
            .position(|required| *required == scope)
        else {
            return false;
        };
        if std::mem::replace(&mut seen[index], true) {
            return false;
        }
    }
    seen.into_iter().all(|present| present)
}

fn valid_token(token: &str) -> bool {
    let Some(encoded) = token.strip_prefix("wok_proxy_v1_") else {
        return false;
    };
    if encoded.len() != 43 {
        return false;
    }
    let mut decoded = Zeroizing::new([0_u8; 32]);
    matches!(
        URL_SAFE_NO_PAD.decode_slice(encoded, decoded.as_mut()),
        Ok(32)
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationWire {
    client_id: String,
    token_id: String,
    token: String,
    scopes: Vec<String>,
}

impl Drop for AuthorizationWire {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationState {
    Ready,
    Required,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthorizationError {
    #[error("the WokCore executable is unavailable")]
    Unavailable,
    #[error("WokCore authorization timed out")]
    TimedOut,
    #[error("WokCore authorization output exceeded its limit")]
    OutputTooLarge,
    #[error("WokCore authorization failed")]
    Failed,
    #[error("WokCore returned an invalid authorization response")]
    InvalidResponse,
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::{
        AUTHORIZATION_ARGUMENTS, AuthorizationError, CLIENT_ID, MAX_AUTHORIZATION_STDOUT_BYTES,
        REQUIRED_SCOPES, parse_authorization, read_bounded,
    };

    fn valid_response(token: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "client_id": CLIENT_ID,
            "token_id": "synthetic-token-id",
            "token": token,
            "scopes": REQUIRED_SCOPES,
        }))
        .unwrap()
    }

    #[test]
    fn authorization_command_contains_only_fixed_non_secret_arguments() {
        assert_eq!(AUTHORIZATION_ARGUMENTS.first(), Some(&"authorize"));
        assert_eq!(AUTHORIZATION_ARGUMENTS.last(), Some(&"--json"));
        assert_eq!(
            AUTHORIZATION_ARGUMENTS
                .windows(2)
                .filter(|pair| pair[0] == "--scope")
                .map(|pair| pair[1])
                .collect::<Vec<_>>(),
            REQUIRED_SCOPES
        );
        assert!(
            !AUTHORIZATION_ARGUMENTS
                .iter()
                .any(|argument| argument.starts_with("wok_proxy_v1_"))
        );
    }

    #[test]
    fn valid_authorization_returns_only_zeroizing_secret_material() {
        let token = format!("wok_proxy_v1_{}", "A".repeat(43));
        let parsed = parse_authorization(&valid_response(&token)).unwrap();

        assert_eq!(parsed.expose_secret(), &token);
    }

    #[test]
    fn invalid_token_never_appears_in_the_error_contract() {
        let token = "wok_proxy_v1_private-invalid-token";
        let error = parse_authorization(&valid_response(token)).unwrap_err();

        assert_eq!(error, AuthorizationError::InvalidResponse);
        assert!(!error.to_string().contains(token));
        assert!(!format!("{error:?}").contains(token));
    }

    #[tokio::test]
    async fn authorization_stdout_is_rejected_at_the_first_byte_over_the_limit() {
        let oversized = vec![b'x'; MAX_AUTHORIZATION_STDOUT_BYTES + 1];
        let mut input = oversized.as_slice();

        let error = read_bounded(&mut input).await.unwrap_err();

        assert_eq!(error, AuthorizationError::OutputTooLarge);
    }
}
