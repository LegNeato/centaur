//! The types and payloads of the events in a session stream.
//!
//! The server stores each event with an `event_type`. It replays the event
//! over SSE with the type as the event name. Every payload is JSON except the
//! payload of [`SessionEventType::OutputLine`]. That payload is one raw line
//! of harness output.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use strum::{AsRefStr, Display, EnumString};

use crate::ThreadKey;

/// The type of a session event. It is also the SSE event name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, AsRefStr, Display, EnumString)]
pub enum SessionEventType {
    /// One line of harness output.
    #[strum(serialize = "session.output.line")]
    OutputLine,
    #[strum(serialize = "session.execution_started")]
    ExecutionStarted,
    #[strum(serialize = "session.execution_completed")]
    ExecutionCompleted,
    #[strum(serialize = "session.execution_failed")]
    ExecutionFailed,
    #[strum(serialize = "session.execution_cancelled")]
    ExecutionCancelled,
    /// A short note on the work of a running execution. A model writes it
    /// from the recent events when the deployment enables progress updates.
    #[strum(serialize = "session.progress_update")]
    ProgressUpdate,
    /// The server stopped the stream because of a server fault.
    #[strum(serialize = "session.stream_error")]
    StreamError,
}

impl SessionEventType {
    /// Returns true when the event ends its execution. A stream of one
    /// execution has no events after such an event.
    pub const fn ends_execution(self) -> bool {
        matches!(
            self,
            Self::ExecutionCompleted | Self::ExecutionFailed | Self::ExecutionCancelled
        )
    }
}

mod sealed {
    /// Limits [`super::EventPayload`] to the payloads of this module. Their
    /// fields serialize without error.
    pub trait Sealed {}
}

/// A JSON event payload with a fixed event type.
pub trait EventPayload: Serialize + DeserializeOwned + sealed::Sealed {
    const EVENT_TYPE: SessionEventType;

    /// Returns the payload as the JSON value that the store persists.
    fn to_value(&self) -> Value {
        // Payloads hold only strings and integers. Their serialization cannot
        // fail.
        serde_json::to_value(self).expect("session event payloads serialize to JSON")
    }
}

/// The payload of [`SessionEventType::ExecutionCompleted`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionCompleted {
    pub execution_id: String,
    pub thread_key: ThreadKey,
    pub completion_reason: String,
    /// The final answer of the turn. It is absent when the turn ended without
    /// answer text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
}

impl sealed::Sealed for ExecutionCompleted {}

impl EventPayload for ExecutionCompleted {
    const EVENT_TYPE: SessionEventType = SessionEventType::ExecutionCompleted;
}

/// The payload of [`SessionEventType::ExecutionFailed`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionFailed {
    pub execution_id: String,
    pub thread_key: ThreadKey,
    pub error: String,
    /// The machine-readable cause, for example `max_duration_exceeded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
}

impl sealed::Sealed for ExecutionFailed {}

impl EventPayload for ExecutionFailed {
    const EVENT_TYPE: SessionEventType = SessionEventType::ExecutionFailed;
}

/// The payload of [`SessionEventType::ExecutionCancelled`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionCancelled {
    pub execution_id: String,
    pub thread_key: ThreadKey,
    pub reason: String,
}

impl sealed::Sealed for ExecutionCancelled {}

impl EventPayload for ExecutionCancelled {
    const EVENT_TYPE: SessionEventType = SessionEventType::ExecutionCancelled;
}

/// The payload of [`SessionEventType::ProgressUpdate`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgressUpdate {
    pub execution_id: String,
    pub text: String,
    /// The Unix time in milliseconds when the server wrote the update. A
    /// renderer that replays history can drop stale updates.
    pub created_at_ms: i64,
    /// The run time of the execution when the server wrote the update.
    pub elapsed_secs: u64,
    /// The model that wrote the update.
    pub model: String,
}

impl sealed::Sealed for ProgressUpdate {}

impl EventPayload for ProgressUpdate {
    const EVENT_TYPE: SessionEventType = SessionEventType::ProgressUpdate;
}

/// The payload of [`SessionEventType::StreamError`]. The message is generic on
/// purpose. The server writes the details to its log.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamError {
    pub error: String,
}

impl sealed::Sealed for StreamError {}

impl EventPayload for StreamError {
    const EVENT_TYPE: SessionEventType = SessionEventType::StreamError;
}

/// The end of an execution, decoded from the event that ended it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionEnd {
    Completed(ExecutionCompleted),
    Failed(ExecutionFailed),
    Cancelled(ExecutionCancelled),
}

