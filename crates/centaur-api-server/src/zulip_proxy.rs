//! The Zulip routes of the agent.
//!
//! A sandbox posts a message or a file to the Zulip conversation of its own
//! session. The route finds that session from the sandbox ID of the sandbox
//! token. The request cannot name another conversation. api-rs posts with the
//! account of the Zulip bot. `ZULIP_SITE`, `ZULIP_EMAIL`, and `ZULIP_API_KEY`
//! configure it.

use std::sync::OnceLock;

use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use centaur_session_core::{
    ThreadKey,
    destination::{ChatDestination, ZulipDestination},
};
use centaur_zulip::{
    ChannelId, ErrorCode, Recipient, UserId, ZulipClient, ZulipCredentials, ZulipError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;
use url::Url;

use crate::{
    ApiError,
    console::sandbox::SandboxContext,
    routes::{AppState, non_empty_env},
    upload::{
        DEFAULT_CONTENT_TYPE, content_length, ensure_upload_size, validate_content_type,
        validate_filename,
    },
};

/// The largest file that the agent can post. The Zulip server can set a
/// lower limit.
const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

pub(crate) fn zulip_proxy_router() -> Router<AppState> {
    Router::new()
        .route("/api/zulip/messages", post(post_message))
        .route("/api/zulip/files", post(post_file))
}

#[derive(Debug, Deserialize)]
struct PostMessageRequest {
    /// Zulip Markdown.
    content: String,
}

#[derive(Debug, Deserialize)]
struct PostFileQuery {
    filename: String,
    /// A text that the message shows above the file link.
    comment: Option<String>,
    content_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct PostResponse {
    ok: bool,
    message_id: u64,
    /// The upload path of a posted file.
    #[serde(skip_serializing_if = "Option::is_none")]
    upload: Option<String>,
}

async fn post_message(
    State(state): State<AppState>,
    context: SandboxContext,
    Json(request): Json<PostMessageRequest>,
) -> Result<Json<PostResponse>, ApiError> {
    if request.content.trim().is_empty() {
        return Err(ApiError::BadRequest("content must not be empty".to_owned()));
    }
    let recipient = session_recipient(&state, &context).await?;
    let message_id = client()?
        .send_message_to(&recipient, &request.content)
        .await
        .map_err(zulip_failure)?;
    Ok(Json(PostResponse {
        ok: true,
        message_id: message_id.0,
        upload: None,
    }))
}

/// Streams the request body to Zulip. The route therefore needs the
/// `Content-Length` header. It finds the conversation before it reads the
/// body.
async fn post_file(
    State(state): State<AppState>,
    context: SandboxContext,
    Query(query): Query<PostFileQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<PostResponse>, ApiError> {
    validate_filename(&query.filename)?;
    let file_name = query.filename.trim();
    let media_type = match query.content_type.as_deref() {
        Some(content_type) => {
            validate_content_type(content_type)?;
            content_type
        }
        None => DEFAULT_CONTENT_TYPE,
    };
    let length = content_length(&headers)?;
    ensure_upload_size(length, MAX_FILE_BYTES)?;
    let recipient = session_recipient(&state, &context).await?;
    let client = client()?;
    let file = reqwest::Body::wrap_stream(body.into_data_stream());
    let upload = client
        .upload(file_name, media_type, file, length)
        .await
        .map_err(zulip_failure)?
        .to_string();
    let content = file_message(file_name, &upload, query.comment.as_deref());
    let message_id = client
        .send_message_to(&recipient, &content)
        .await
        .map_err(zulip_failure)?;
    Ok(Json(PostResponse {
        ok: true,
        message_id: message_id.0,
        upload: Some(upload),
    }))
}

/// Returns the Zulip conversation of the session that holds the calling
/// sandbox.
async fn session_recipient(
    state: &AppState,
    context: &SandboxContext,
) -> Result<Recipient, ApiError> {
    let thread_keys = state
        .runtime()?
        .thread_keys_for_sandbox(&context.claims.sandbox_id)
        .await?;
    Ok(recipient(&zulip_destination(&thread_keys)?))
}

/// Returns the one Zulip conversation among the thread keys of a sandbox.
fn zulip_destination(thread_keys: &[ThreadKey]) -> Result<ZulipDestination, ApiError> {
    let mut conversations =
        thread_keys
            .iter()
            .filter_map(|thread_key| match thread_key.chat_destination()? {
                ChatDestination::Zulip(destination) => Some(destination),
                _ => None,
            });
    let destination = conversations.next().ok_or_else(|| {
        ApiError::Forbidden("the sandbox serves no Zulip conversation".to_owned())
    })?;
    if conversations.next().is_some() {
        return Err(ApiError::Forbidden(
            "the sandbox serves more than one Zulip conversation".to_owned(),
        ));
    }
    Ok(destination)
}

fn recipient(destination: &ZulipDestination) -> Recipient {
    match destination {
        // Zulip matches topic names without regard to case. The lowercase
        // topic of the thread key therefore reaches the same topic.
        ZulipDestination::Topic(topic) => Recipient::Topic {
            channel_id: ChannelId(topic.channel_id()),
            topic: topic.topic().to_owned(),
        },
        ZulipDestination::Direct(direct) => Recipient::Direct {
            user_ids: direct.user_ids().iter().copied().map(UserId).collect(),
        },
    }
}

/// Returns the message that shares an upload: the comment, if any, and a
/// link to the file.
fn file_message(file_name: &str, upload: &str, comment: Option<&str>) -> String {
    let link = format!("[{}]({upload})", markdown_link_text(file_name));
    match comment.map(str::trim).filter(|comment| !comment.is_empty()) {
        Some(comment) => format!("{comment}\n\n{link}"),
        None => link,
    }
}

/// Escapes the Markdown link text characters of a file name.
fn markdown_link_text(file_name: &str) -> String {
    file_name
        .chars()
        .flat_map(|character| {
            let escape = matches!(character, '[' | ']' | '\\').then_some('\\');
            escape.into_iter().chain([character])
        })
        .collect()
}

#[derive(Debug, Error)]
enum ConfigError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("ZULIP_SITE is not a URL: {0}")]
    InvalidSite(#[from] url::ParseError),
    #[error(transparent)]
    Client(#[from] ZulipError),
}

/// Returns the client of the bot account. The first call reads the
/// environment and logs a missing or invalid setting.
fn client() -> Result<&'static ZulipClient, ApiError> {
    static CLIENT: OnceLock<Option<ZulipClient>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            client_from(non_empty_env)
                .inspect_err(|error| warn!(event = "zulip_proxy_not_configured", %error))
                .ok()
        })
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Zulip is not configured".to_owned()))
}

