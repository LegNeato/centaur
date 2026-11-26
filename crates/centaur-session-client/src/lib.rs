//! The HTTP client of the Centaur session API.
//!
//! Chat ingresses and the session CLI use the client to create sessions,
//! append messages, start executions, and replay session events.
//! [`centaur_session_core::api`] defines the request and response bodies.

use std::{pin::Pin, time::Duration};

use centaur_session_core::{
    HarnessType, ThreadKey,
    api::{
        ApiErrorBody, ApiErrorCode, AppendMessagesRequest, AppendMessagesResponse,
        CreateSessionRequest, CreateSessionResponse, EventsQuery, ExecuteSessionRequest,
        ExecuteSessionResponse, InterruptSessionExecutionRequest,
        InterruptSessionExecutionResponse,
    },
    event::{ExecutionEnd, ProgressUpdate, SessionEventType},
};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt};
use reqwest::{
    Client as HttpClient, StatusCode,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use url::Url;

pub use eventsource_stream::Event as SseEvent;

/// The replayed and live events of a session in event ID order.
pub type SseEventStream = Pin<Box<dyn Stream<Item = Result<SseEvent, ClientError>> + Send>>;

/// The time limit of a new connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The idle time before TCP probes a connection. The probes find a dead
/// server under a quiet event stream.
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
/// The time limit of a request with a JSON response. An execute request can
/// wait for a cold sandbox to start.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct SessionClient {
    http: HttpClient,
    /// A URL that can hold a path. The client appends path segments to it.
    base_url: Url,
}

impl SessionClient {
    pub fn new(base_url: Url) -> Result<Self, ClientError> {
        Self::with_headers(base_url, HeaderMap::new())
    }

    /// Returns a client that sends `Authorization: Bearer <token>` with every
    /// request.
    pub fn with_bearer_token(base_url: Url, token: &str) -> Result<Self, ClientError> {
        let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| ClientError::InvalidBearerToken)?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        Self::with_headers(base_url, headers)
    }

    /// Returns a client that uses `http`. The time limits of `http` apply to
    /// the event stream.
    pub fn with_http_client(http: HttpClient, base_url: Url) -> Result<Self, ClientError> {
        if base_url.cannot_be_a_base() {
            return Err(ClientError::InvalidBaseUrl(base_url));
        }
        Ok(Self { http, base_url })
    }

    fn with_headers(base_url: Url, headers: HeaderMap) -> Result<Self, ClientError> {
        let http = HttpClient::builder()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(TCP_KEEPALIVE)
            .build()?;
        Self::with_http_client(http, base_url)
    }

    /// Creates the session or returns the existing session.
    pub async fn create_session(
        &self,
        thread_key: &ThreadKey,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse, ClientError> {
        self.post_json(self.session_url(thread_key, None), request)
            .await
    }

    pub async fn append_messages(
        &self,
        thread_key: &ThreadKey,
        request: &AppendMessagesRequest,
    ) -> Result<AppendMessagesResponse, ClientError> {
        self.post_json(self.session_url(thread_key, Some("messages")), request)
            .await
    }

    pub async fn execute_session(
        &self,
        thread_key: &ThreadKey,
        request: &ExecuteSessionRequest,
    ) -> Result<ExecuteSessionResponse, ClientError> {
        self.post_json(self.session_url(thread_key, Some("execute")), request)
            .await
    }

    /// Interrupts the active execution of the session. The execution ends with
    /// a cancellation event that carries the reason.
    pub async fn interrupt_execution(
        &self,
        thread_key: &ThreadKey,
        request: &InterruptSessionExecutionRequest,
    ) -> Result<InterruptSessionExecutionResponse, ClientError> {
        self.post_json(self.session_url(thread_key, Some("interrupt")), request)
            .await
    }

    /// Opens the event stream of the session. The server first replays the
    /// stored events that match `query`. Then it sends each new event.
    pub async fn stream_events(
        &self,
        thread_key: &ThreadKey,
        query: &EventsQuery,
    ) -> Result<SseEventStream, ClientError> {
        let response = self
            .http
            .get(self.session_url(thread_key, Some("events")))
            .query(query)
            .send()
            .await?;
        let stream = ensure_success(response)
            .await?
            .bytes_stream()
            .eventsource()
            .map(|event| event.map_err(ClientError::from));
        Ok(Box::pin(stream))
    }

    /// Waits for the event that ends `execution_id` and returns the decoded
    /// end. The client passes each progress update before the end to
    /// `on_progress` and skips a malformed update. The client reopens a stream
    /// that drops, ends, or reports a server fault. The new stream starts
    /// after the last delivered event. `reconnect` limits the reconnects.
    pub async fn wait_for_execution_end(
        &self,
        thread_key: &ThreadKey,
        execution_id: &str,
        reconnect: Reconnect,
        mut on_progress: impl FnMut(ProgressUpdate) + Send,
    ) -> Result<ExecutionEnd, ClientError> {
        let mut query = EventsQuery {
            after_event_id: None,
            execution_id: Some(execution_id.to_owned()),
        };
        // The number of consecutive attempts that delivered no stored event.
        let mut failures = 0;
        loop {
            let (interruption, progressed) = match self.stream_events(thread_key, &query).await {
                Ok(events) => match follow(events, &mut query, &mut on_progress).await {
                    Followed::End(end) => return end,
                    Followed::Interrupted { error, progressed } => (error, progressed),
                },
                Err(error) => (error, false),
            };
            if progressed {
                failures = 0;
            }
            if !interruption.is_retryable() || failures >= reconnect.max_reconnects {
                return Err(interruption);
            }
            tokio::time::sleep(reconnect.delay(failures)).await;
            if !progressed {
                failures += 1;
            }
        }
    }

    async fn post_json<T, R>(&self, url: Url, payload: &T) -> Result<R, ClientError>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let response = self
            .http
            .post(url)
            .json(payload)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        Ok(ensure_success(response).await?.json().await?)
    }

    fn session_url(&self, thread_key: &ThreadKey, action: Option<&str>) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("the constructors accept only a base URL")
            .pop_if_empty()
            .extend(["api", "session", thread_key.as_str()])
            .extend(action);
        url
    }
}

