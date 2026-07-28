use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::{
    ManagementError, WokCoreClient,
    management::{map_http_error, push_optional},
    sessions::valid_utc_timestamp,
};

const LOG_TIMEOUT: Duration = Duration::from_secs(5);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LOG_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_EXPORT_BYTES: usize = 16 * 1024 * 1024;
const MIN_EXPORT_BYTES: u32 = 65_536;
const MAX_EXPORT_BYTES: u32 = 67_108_864;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_FILTER_BYTES: usize = 256;

impl WokCoreClient {
    pub async fn diagnostic_logs(
        &self,
        token: &SecretString,
        query: &DiagnosticLogQuery,
    ) -> Result<DiagnosticLogs, ManagementError> {
        query.validate()?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/logs",
                &query.to_pairs(),
                token,
                LOG_TIMEOUT,
                MAX_LOG_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_logs(response)
    }

    pub async fn export_diagnostics(
        &self,
        token: &SecretString,
        query: &DiagnosticExportQuery,
    ) -> Result<Zeroizing<Vec<u8>>, ManagementError> {
        query.validate()?;
        let discovery = self.management_discovery()?;
        self.http
            .protected_bytes_query(
                &discovery,
                "/wokcore/v1/diagnostics/export",
                &query.to_pairs(),
                token,
                EXPORT_TIMEOUT,
                query
                    .max_bytes
                    .map_or(DEFAULT_EXPORT_BYTES, |bytes| bytes as usize),
            )
            .await
            .map_err(map_http_error)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl DiagnosticLogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOrder {
    Asc,
    Desc,
}

impl DiagnosticOrder {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticLogQuery {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub session_key: Option<String>,
    pub level_min: Option<DiagnosticLogLevel>,
    pub component: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub order: Option<DiagnosticOrder>,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

impl DiagnosticLogQuery {
    fn validate(&self) -> Result<(), ManagementError> {
        validate_common_filters(
            self.request_id.as_deref(),
            self.trace_id.as_deref(),
            self.session_key.as_deref(),
            self.since.as_deref(),
            self.until.as_deref(),
        )?;
        if self
            .component
            .as_ref()
            .is_some_and(|component| component.is_empty() || component.len() > MAX_FILTER_BYTES)
            || self
                .after
                .as_ref()
                .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
            || self.limit.is_some_and(|limit| !(1..=1000).contains(&limit))
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut query = Vec::with_capacity(10);
        append_common_filters(
            &mut query,
            self.request_id.as_deref(),
            self.trace_id.as_deref(),
            self.session_key.as_deref(),
            self.since.as_deref(),
            self.until.as_deref(),
        );
        push_optional(
            &mut query,
            "level_min",
            self.level_min.map(DiagnosticLogLevel::as_str),
        );
        push_optional(&mut query, "component", self.component.as_deref());
        push_optional(&mut query, "order", self.order.map(DiagnosticOrder::as_str));
        push_optional(&mut query, "after", self.after.as_deref());
        if let Some(limit) = self.limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        query
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticExportQuery {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub session_key: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub include_snapshots: Option<bool>,
    pub max_bytes: Option<u32>,
}

impl DiagnosticExportQuery {
    fn validate(&self) -> Result<(), ManagementError> {
        validate_common_filters(
            self.request_id.as_deref(),
            self.trace_id.as_deref(),
            self.session_key.as_deref(),
            self.since.as_deref(),
            self.until.as_deref(),
        )?;
        if self
            .max_bytes
            .is_some_and(|bytes| !(MIN_EXPORT_BYTES..=MAX_EXPORT_BYTES).contains(&bytes))
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut query = Vec::with_capacity(7);
        append_common_filters(
            &mut query,
            self.request_id.as_deref(),
            self.trace_id.as_deref(),
            self.session_key.as_deref(),
            self.since.as_deref(),
            self.until.as_deref(),
        );
        if let Some(include_snapshots) = self.include_snapshots {
            query.push((
                "include_snapshots".to_owned(),
                include_snapshots.to_string(),
            ));
        }
        if let Some(max_bytes) = self.max_bytes {
            query.push(("max_bytes".to_owned(), max_bytes.to_string()));
        }
        query
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticLogs {
    pub schema_version: u32,
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
    pub dropped_events: u64,
}

fn validate_common_filters(
    request_id: Option<&str>,
    trace_id: Option<&str>,
    session_key: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<(), ManagementError> {
    if [request_id, trace_id, session_key]
        .into_iter()
        .flatten()
        .any(|value| value.len() > MAX_FILTER_BYTES)
        || since.is_some_and(|value| !valid_utc_timestamp(value))
        || until.is_some_and(|value| !valid_utc_timestamp(value))
    {
        return Err(ManagementError::InvalidInput);
    }
    Ok(())
}

fn append_common_filters(
    query: &mut Vec<(String, String)>,
    request_id: Option<&str>,
    trace_id: Option<&str>,
    session_key: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) {
    push_optional(query, "request_id", request_id);
    push_optional(query, "trace_id", trace_id);
    push_optional(query, "session_key", session_key);
    push_optional(query, "since", since);
    push_optional(query, "until", until);
}

fn validate_logs(response: DiagnosticLogs) -> Result<DiagnosticLogs, ManagementError> {
    let valid = response.schema_version == 1
        && response.items.len() <= 1000
        && response.items.iter().all(Value::is_object)
        && response
            .next_cursor
            .as_ref()
            .is_none_or(|cursor| cursor.len() <= MAX_CURSOR_BYTES);
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}
