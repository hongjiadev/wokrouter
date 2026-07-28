use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    ManagementError, SessionSource, WokCoreClient,
    management::{map_http_error, push_optional, valid_opaque_key},
    sessions::valid_utc_timestamp,
};

const USAGE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_USAGE_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CURSOR_BYTES: usize = 4096;

impl WokCoreClient {
    pub async fn usage(
        &self,
        token: &SecretString,
        query: &UsageQuery,
    ) -> Result<UsageResponse, ManagementError> {
        query.validate()?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/usage",
                &query.to_pairs(),
                token,
                USAGE_TIMEOUT,
                MAX_USAGE_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_response(response)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageGroup {
    Day,
    Source,
    Model,
}

impl UsageGroup {
    fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Source => "source",
            Self::Model => "model",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageQuery {
    pub source: Option<SessionSource>,
    pub session_key: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub group_by: Option<UsageGroup>,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

impl UsageQuery {
    fn validate(&self) -> Result<(), ManagementError> {
        if self
            .session_key
            .as_ref()
            .is_some_and(|key| !valid_opaque_key(key))
            || self
                .since
                .as_ref()
                .is_some_and(|value| !valid_utc_timestamp(value))
            || self
                .until
                .as_ref()
                .is_some_and(|value| !valid_utc_timestamp(value))
            || self
                .after
                .as_ref()
                .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
            || self.limit.is_some_and(|limit| !(1..=500).contains(&limit))
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut query = Vec::with_capacity(7);
        push_optional(
            &mut query,
            "source",
            self.source.map(|source| match source {
                SessionSource::Codex => "codex",
                SessionSource::Claude => "claude",
                SessionSource::Gemini => "gemini",
            }),
        );
        push_optional(&mut query, "session_key", self.session_key.as_deref());
        push_optional(&mut query, "since", self.since.as_deref());
        push_optional(&mut query, "until", self.until.as_deref());
        push_optional(
            &mut query,
            "group_by",
            self.group_by.map(UsageGroup::as_str),
        );
        push_optional(&mut query, "after", self.after.as_deref());
        if let Some(limit) = self.limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        query
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageResponse {
    pub schema_version: u32,
    pub group_by: UsageGroup,
    pub totals: UsageTotals,
    pub buckets: Vec<UsageBucket>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub session_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageBucket {
    pub key: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub session_count: u64,
}

fn validate_response(response: UsageResponse) -> Result<UsageResponse, ManagementError> {
    let valid = response.schema_version == 1
        && response.buckets.len() <= 500
        && response
            .next_cursor
            .as_ref()
            .is_none_or(|cursor| cursor.len() <= MAX_CURSOR_BYTES)
        && response.buckets.iter().all(|bucket| {
            !bucket.key.is_empty()
                && bucket
                    .start
                    .as_ref()
                    .is_none_or(|value| valid_utc_timestamp(value))
                && bucket
                    .end
                    .as_ref()
                    .is_none_or(|value| valid_utc_timestamp(value))
        });
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}