/// How one event stream ended for a wait.
enum Followed {
    End(Result<ExecutionEnd, ClientError>),
    /// The stream stopped before the end. `progressed` is true when the stream
    /// delivered a stored event.
    Interrupted {
        error: ClientError,
        progressed: bool,
    },
}

/// Reads `events` until the end of the execution or an interruption. Each
/// stored event moves `query` past it.
async fn follow(
    mut events: SseEventStream,
    query: &mut EventsQuery,
    on_progress: &mut (impl FnMut(ProgressUpdate) + Send),
) -> Followed {
    let mut progressed = false;
    let error = loop {
        let event = match events.next().await {
            Some(Ok(event)) => event,
            Some(Err(error)) => break error,
            None => break ClientError::StreamEnded,
        };
        // Only stored events carry an ID. A stream error frame has none.
        if let Ok(event_id) = event.id.parse() {
            query.after_event_id = Some(event_id);
            progressed = true;
        }
        let Ok(event_type) = event.event.parse::<SessionEventType>() else {
            continue;
        };
        match event_type {
            SessionEventType::StreamError => break ClientError::StreamEnded,
            SessionEventType::ProgressUpdate => {
                if let Ok(update) = serde_json::from_str(&event.data) {
                    on_progress(update);
                }
            }
            _ => {
                if let Some(end) = ExecutionEnd::decode(event_type, &event.data) {
                    return Followed::End(
                        end.map_err(|source| ClientError::MalformedEvent { event_type, source }),
                    );
                }
            }
        }
    };
    Followed::Interrupted { error, progressed }
}

/// The policy that [`SessionClient::wait_for_execution_end`] uses to reopen
/// an interrupted event stream. The delay doubles after each failed attempt
/// until it reaches `max_delay`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reconnect {
    /// The maximum number of reconnects after consecutive failed attempts.
    /// The wait then returns the last error. An attempt that delivers a
    /// stored event resets the count.
    pub max_reconnects: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Reconnect {
    fn delay(self, failures: u32) -> Duration {
        self.initial_delay
            .saturating_mul(2_u32.saturating_pow(failures))
            .min(self.max_delay)
    }
}

impl Default for Reconnect {
    /// The delays add up to about three and a half minutes. The wait then
    /// fails for a server that stays unreachable.
    fn default() -> Self {
        Self {
            max_reconnects: 12,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await?;
    Err(ClientError::api(status, body))
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    /// The server answered with an error status. The body can hold server
    /// internals. Callers must log it and must not show it to users.
    #[error("HTTP {status}: {body}")]
    Api {
        status: StatusCode,
        body: String,
        /// The parsed body. A proxy in front of api-rs can send another body.
        detail: Option<ApiErrorBody>,
    },
    #[error("the base URL {0} cannot hold an API path")]
    InvalidBaseUrl(Url),
    #[error("event stream failed: {0}")]
    EventStream(#[from] EventStreamError<reqwest::Error>),
    #[error("bearer token cannot be represented as an HTTP header")]
    InvalidBearerToken,
    /// The server closed the event stream or reported a server fault before
    /// the awaited event.
    #[error("event stream ended before the execution ended")]
    StreamEnded,
    #[error("{event_type} event payload is malformed: {source}")]
    MalformedEvent {
        event_type: SessionEventType,
        source: serde_json::Error,
    },
}

impl ClientError {
    /// Returns the error of a response with an error status.
    pub fn api(status: StatusCode, body: String) -> Self {
        let detail = serde_json::from_str(&body).ok();
        Self::Api {
            status,
            body,
            detail,
        }
    }

    /// Returns the `code` of an error response.
    pub fn api_code(&self) -> Option<ApiErrorCode> {
        self.detail()?.code
    }

    /// Returns the harness of the existing session in a harness conflict.
    pub fn existing_harness(&self) -> Option<HarnessType> {
        let detail = self.detail()?;
        (detail.code == Some(ApiErrorCode::HarnessConflict))
            .then_some(detail.existing_harness)
            .flatten()
    }

    fn detail(&self) -> Option<&ApiErrorBody> {
        match self {
            Self::Api { detail, .. } => detail.as_ref(),
            _ => None,
        }
    }

    /// Returns true when the same request can succeed later.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(error) => error.is_connect() || error.is_timeout() || error.is_request(),
            Self::Api { status, .. } => {
                status.is_server_error()
                    || matches!(
                        *status,
                        StatusCode::REQUEST_TIMEOUT
                            | StatusCode::TOO_EARLY
                            | StatusCode::TOO_MANY_REQUESTS
                    )
            }
            Self::EventStream(EventStreamError::Transport(_)) | Self::StreamEnded => true,
            Self::EventStream(_)
            | Self::InvalidBearerToken
            | Self::InvalidBaseUrl(_)
            | Self::MalformedEvent { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests;
