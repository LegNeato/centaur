//! The request and response bodies of the session HTTP API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ExecutionStatus, HarnessType, Session, SessionMessageInput, ThreadKey};

/// The body of `POST /api/session/{thread_key}`. The request creates the
/// session or returns the existing session.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateSessionRequest {
    pub harness_type: HarnessType,
    /// The persona of a new session. The server ignores it for an existing
    /// session because the first persona stays pinned to the thread. An ID
    /// that the deployment does not define selects the eligible deployment
    /// default or no persona.
    pub persona_id: Option<String>,
    pub metadata: Option<Value>,
    /// The action when the session exists on a different harness.
    #[serde(default)]
    pub on_harness_conflict: OnHarnessConflict,
    /// The principal of a new session, as an OID or a foreign ID. Only API
    /// clients can set it. The principal must belong to the client. The
    /// client's default principal applies when it is not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OnHarnessConflict {
    /// Fails the request with 409.
    #[default]
    Reject,
    /// Stops the old sandbox and restarts the thread on the requested
    /// harness. The new harness starts with no conversation history.
    Restart,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateSessionResponse {
    #[serde(flatten)]
    pub session: Session,
    /// True when the request restarted the thread on a different harness.
    pub harness_switched: bool,
    /// The unavailable persona that a new-session request named. The returned
    /// session then uses the fallback persona.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_requested_persona_id: Option<String>,
    /// The model that the session's persona pins for its harness. Every turn
    /// runs on this model. The server ignores requested models and providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_model: Option<String>,
}

/// The body of `POST /api/session/{thread_key}/messages`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppendMessagesRequest {
    pub messages: Vec<SessionMessageInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppendMessagesResponse {
    pub ok: bool,
    pub message_ids: Vec<String>,
    /// The result of steering the active execution with the appended user
    /// messages. It is absent when the append stored no new user message. A
    /// repeated client message ID stores nothing and does not steer again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<SteeringOutcome>,
}

/// The delivery state of a steer to a running execution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SteeringStatus {
    /// The server wrote the messages to the sandbox input of the active
    /// execution.
    Delivered,
    /// No execution was queued or running. No harness received the messages.
    NoActiveExecution,
    /// The server wrote nothing. The lookup failed, the sandbox had no input
    /// pipe, or the execution ended first. No harness received the messages.
    NotDelivered,
    /// The write to the sandbox failed partway or with an ambiguous transport
    /// error. The harness can have received some or all of the messages.
    DeliveryUnknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SteeringOutcome {
    pub status: SteeringStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
}

/// The body of `POST /api/session/{thread_key}/execute`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ExecuteSessionRequest {
    /// The key that makes the request idempotent. Every request of one
    /// thread with the same key gets the same execution.
    pub idempotency_key: Option<String>,
    pub metadata: Option<Value>,
    #[serde(default)]
    pub input_lines: Vec<String>,
    pub idle_timeout_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecuteSessionResponse {
    pub ok: bool,
    pub execution_id: String,
    pub thread_key: ThreadKey,
    pub status: ExecutionStatus,
}

/// The body of `POST /api/session/{thread_key}/interrupt`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InterruptSessionExecutionRequest {
    /// The text of the cancellation event. A blank reason gets a default.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InterruptSessionExecutionResponse {
    pub ok: bool,
    /// False when the session has no active execution.
    pub interrupted: bool,
    pub execution_id: Option<String>,
    pub thread_key: ThreadKey,
}

/// The body of a session API error response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApiErrorBody {
    /// Always false.
    #[serde(default)]
    pub ok: bool,
    /// The message for the caller. A server fault has a generic message.
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ApiErrorCode>,
    /// The harness of the existing session in a harness conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_harness: Option<HarnessType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_harness: Option<HarnessType>,
    /// The principal of the existing session in a principal conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_principal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_principal: Option<String>,
}

impl ApiErrorBody {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            ..Self::default()
        }
    }
}

/// The `code` field of a session API error response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// The session exists on another harness.
    HarnessConflict,
    /// The session runs as a principal other than the requested principal.
    PrincipalConflict,
    /// The deployment admits only preapproved principals. An operator has not
    /// approved the session's principal.
    SessionPrincipalNotPreapproved,
    /// A code that this version does not know. A newer server can add codes.
    #[serde(other)]
    Other,
}

/// The query of `GET /api/session/{thread_key}/events`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EventsQuery {
    /// The stream replays only the events with a larger ID.
    pub after_event_id: Option<i64>,
    /// The stream sends only the events of this execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
}
