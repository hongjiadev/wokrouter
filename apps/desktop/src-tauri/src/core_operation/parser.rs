use semver::Version;
use serde::Deserialize;

use super::{CoreOperationKind, CoreOperationPhase, CoreOperationState};

pub(super) const MAX_LINE_BYTES: usize = 16 * 1024;
pub(super) const MAX_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ACTIVE_REQUESTS: u64 = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(super) struct ChildProgress {
    pub schema_version: u8,
    pub sequence: u64,
    pub operation: CoreOperationKind,
    pub state: CoreOperationState,
    pub phase: CoreOperationPhase,
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub bytes_completed: Option<u64>,
    #[serde(default)]
    pub bytes_total: Option<u64>,
    #[serde(default)]
    pub active_requests: Option<u64>,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProgressParseError;

pub(super) struct ProgressParser {
    operation: CoreOperationKind,
    next_sequence: u64,
    terminal: bool,
    download: Option<DownloadState>,
    buffer: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DownloadState {
    bytes_completed: u64,
    bytes_total: u64,
}

impl ProgressParser {
    pub(super) fn new(operation: CoreOperationKind) -> Self {
        Self {
            operation,
            next_sequence: 0,
            terminal: false,
            download: None,
            buffer: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<ChildProgress>, ProgressParseError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFER_BYTES {
            return Err(ProgressParseError);
        }
        self.buffer.extend_from_slice(bytes);

        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            if newline > MAX_LINE_BYTES {
                return Err(ProgressParseError);
            }
            let line = self.buffer[..newline].to_vec();
            self.buffer.drain(..=newline);
            events.push(self.parse_line(&line)?);
        }
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err(ProgressParseError);
        }
        Ok(events)
    }

    pub(super) fn finish(&self) -> Result<(), ProgressParseError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(ProgressParseError)
        }
    }

    fn parse_line(&mut self, line: &[u8]) -> Result<ChildProgress, ProgressParseError> {
        if self.terminal || line.is_empty() || line.len() > MAX_LINE_BYTES {
            return Err(ProgressParseError);
        }
        let value =
            serde_json::from_slice::<serde_json::Value>(line).map_err(|_| ProgressParseError)?;
        let schema_version = value
            .as_object()
            .and_then(|object| object.get("schema_version"))
            .and_then(serde_json::Value::as_u64)
            .ok_or(ProgressParseError)?;
        if schema_version != 1 {
            return Err(ProgressParseError);
        }
        let event =
            serde_json::from_value::<ChildProgress>(value).map_err(|_| ProgressParseError)?;
        self.validate(&event)?;
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(ProgressParseError)?;
        if let (CoreOperationPhase::Downloading, Some(bytes_completed), Some(bytes_total)) =
            (event.phase, event.bytes_completed, event.bytes_total)
        {
            self.download = Some(DownloadState {
                bytes_completed,
                bytes_total,
            });
        }
        self.next_sequence = next_sequence;
        self.terminal = event.state != CoreOperationState::Running;
        Ok(event)
    }

    fn validate(&self, event: &ChildProgress) -> Result<(), ProgressParseError> {
        if event.schema_version != 1
            || event.operation != self.operation
            || event.sequence != self.next_sequence
            || !phase_is_valid(event.operation, event.phase)
            || !state_is_valid(event.state, event.phase, event.error_code.as_deref())
            || !versions_are_valid(event)
            || !self.bytes_are_valid(event)
            || !active_requests_are_valid(event)
            || !error_code_is_valid(event)
        {
            return Err(ProgressParseError);
        }
        Ok(())
    }

    fn bytes_are_valid(&self, event: &ChildProgress) -> bool {
        match (event.phase, event.bytes_completed, event.bytes_total) {
            (CoreOperationPhase::Downloading, Some(completed), Some(total)) => {
                total > 0
                    && completed <= total
                    && self.download.is_none_or(|previous| {
                        previous.bytes_total == total && previous.bytes_completed <= completed
                    })
            }
            (CoreOperationPhase::Downloading, _, _) => false,
            (_, None, None) => true,
            (_, _, _) => false,
        }
    }
}

fn phase_is_valid(operation: CoreOperationKind, phase: CoreOperationPhase) -> bool {
    match operation {
        CoreOperationKind::Install => matches!(
            phase,
            CoreOperationPhase::CheckingRelease
                | CoreOperationPhase::Downloading
                | CoreOperationPhase::Verifying
                | CoreOperationPhase::Installing
                | CoreOperationPhase::Starting
                | CoreOperationPhase::Authorizing
                | CoreOperationPhase::VerifyingRuntime
                | CoreOperationPhase::Completed
        ),
        CoreOperationKind::Update => matches!(
            phase,
            CoreOperationPhase::CheckingRelease
                | CoreOperationPhase::Downloading
                | CoreOperationPhase::Verifying
                | CoreOperationPhase::Installing
                | CoreOperationPhase::PreparingService
                | CoreOperationPhase::Draining
                | CoreOperationPhase::Stopping
                | CoreOperationPhase::Starting
                | CoreOperationPhase::VerifyingRuntime
                | CoreOperationPhase::RollingBack
                | CoreOperationPhase::Completed
        ),
    }
}

