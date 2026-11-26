use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

/// The `code` of a Zulip error response. Centaur handles the codes below. It
/// handles every other code as [`ErrorCode::Other`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// A general request error. Zulip uses it for a channel that the bot
    /// cannot see or cannot join.
    BadRequest,
    /// The event queue expired or does not exist. The client must register a
    /// new queue.
    BadEventQueueId,
    RateLimitHit,
    ReactionAlreadyExists,
    ReactionDoesNotExist,
    #[serde(other)]
    Other,
}

#[derive(Debug, Error)]
pub enum ZulipError {
    #[error("the Zulip site {0} cannot hold an API path")]
    InvalidSite(Url),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("Zulip returned HTTP {status} ({code:?}): {message}")]
    Api {
        status: StatusCode,
        code: ErrorCode,
        message: String,
        /// The wait that the server requests after a rate limit.
        retry_after: Option<Duration>,
    },
    /// An error status without a Zulip error body. A proxy in front of Zulip
    /// can send an HTML error page. The upload routes also answer with HTML.
    #[error("Zulip returned HTTP {status} without a Zulip error body")]
    UnexpectedStatus { status: StatusCode },
    #[error("Zulip response could not be parsed: {0}")]
    MalformedResponse(#[source] serde_json::Error),
    #[error("Zulip server feature level {actual} is older than the required {required}")]
    UnsupportedServer { actual: u64, required: u64 },
    #[error("the upload is larger than the {limit}-byte download limit")]
    UploadTooLarge { limit: u64 },
    #[error("Zulip returned an invalid upload path: {0:?}")]
    InvalidUploadPath(String),
}

impl ZulipError {
    /// Returns the error of a response with an error status.
    pub(crate) fn from_response(status: StatusCode, body: &[u8]) -> Self {
        match serde_json::from_slice::<ErrorBody>(body) {
            Ok(error) => Self::Api {
                status,
                code: error.code,
                message: error.msg,
                retry_after: error
                    .retry_after
                    .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
            },
            Err(_) => Self::UnexpectedStatus { status },
        }
    }

    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Api { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Api { status, .. } | Self::UnexpectedStatus { status } => Some(*status),
            _ => None,
        }
    }

    /// Returns true when the same request can succeed later.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(error) => error.is_connect() || error.is_timeout() || error.is_request(),
            Self::Api { status, code, .. } => {
                *code == ErrorCode::RateLimitHit || is_retryable_status(*status)
            }
            Self::UnexpectedStatus { status } => is_retryable_status(*status),
            Self::InvalidSite(_)
            | Self::MalformedResponse(_)
            | Self::UnsupportedServer { .. }
            | Self::UploadTooLarge { .. }
            | Self::InvalidUploadPath(_) => false,
        }
    }

    /// Returns true when the server surely did not act on the request: the
    /// connection failed, or the server refused the request for its rate
    /// limit. A repeat of a request that is not idempotent, such as a sent
    /// message, is safe only then.
    pub fn is_unsent(&self) -> bool {
        match self {
            Self::Http(error) => error.is_connect(),
            Self::Api { status, code, .. } => {
                *code == ErrorCode::RateLimitHit || *status == StatusCode::TOO_MANY_REQUESTS
            }
            Self::UnexpectedStatus { status } => *status == StatusCode::TOO_MANY_REQUESTS,
            Self::InvalidSite(_)
            | Self::MalformedResponse(_)
            | Self::UnsupportedServer { .. }
            | Self::UploadTooLarge { .. }
            | Self::InvalidUploadPath(_) => false,
        }
    }

    /// Returns the wait that the server requested after a rate limit.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS
}

/// The body of a Zulip error response.
#[derive(Deserialize)]
struct ErrorBody {
    code: ErrorCode,
    msg: String,
    #[serde(rename = "retry-after")]
    retry_after: Option<f64>,
}
