//! The HTTP client of the Centaur session API.
//!
//! Chat ingresses and the session CLI use the client to create sessions,
//! append messages, start executions, and replay session events.
//! [`centaur_session_core::api`] defines the request and response bodies.

use std::{pin::Pin, time::Duration};

use centaur_session_core::{
    ThreadKey,
    api::{
        AppendMessagesRequest, AppendMessagesResponse, CreateSessionRequest, CreateSessionResponse,
        EventsQuery, ExecuteSessionRequest, ExecuteSessionResponse,
    },
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

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await?;
    Err(ClientError::Api { status, body })
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    /// The server answered with an error status. The body can hold server
    /// internals. Callers must log it and must not show it to users.
    #[error("HTTP {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("the base URL {0} cannot hold an API path")]
    InvalidBaseUrl(Url),
    #[error("event stream failed: {0}")]
    EventStream(#[from] EventStreamError<reqwest::Error>),
    #[error("bearer token cannot be represented as an HTTP header")]
    InvalidBearerToken,
}

impl ClientError {
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
            Self::EventStream(EventStreamError::Transport(_)) => true,
            Self::EventStream(_) | Self::InvalidBearerToken | Self::InvalidBaseUrl(_) => {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests;
