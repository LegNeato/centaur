use axum::response::sse::Event;
use centaur_session_core::{
    SessionEvent, ThreadKey,
    destination::ChatDestination,
    event::{EventPayload, SessionEventType, StreamError},
};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;
use thiserror::Error;

/// The body of the `GET /api/session/{thread_key}` response.
#[derive(Clone, Debug, Serialize)]
pub struct SessionContextResponse {
    pub thread_key: ThreadKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The chat surface of the thread. It adds `platform` and the object of
    /// the platform. A thread that no chat platform can address gets
    /// `"platform": "unknown"`.
    #[serde(flatten, serialize_with = "serialize_destination")]
    pub destination: Option<ChatDestination>,
}

fn serialize_destination<S: Serializer>(
    destination: &Option<ChatDestination>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match destination {
        Some(destination) => destination.serialize(serializer),
        None => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("platform", "unknown")?;
            map.end()
        }
    }
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
