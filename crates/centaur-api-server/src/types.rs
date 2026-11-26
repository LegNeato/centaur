use axum::response::sse::Event;
use centaur_session_core::{
    SessionEvent, ThreadKey,
    event::{EventPayload, SessionEventType, StreamError},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Serialize)]
pub struct SessionContextResponse {
    pub thread_key: ThreadKey,
    /// The chat surface the agent is operating on: `slack`, `discord`, `linear`,
    /// `github`, or `unknown` for threads that are not platform-addressable.
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slack: Option<SlackThreadContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discord: Option<DiscordThreadContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linear: Option<LinearThreadContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubThreadContext>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SlackThreadContext {
    pub channel_id: String,
    pub thread_ts: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiscordThreadContext {
    pub guild_id: String,
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LinearThreadContext {
    pub issue_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GithubThreadContext {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    /// `issue` or `pr`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_comment_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListWorkflowRunsQuery {
    pub limit: Option<i64>,
    pub workflow_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmitWorkflowEventRequest {
    pub event_name: Option<String>,
    pub event_type: Option<String>,
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

pub struct SessionSseEvent(Event);

impl TryFrom<SessionEvent> for SessionSseEvent {
    type Error = SessionEventConversionError;

    fn try_from(event: SessionEvent) -> Result<Self, Self::Error> {
        let event_id = event.event_id;
        let sse = Event::default()
            .id(event_id.to_string())
            .event(&event.event_type);

        // Output lines are raw harness lines. Every other payload is JSON.
        let sse = if event.event_type == SessionEventType::OutputLine.as_ref() {
            let Some(line) = event.payload.as_str() else {
                return Err(SessionEventConversionError::OutputLinePayload { event_id });
            };
            sse.data(line)
        } else {
            sse.json_data(event.payload)
                .map_err(|source| SessionEventConversionError::JsonData { event_id, source })?
        };

        Ok(Self(sse))
    }
}

impl From<SessionSseEvent> for Event {
    fn from(value: SessionSseEvent) -> Self {
        value.0
    }
}

pub fn stream_error_sse(message: impl Into<String>) -> Event {
    let payload = StreamError {
        error: message.into(),
    };
    Event::default()
        .event(StreamError::EVENT_TYPE)
        .json_data(payload.to_value())
        .unwrap_or_else(|_| Event::default().event(StreamError::EVENT_TYPE).data("{}"))
}

#[derive(Debug, Error)]
pub enum SessionEventConversionError {
    #[error("session.output.line event {event_id} payload must be a string")]
    OutputLinePayload { event_id: i64 },
    #[error("failed to serialize session event {event_id} payload as SSE JSON: {source}")]
    JsonData { event_id: i64, source: axum::Error },
}
