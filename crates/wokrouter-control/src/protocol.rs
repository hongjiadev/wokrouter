use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CONTROL_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    Running,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct DaemonStatus {
    pub state: DaemonState,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ControlRequest {
    Ping,
    Status,
    Reload { expected_revision: u64 },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum ControlResponse {
    Pong { protocol_version: u16 },
    Status(DaemonStatus),
    Accepted { revision: u64 },
    Error(ControlError),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, thiserror::Error)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ControlError {
    #[error("control protocol version {client} is incompatible with daemon version {daemon}")]
    IncompatibleVersion { client: u16, daemon: u16 },
    #[error("control frame length {length} exceeds the {max}-byte limit")]
    FrameTooLarge { length: u32, max: u32 },
    #[error("control endpoint is already in use")]
    EndpointInUse,
    #[error("control endpoint is unavailable")]
    EndpointUnavailable,
    #[error("configured data-plane port {port} is already in use")]
    DataPlanePortInUse { port: u16 },
    #[error("invalid control frame: {message}")]
    InvalidFrame { message: String },
    #[error("control transport failed: {message}")]
    Transport { message: String },
    #[error("control response request ID did not match")]
    RequestIdMismatch,
    #[error("control server task failed")]
    ServerTaskFailed,
    #[error("configuration revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
}

impl From<std::io::Error> for ControlError {
    fn from(error: std::io::Error) -> Self {
        Self::Transport {
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Frame<T> {
    pub(crate) protocol_version: u16,
    pub(crate) request_id: Uuid,
    pub(crate) payload: T,
}