fn state_is_valid(
    state: CoreOperationState,
    phase: CoreOperationPhase,
    error_code: Option<&str>,
) -> bool {
    match state {
        CoreOperationState::Running => {
            phase != CoreOperationPhase::Completed && error_code.is_none()
        }
        CoreOperationState::Succeeded => {
            phase == CoreOperationPhase::Completed && error_code.is_none()
        }
        CoreOperationState::Failed => error_code.is_some(),
    }
}

fn versions_are_valid(event: &ChildProgress) -> bool {
    [&event.current_version, &event.target_version]
        .into_iter()
        .flatten()
        .all(|version| Version::parse(version).is_ok())
}

fn active_requests_are_valid(event: &ChildProgress) -> bool {
    match event.active_requests {
        None => true,
        Some(count) => count <= MAX_ACTIVE_REQUESTS && event.operation == CoreOperationKind::Update,
    }
}

fn error_code_is_valid(event: &ChildProgress) -> bool {
    let Some(error_code) = event.error_code.as_deref() else {
        return event.state != CoreOperationState::Failed;
    };
    if event.state != CoreOperationState::Failed {
        return false;
    }
    match event.operation {
        CoreOperationKind::Install => matches!(
            error_code,
            "download_failed"
                | "invalid_install_state"
                | "invalid_manifest"
                | "invalid_signature"
                | "incompatible_manifest"
                | "artifact_size_mismatch"
                | "artifact_hash_mismatch"
                | "invalid_archive"
                | "unsafe_install_location"
                | "install_in_progress"
                | "install_failed"
                | "install_record_failed"
                | "start_failed"
                | "authorization_failed"
        ),
        CoreOperationKind::Update => matches!(
            error_code,
            "update_unavailable"
                | "incompatible_manifest"
                | "update_verification_failed"
                | "update_install_failed"
                | "active_requests_remain"
                | "rolled_back"
                | "recovery_required"
                | "operation_in_progress"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_BUFFER_BYTES, MAX_LINE_BYTES, ProgressParser};
    use crate::core_operation::{CoreOperationKind, CoreOperationPhase, CoreOperationState};

    const VALID_DOWNLOAD: &str = r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"downloading","target_version":"0.1.23","bytes_completed":1,"bytes_total":2}"#;

    #[test]
    fn valid_download_and_same_schema_optional_fields_are_accepted() {
        let event = parse_one(
            CoreOperationKind::Install,
            &VALID_DOWNLOAD.replace(
                r#","bytes_completed""#,
                r#","future_optional":{"ignored":true},"bytes_completed""#,
            ),
        )
        .unwrap();

        assert_eq!(event.sequence, 0);
        assert_eq!(event.operation, CoreOperationKind::Install);
        assert_eq!(event.state, CoreOperationState::Running);
        assert_eq!(event.phase, CoreOperationPhase::Downloading);
        assert_eq!(event.target_version.as_deref(), Some("0.1.23"));
        assert_eq!(event.bytes_completed, Some(1));
        assert_eq!(event.bytes_total, Some(2));
    }

    #[test]
    fn invalid_schema_bytes_and_terminal_shapes_are_rejected() {
        for line in [
            r#"{"schema_version":2,"sequence":0,"operation":"install","state":"running","phase":"downloading","bytes_completed":1,"bytes_total":2}"#,
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"downloading","bytes_completed":3,"bytes_total":2}"#,
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"succeeded","phase":"starting"}"#,
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"failed","phase":"completed"}"#,
        ] {
            assert!(
                parse_one(CoreOperationKind::Install, line).is_err(),
                "unexpectedly accepted {line}"
            );
        }
    }

    #[test]
    fn line_and_stream_buffers_are_rejected_above_their_limits() {
        let accepted = event_with_size(MAX_LINE_BYTES);
        assert_eq!(accepted.len(), MAX_LINE_BYTES);
        assert!(parse_one(CoreOperationKind::Install, &accepted).is_ok());

        let oversized = event_with_size(MAX_LINE_BYTES + 1);
        assert_eq!(oversized.len(), MAX_LINE_BYTES + 1);
        assert!(parse_one(CoreOperationKind::Install, &oversized).is_err());

        let mut parser = ProgressParser::new(CoreOperationKind::Install);
        assert!(parser.push(&vec![b'x'; MAX_BUFFER_BYTES + 1]).is_err());
    }

    #[test]
    fn sequence_must_start_at_zero_and_increase_without_gaps() {
        let mut parser = ProgressParser::new(CoreOperationKind::Install);
        assert!(
            parser
                .push(format!("{VALID_DOWNLOAD}\n").as_bytes())
                .is_ok()
        );
        assert!(
            parser
                .push(format!("{VALID_DOWNLOAD}\n").as_bytes())
                .is_err()
        );

        let mut skipped = ProgressParser::new(CoreOperationKind::Install);
        assert!(
            skipped
                .push(
                    format!(
                        "{}\n",
                        VALID_DOWNLOAD.replace(r#""sequence":0"#, r#""sequence":1"#)
                    )
                    .as_bytes()
                )
                .is_err()
        );
    }

    #[test]
    fn no_progress_is_accepted_after_a_terminal_event() {
        let mut parser = ProgressParser::new(CoreOperationKind::Install);
        assert!(
            parser
                .push(
                    br#"{"schema_version":1,"sequence":0,"operation":"install","state":"failed","phase":"starting","error_code":"start_failed"}
"#,
                )
                .is_ok()
        );
        assert!(
            parser
                .push(
                    br#"{"schema_version":1,"sequence":1,"operation":"install","state":"running","phase":"verifying_runtime"}
"#,
                )
                .is_err()
        );
    }

    #[test]
    fn versions_bytes_and_active_requests_are_strictly_validated() {
        for line in [
            VALID_DOWNLOAD.replace("0.1.23", "not-semver"),
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"starting","bytes_completed":1,"bytes_total":2}"#.to_owned(),
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"downloading","bytes_completed":1}"#.to_owned(),
            r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"installing","active_requests":1}"#.to_owned(),
            r#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"draining","active_requests":1000001}"#.to_owned(),
        ] {
            let operation = if line.contains(r#""operation":"update""#) {
                CoreOperationKind::Update
            } else {
                CoreOperationKind::Install
            };
            assert!(parse_one(operation, &line).is_err(), "accepted {line}");
        }

        let event = parse_one(
            CoreOperationKind::Update,
            r#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"draining","active_requests":1000000}"#,
        )
        .unwrap();
        assert_eq!(event.active_requests, Some(1_000_000));
    }

    #[test]
    fn downloading_requires_a_positive_total() {
        assert!(
            parse_one(
                CoreOperationKind::Update,
                r#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"downloading","bytes_completed":0,"bytes_total":0}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn download_progress_never_retreats_and_keeps_one_total() {
        for invalid_second in [
            r#"{"schema_version":1,"sequence":1,"operation":"update","state":"running","phase":"downloading","bytes_completed":4,"bytes_total":10}"#,
            r#"{"schema_version":1,"sequence":1,"operation":"update","state":"running","phase":"downloading","bytes_completed":6,"bytes_total":11}"#,
        ] {
            let mut parser = ProgressParser::new(CoreOperationKind::Update);
            assert!(
                parser
                    .push(
                        br#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"downloading","bytes_completed":5,"bytes_total":10}
"#,
                    )
                    .is_ok()
            );
            assert!(
                parser
                    .push(format!("{invalid_second}\n").as_bytes())
                    .is_err(),
                "accepted {invalid_second}"
            );
        }
    }

    #[test]
    fn invalid_download_events_do_not_advance_download_state() {
        let mut parser = ProgressParser::new(CoreOperationKind::Update);
        assert!(
            parser
                .push(
                    br#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"downloading","bytes_completed":5,"bytes_total":10}
"#,
                )
                .is_ok()
        );
        assert!(
            parser
                .push(
                    br#"{"schema_version":1,"sequence":1,"operation":"update","state":"running","phase":"downloading","bytes_completed":9,"bytes_total":10,"active_requests":1000001}
"#,
                )
                .is_err()
        );
        assert!(
            parser
                .push(
                    br#"{"schema_version":1,"sequence":1,"operation":"update","state":"running","phase":"downloading","bytes_completed":6,"bytes_total":10}
"#,
                )
                .is_ok()
        );
    }

    #[test]
    fn update_active_requests_are_valid_during_rolling_back() {
        let event = parse_one(
            CoreOperationKind::Update,
            r#"{"schema_version":1,"sequence":0,"operation":"update","state":"failed","phase":"rolling_back","active_requests":2,"error_code":"update_install_failed"}"#,
        )
        .unwrap();

        assert_eq!(event.state, CoreOperationState::Failed);
        assert_eq!(event.active_requests, Some(2));
    }

    #[test]
    fn operation_and_phase_must_match_the_active_command() {
        assert!(parse_one(CoreOperationKind::Update, VALID_DOWNLOAD).is_err());
        assert!(
            parse_one(
                CoreOperationKind::Update,
                r#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"authorizing"}"#,
            )
            .is_err()
        );
        assert!(
            parse_one(
                CoreOperationKind::Install,
                r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"draining"}"#,
            )
            .is_err()
        );
    }

    fn parse_one(
        operation: CoreOperationKind,
        line: &str,
    ) -> Result<super::ChildProgress, super::ProgressParseError> {
        let mut parser = ProgressParser::new(operation);
        let events = parser.push(format!("{line}\n").as_bytes())?;
        parser.finish()?;
        assert_eq!(events.len(), 1);
        Ok(events.into_iter().next().unwrap())
    }

    fn event_with_size(size: usize) -> String {
        let baseline = VALID_DOWNLOAD.replace(
            r#","bytes_completed""#,
            r#","future_optional":"","bytes_completed""#,
        );
        assert!(baseline.len() <= size);
        baseline.replace(
            r#""future_optional":"""#,
            &format!(
                r#""future_optional":"{}""#,
                "x".repeat(size - baseline.len())
            ),
        )
    }
}
