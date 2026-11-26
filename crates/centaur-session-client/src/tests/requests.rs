use centaur_session_core::{
    HarnessType, ThreadKey,
    api::{ApiErrorCode, EventsQuery},
};
use reqwest::StatusCode;
use url::Url;

use crate::{ClientError, SessionClient};

fn client(base_url: &str) -> SessionClient {
    SessionClient::new(Url::parse(base_url).unwrap()).unwrap()
}

#[test]
fn session_url_percent_encodes_the_thread_key_as_one_segment() {
    let key = ThreadKey::parse("github:owner/repo:7 x%").unwrap();
    let url = client("http://api.test").session_url(&key, Some("events"));
    assert_eq!(
        url.as_str(),
        "http://api.test/api/session/github:owner%2Frepo:7%20x%25/events"
    );
}

#[test]
fn session_url_keeps_a_base_path_prefix() {
    let key = ThreadKey::parse("cli:abc").unwrap();
    for base in ["http://api.test/centaur", "http://api.test/centaur/"] {
        let url = client(base).session_url(&key, None);
        assert_eq!(url.as_str(), "http://api.test/centaur/api/session/cli:abc");
    }
}

#[test]
fn events_query_serializes_only_present_fields() {
    let query = EventsQuery {
        after_event_id: Some(4),
        execution_id: None,
    };
    let url = reqwest::Client::new()
        .get("http://api.test/events")
        .query(&query)
        .build()
        .unwrap()
        .url()
        .clone();
    assert_eq!(url.query(), Some("after_event_id=4"));
}

#[test]
fn api_code_reads_a_known_error_code() {
    let denied = ClientError::api(
        StatusCode::FORBIDDEN,
        r#"{"error":"forbidden","code":"session_principal_not_preapproved"}"#.to_owned(),
    );
    assert_eq!(
        denied.api_code(),
        Some(ApiErrorCode::SessionPrincipalNotPreapproved)
    );
}

#[test]
fn api_code_marks_a_newer_code_as_other() {
    let error = ClientError::api(
        StatusCode::BAD_REQUEST,
        r#"{"error":"x","code":"something_new"}"#.to_owned(),
    );
    assert_eq!(error.api_code(), Some(ApiErrorCode::Other));
}

#[test]
fn api_code_is_absent_for_a_missing_code_or_another_body() {
    for body in [r#"{"error":"x"}"#, "not json", "<html>bad gateway</html>"] {
        let error = ClientError::api(StatusCode::BAD_REQUEST, body.to_owned());
        assert_eq!(error.api_code(), None, "{body}");
    }
    assert_eq!(ClientError::StreamEnded.api_code(), None);
}

#[test]
fn server_errors_and_throttling_are_retryable() {
    for status in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let error = ClientError::api(status, String::new());
        assert!(error.is_retryable(), "{status} should be retryable");
    }
}

#[test]
fn client_errors_are_not_retryable() {
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::CONFLICT,
    ] {
        let error = ClientError::api(status, String::new());
        assert!(!error.is_retryable(), "{status} should not be retryable");
    }
    assert!(!ClientError::InvalidBearerToken.is_retryable());
}

#[test]
fn existing_harness_reads_only_a_harness_conflict() {
    let conflict = ClientError::api(
        StatusCode::CONFLICT,
        r#"{"error":"conflict","code":"harness_conflict","existing_harness":"claudecode","requested_harness":"codex"}"#.to_owned(),
    );
    assert_eq!(conflict.existing_harness(), Some(HarnessType::ClaudeCode));

    let principal = ClientError::api(
        StatusCode::CONFLICT,
        r#"{"error":"conflict","code":"principal_conflict","existing_harness":"codex"}"#.to_owned(),
    );
    assert_eq!(principal.existing_harness(), None);
    assert_eq!(ClientError::StreamEnded.existing_harness(), None);
}

#[test]
fn a_base_url_that_cannot_hold_a_path_is_rejected() {
    let error = SessionClient::new(Url::parse("data:text/plain,x").unwrap()).unwrap_err();
    assert!(matches!(error, ClientError::InvalidBaseUrl(_)), "{error}");
}