impl ExecutionEnd {
    /// Decodes the payload of an event that ends an execution. It returns
    /// `None` for every other event type.
    pub fn decode(
        event_type: SessionEventType,
        data: &str,
    ) -> Option<Result<Self, serde_json::Error>> {
        let end = match event_type {
            SessionEventType::ExecutionCompleted => serde_json::from_str(data).map(Self::Completed),
            SessionEventType::ExecutionFailed => serde_json::from_str(data).map(Self::Failed),
            SessionEventType::ExecutionCancelled => serde_json::from_str(data).map(Self::Cancelled),
            SessionEventType::OutputLine
            | SessionEventType::ExecutionStarted
            | SessionEventType::ProgressUpdate
            | SessionEventType::StreamError => return None,
        };
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;

    use super::{
        EventPayload, ExecutionCompleted, ExecutionEnd, ExecutionFailed, SessionEventType,
    };
    use crate::ThreadKey;

    #[test]
    fn event_types_round_trip_through_their_wire_names() {
        for (event_type, name) in [
            (SessionEventType::OutputLine, "session.output.line"),
            (
                SessionEventType::ExecutionStarted,
                "session.execution_started",
            ),
            (
                SessionEventType::ExecutionCompleted,
                "session.execution_completed",
            ),
            (
                SessionEventType::ExecutionFailed,
                "session.execution_failed",
            ),
            (
                SessionEventType::ExecutionCancelled,
                "session.execution_cancelled",
            ),
            (SessionEventType::ProgressUpdate, "session.progress_update"),
            (SessionEventType::StreamError, "session.stream_error"),
        ] {
            assert_eq!(event_type.as_ref(), name);
            assert_eq!(SessionEventType::from_str(name), Ok(event_type));
        }
        assert!(SessionEventType::from_str("session.activity_summary").is_err());
    }

    #[test]
    fn only_completion_failure_and_cancellation_end_an_execution() {
        assert!(SessionEventType::ExecutionCompleted.ends_execution());
        assert!(SessionEventType::ExecutionFailed.ends_execution());
        assert!(SessionEventType::ExecutionCancelled.ends_execution());
        assert!(!SessionEventType::OutputLine.ends_execution());
        assert!(!SessionEventType::ExecutionStarted.ends_execution());
        assert!(!SessionEventType::ProgressUpdate.ends_execution());
        assert!(!SessionEventType::StreamError.ends_execution());
    }

    #[test]
    fn completed_payload_omits_missing_result_text() {
        let payload = ExecutionCompleted {
            execution_id: "exe-1".to_owned(),
            thread_key: ThreadKey::parse("cli:test").unwrap(),
            completion_reason: "turn_completed".to_owned(),
            result_text: None,
        };
        assert_eq!(
            payload.to_value(),
            json!({
                "execution_id": "exe-1",
                "thread_key": "cli:test",
                "completion_reason": "turn_completed",
            })
        );
    }

    #[test]
    fn decode_reads_terminal_payloads() {
        let completed = ExecutionEnd::decode(
            SessionEventType::ExecutionCompleted,
            r#"{"execution_id":"exe-1","thread_key":"cli:test","completion_reason":"turn_completed","result_text":"PONG"}"#,
        );
        let Some(Ok(ExecutionEnd::Completed(completed))) = completed else {
            panic!("expected a completed execution, got {completed:?}");
        };
        assert_eq!(completed.result_text.as_deref(), Some("PONG"));

        let failed = ExecutionEnd::decode(
            SessionEventType::ExecutionFailed,
            r#"{"execution_id":"exe-1","thread_key":"cli:test","error":"boom","reason":"max_duration_exceeded","max_duration_ms":10}"#,
        );
        assert_eq!(
            failed.unwrap().unwrap(),
            ExecutionEnd::Failed(ExecutionFailed {
                execution_id: "exe-1".to_owned(),
                thread_key: ThreadKey::parse("cli:test").unwrap(),
                error: "boom".to_owned(),
                reason: Some("max_duration_exceeded".to_owned()),
                max_duration_ms: Some(10),
            })
        );
    }

    #[test]
    fn decode_ignores_events_that_do_not_end_an_execution() {
        assert!(ExecutionEnd::decode(SessionEventType::OutputLine, "{}").is_none());
        assert!(ExecutionEnd::decode(SessionEventType::StreamError, "{}").is_none());
    }

    #[test]
    fn decode_rejects_malformed_terminal_payloads() {
        let decoded = ExecutionEnd::decode(SessionEventType::ExecutionCancelled, r#"{"reason":1}"#);
        assert!(matches!(decoded, Some(Err(_))));
    }
}