fn client_from(env: impl Fn(&str) -> Option<String>) -> Result<ZulipClient, ConfigError> {
    let var = |name: &'static str| env(name).ok_or(ConfigError::Missing(name));
    let credentials = ZulipCredentials {
        site: Url::parse(&var("ZULIP_SITE")?)?,
        email: var("ZULIP_EMAIL")?,
        api_key: var("ZULIP_API_KEY")?,
    };
    Ok(ZulipClient::new(&credentials)?)
}

/// Maps a Zulip failure. Zulip explains a rejected request, for example a
/// message over the length limit. The agent can then fix its request. A
/// rejected bot credential and a rate limit are not faults of the request.
fn zulip_failure(error: ZulipError) -> ApiError {
    if error.code() == Some(ErrorCode::RateLimitHit) {
        let wait = error
            .retry_after()
            .map(|wait| format!(" Retry after {} seconds.", wait.as_secs().max(1)))
            .unwrap_or_default();
        return ApiError::ServiceUnavailable(format!("Zulip limits the request rate.{wait}"));
    }
    match (error.status(), &error) {
        (Some(StatusCode::PAYLOAD_TOO_LARGE), _) => {
            ApiError::PayloadTooLarge("the file is larger than the Zulip upload limit".to_owned())
        }
        (Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN), _) => unavailable(&error),
        (Some(status), ZulipError::Api { message, .. }) if status.is_client_error() => {
            ApiError::Unprocessable(format!("Zulip rejected the request: {message}"))
        }
        (Some(status), _) if status.is_client_error() => {
            ApiError::Unprocessable(format!("Zulip rejected the request with HTTP {status}"))
        }
        _ => unavailable(&error),
    }
}

fn unavailable(error: &ZulipError) -> ApiError {
    warn!(event = "zulip_proxy_request_failed", %error);
    ApiError::ServiceUnavailable("Zulip is not available".to_owned())
}

#[cfg(test)]
mod tests;
