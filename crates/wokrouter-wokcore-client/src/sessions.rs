use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    ManagementError, WokCoreClient,
    management::{map_http_error, push_optional, valid_opaque_key},
};

const SESSION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SESSION_LIST_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SESSION_MESSAGES_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CURSOR_BYTES: usize = 4096;

impl WokCoreClient {
    pub async fn list_sessions(
        &self,
        token: &SecretString,
        query: &SessionQuery,
    ) -> Result<SessionList, ManagementError> {
        query.validate()?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/sessions",
                &query.to_pairs(),
                token,
                SESSION_TIMEOUT,
                MAX_SESSION_LIST_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_session_list(response)
    }

    pub async fn session_messages(
        &self,
        token: &SecretString,
        session_key: &str,
        query: &SessionMessageQuery,
    ) -> Result<SessionMessages, ManagementError> {
        if !valid_opaque_key(session_key) {
            return Err(ManagementError::InvalidInput);
        }
        query.validate()?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                &format!("/wokcore/v1/sessions/{session_key}/messages"),
                &query.to_pairs(),
                token,
                SESSION_TIMEOUT,
                MAX_SESSION_MESSAGES_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_session_messages(response)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Codex,
    Claude,
    Gemini,
}

impl SessionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAvailability {
    Available,
    Unavailable,
}

impl SessionAvailability {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionQuery {
    pub source: Option<SessionSource>,
    pub availability: Option<SessionAvailability>,
    pub before: Option<String>,
    pub limit: Option<u16>,
}

impl SessionQuery {
    fn validate(&self) -> Result<(), ManagementError> {
        if self.limit.is_some_and(|limit| !(1..=200).contains(&limit))
            || self
                .before
                .as_ref()
                .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut query = Vec::with_capacity(4);
        push_optional(&mut query, "source", self.source.map(SessionSource::as_str));
        push_optional(
            &mut query,
            "availability",
            self.availability.map(SessionAvailability::as_str),
        );
        push_optional(&mut query, "before", self.before.as_deref());
        if let Some(limit) = self.limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        query
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionMessageQuery {
    pub after: Option<String>,
    pub limit: Option<u16>,
    pub max_bytes: Option<u32>,
}

impl SessionMessageQuery {
    fn validate(&self) -> Result<(), ManagementError> {
        if self.limit.is_some_and(|limit| !(1..=500).contains(&limit))
            || self
                .max_bytes
                .is_some_and(|bytes| !(4096..=1_048_576).contains(&bytes))
            || self
                .after
                .as_ref()
                .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut query = Vec::with_capacity(3);
        push_optional(&mut query, "after", self.after.as_deref());
        if let Some(limit) = self.limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        if let Some(max_bytes) = self.max_bytes {
            query.push(("max_bytes".to_owned(), max_bytes.to_string()));
        }
        query
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionList {
    pub schema_version: u32,
    pub items: Vec<SessionListItem>,
    pub next_cursor: Option<String>,
    pub index_status: IndexStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionListItem {
    pub session_key: String,
    pub source: SessionSource,
    pub created_at: String,
    pub last_active_at: String,
    pub availability: SessionAvailability,
    pub message_count: u64,
    pub usage_event_count: u64,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexStatus {
    pub phase: IndexPhase,
    pub sources: Vec<SourceIndexStatus>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPhase {
    Starting,
    Scanning,
    Idle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceIndexStatus {
    pub source: SessionSource,
    pub status: SourceAvailability,
    pub last_transition_at: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAvailability {
    Undiscovered,
    Available,
    Stale,
    Unavailable,
    ResourceLimited,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionMessages {
    pub schema_version: u32,
    pub items: Vec<SessionMessage>,
    pub next_cursor: Option<String>,
    pub source_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionMessage {
    pub message_key: String,
    pub role: MessageRole,
    pub timestamp: String,
    pub content: String,
    pub fragment_offset_bytes: u64,
    pub fragment_final: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

fn validate_session_list(response: SessionList) -> Result<SessionList, ManagementError> {
    let valid = response.schema_version == 1
        && response.items.len() <= 200
        && response.index_status.sources.len() <= 3
        && response
            .next_cursor
            .as_ref()
            .is_none_or(|cursor| cursor.len() <= MAX_CURSOR_BYTES)
        && response.items.iter().all(|session| {
            valid_opaque_key(&session.session_key)
                && valid_utc_timestamp(&session.created_at)
                && valid_utc_timestamp(&session.last_active_at)
        })
        && response.index_status.sources.iter().all(|source| {
            source
                .last_transition_at
                .as_ref()
                .is_none_or(|timestamp| valid_utc_timestamp(timestamp))
        });
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn validate_session_messages(
    response: SessionMessages,
) -> Result<SessionMessages, ManagementError> {
    let valid = response.schema_version == 1
        && response.source_generation > 0
        && response.items.len() <= 500
        && response
            .next_cursor
            .as_ref()
            .is_none_or(|cursor| cursor.len() <= MAX_CURSOR_BYTES)
        && response.items.iter().all(|message| {
            valid_opaque_key(&message.message_key) && valid_utc_timestamp(&message.timestamp)
        });
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

pub(crate) fn valid_utc_timestamp(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let bytes = value.as_bytes();
    for index in [4, 7, 10, 13, 16, 19] {
        let expected = match index {
            4 | 7 => b'-',
            10 => b'T',
            13 | 16 => b':',
            19 => b'Z',
            _ => unreachable!(),
        };
        if bytes[index] != expected {
            return false;
        }
    }
    bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| [4, 7, 10, 13, 16, 19].contains(&index) || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::valid_utc_timestamp;

    #[test]
    fn utc_timestamp_accepts_only_second_precision_zulu_wire_values() {
        assert!(valid_utc_timestamp("2026-07-27T08:01:02Z"));
        assert!(!valid_utc_timestamp("2026-07-27T08:01:02+08:00"));
        assert!(!valid_utc_timestamp("2026-07-27T08:01Z"));
    }
}
