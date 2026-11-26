use centaur_session_core::{ThreadKey, api::EventsQuery};
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
fn server_errors_and_throttling_are_retryable() {
    for status in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let error = ClientError::Api {
            status,
            body: String::new(),
        };
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
        let error = ClientError::Api {
            status,
            body: String::new(),
        };
        assert!(!error.is_retryable(), "{status} should not be retryable");
    }
    assert!(!ClientError::InvalidBearerToken.is_retryable());
}


#[test]
fn a_base_url_that_cannot_hold_a_path_is_rejected() {
    let error = SessionClient::new(Url::parse("data:text/plain,x").unwrap()).unwrap_err();
    assert!(matches!(error, ClientError::InvalidBaseUrl(_)), "{error}");
}
