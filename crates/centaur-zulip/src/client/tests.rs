use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{RawQuery, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use url::Url;

use super::{ZulipClient, ZulipCredentials};
use crate::{
    ChannelId, Conversation, Download, ErrorCode, EventId, EventKind, History, MessageId,
    Participant, QueueId, Recipient, Subscription, UploadPath, UserId, ZulipApi, ZulipError,
};

/// One request the fake server received.
#[derive(Clone, Debug)]
struct Request {
    query: String,
    body: String,
    authorization: String,
}

/// The scripted reply to each method and path.
type Replies = HashMap<(Method, String), (StatusCode, Value)>;

/// A scripted file response with its status, media type, and body.
type File = (StatusCode, &'static str, Vec<u8>);

#[derive(Clone, Default)]
struct FakeServer {
    replies: Arc<Mutex<Replies>>,
    /// The scripted `GET` file responses by path.
    files: Arc<Mutex<HashMap<String, File>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl FakeServer {
    async fn start(
        replies: impl IntoIterator<Item = (Method, &'static str, StatusCode, Value)>,
    ) -> (Self, ZulipClient) {
        let server = Self::default();
        server.replies.lock().unwrap().extend(
            replies
                .into_iter()
                .map(|(method, path, status, body)| ((method, path.to_owned()), (status, body))),
        );
        let app = Router::new()
            .fallback(Self::reply)
            .with_state(server.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let client = ZulipClient::new(&ZulipCredentials {
            site: Url::parse(&format!("http://{address}/chat")).unwrap(),
            email: "bot@example.test".to_owned(),
            api_key: "key".to_owned(),
        })
        .unwrap();
        (server, client)
    }

    async fn reply(
        State(server): State<Self>,
        method: Method,
        uri: Uri,
        RawQuery(query): RawQuery,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let path = uri.path().to_owned();
        server.requests.lock().unwrap().push(Request {
            query: query.unwrap_or_default(),
            body: String::from_utf8(body.to_vec()).unwrap(),
            authorization: headers
                .get(header::AUTHORIZATION)
                .map(|value| value.to_str().unwrap().to_owned())
                .unwrap_or_default(),
        });
        if let Some((status, media_type, body)) = server.files.lock().unwrap().get(&path).cloned() {
            return (status, [(header::CONTENT_TYPE, media_type)], body).into_response();
        }
        let reply = server.replies.lock().unwrap().get(&(method, path)).cloned();
        match reply {
            Some((status, body)) => (status, axum::Json(body)).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn serve_file(&self, path: &str, status: StatusCode, media_type: &'static str, body: &[u8]) {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_owned(), (status, media_type, body.to_vec()));
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn success(extra: Value) -> Value {
    let mut body = json!({"result": "success", "msg": ""});
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    body
}

fn error(code: &str) -> Value {
    json!({"result": "error", "msg": "error message", "code": code})
}

fn registration(feature_level: u64) -> Value {
    success(json!({
        "queue_id": "q1",
        "last_event_id": -1,
        "zulip_feature_level": feature_level,
        "max_message_length": 10000,
        "server_typing_started_wait_period_milliseconds": 10000,
    }))
}

fn topic() -> Conversation {
    Conversation::Topic {
        channel_id: ChannelId(42),
        channel_name: "engineering".to_owned(),
        topic: "Deploys".to_owned(),
    }
}

#[tokio::test]
async fn registration_authenticates_and_asks_for_raw_markdown() {
    let (server, client) = FakeServer::start([(
        Method::POST,
        "/chat/api/v1/register",
        StatusCode::OK,
        registration(400),
    )])
    .await;

    let registration = client.register_queue().await.unwrap();

    assert_eq!(registration.queue_id, QueueId("q1".to_owned()));
    let request = &server.requests()[0];
    // The base64 encoding of "bot@example.test:key".
    assert_eq!(request.authorization, "Basic Ym90QGV4YW1wbGUudGVzdDprZXk=");
    assert!(
        request.body.contains("apply_markdown=false"),
        "{}",
        request.body
    );
    assert!(
        request.body.contains("event_types=%5B%22message%22%5D"),
        "{}",
        request.body
    );
    assert!(
        request.body.contains("all_public_streams=false"),
        "{}",
        request.body
    );
    assert!(
        request
            .body
            .contains("fetch_event_types=%5B%22realm%22%2C%22custom_profile_fields%22%5D"),
        "{}",
        request.body
    );
}

#[tokio::test]
async fn registration_rejects_a_server_older_than_the_minimum() {
    let (_, client) = FakeServer::start([(
        Method::POST,
        "/chat/api/v1/register",
        StatusCode::OK,
        registration(100),
    )])
    .await;

    let error = client.register_queue().await.unwrap_err();

    assert!(error.to_string().contains("feature level 100"), "{error}");
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn an_expired_queue_reports_its_error_code() {
    let (_, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/events",
        StatusCode::BAD_REQUEST,
        error("BAD_EVENT_QUEUE_ID"),
    )])
    .await;

    let error = client
        .events(&QueueId("q1".to_owned()), EventId(3))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::BadEventQueueId));
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_rate_limit_is_retryable_after_the_requested_wait() {
    let mut body = error("RATE_LIMIT_HIT");
    body["retry-after"] = json!(1.5);
    let (_, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/events",
        StatusCode::TOO_MANY_REQUESTS,
        body,
    )])
    .await;

    let error = client
        .events(&QueueId("q1".to_owned()), EventId(3))
        .await
        .unwrap_err();

    assert!(error.is_retryable());
    assert!(error.is_unsent());
    assert_eq!(error.retry_after(), Some(Duration::from_millis(1500)));
}

#[tokio::test]
async fn a_malformed_event_keeps_its_place_in_the_queue() {
    let (server, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/events",
        StatusCode::OK,
        success(json!({"events": [
            {"id": 4, "type": "message", "message": {"id": "not a message"}},
            {"id": 5, "type": "heartbeat"},
        ]})),
    )])
    .await;

    let events = client
        .events(&QueueId("q1".to_owned()), EventId(3))
        .await
        .unwrap();

    assert_eq!(
        events.iter().map(|event| event.id).collect::<Vec<_>>(),
        [EventId(4), EventId(5)]
    );
    assert_eq!(events[0].kind, EventKind::Other);
    assert_eq!(events[1].kind, EventKind::Heartbeat);
    assert_eq!(server.requests()[0].query, "queue_id=q1&last_event_id=3");
}

#[tokio::test]
async fn reaction_requests_succeed_when_the_state_is_already_reached() {
    let (_, client) = FakeServer::start([
        (
            Method::POST,
            "/chat/api/v1/messages/10/reactions",
            StatusCode::BAD_REQUEST,
            error("REACTION_ALREADY_EXISTS"),
        ),
        (
            Method::DELETE,
            "/chat/api/v1/messages/10/reactions",
            StatusCode::BAD_REQUEST,
            error("REACTION_DOES_NOT_EXIST"),
        ),
    ])
    .await;

    client.add_reaction(MessageId(10), "eyes").await.unwrap();
    client.remove_reaction(MessageId(10), "eyes").await.unwrap();
}

#[tokio::test]
async fn other_reaction_errors_fail() {
    let (_, client) = FakeServer::start([(
        Method::POST,
        "/chat/api/v1/messages/10/reactions",
        StatusCode::BAD_REQUEST,
        error("BAD_REQUEST"),
    )])
    .await;

    assert!(client.add_reaction(MessageId(10), "eyes").await.is_err());
}

#[tokio::test]
async fn a_topic_reply_names_the_channel_and_topic() {
    let (server, client) = FakeServer::start([(
        Method::POST,
        "/chat/api/v1/messages",
        StatusCode::OK,
        success(json!({"id": 77})),
    )])
    .await;

    let id = client
        .send_message(&topic(), "done & dusted")
        .await
        .unwrap();

    assert_eq!(id, MessageId(77));
    assert_eq!(
        server.requests()[0].body,
        "type=channel&to=42&topic=Deploys&content=done+%26+dusted"
    );
}

#[tokio::test]
async fn a_direct_reply_lists_the_participants() {
    let (server, client) = FakeServer::start([(
        Method::POST,
        "/chat/api/v1/messages",
        StatusCode::OK,
        success(json!({"id": 78})),
    )])
    .await;
    let direct = Conversation::Direct {
        participants: vec![
            Participant {
                id: UserId(7),
                full_name: "Ada".to_owned(),
            },
            Participant {
                id: UserId(9),
                full_name: "Grace".to_owned(),
            },
        ],
    };

    client.send_message(&direct, "hi").await.unwrap();

    assert_eq!(
        server.requests()[0].body,
        "type=direct&to=%5B7%2C9%5D&content=hi"
    );
}

#[tokio::test]
async fn history_before_a_message_narrows_to_its_conversation() {
    let (server, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/messages",
        StatusCode::OK,
        success(json!({"messages": []})),
    )])
    .await;
    let conversation = topic();

    client
        .messages(
            History::Conversation {
                conversation: &conversation,
                before: MessageId(10),
            },
            50,
        )
        .await
        .unwrap();

    let query = &server.requests()[0].query;
    let pairs = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect::<HashMap<_, _>>();
    assert_eq!(pairs["anchor"], "10");
    assert_eq!(pairs["include_anchor"], "false");
    assert_eq!(pairs["num_before"], "50");
    assert_eq!(pairs["apply_markdown"], "false");
    assert_eq!(
        serde_json::from_str::<Value>(&pairs["narrow"]).unwrap(),
        json!([
            {"operator": "channel", "operand": 42},
            {"operator": "topic", "operand": "Deploys"},
        ])
    );
}

#[tokio::test]
async fn a_scan_page_starts_at_the_newest_message_or_before_a_given_one() {
    let (server, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/messages",
        StatusCode::OK,
        success(json!({"messages": []})),
    )])
    .await;

    client
        .messages(History::Mentions { before: None }, 1000)
        .await
        .unwrap();
    client
        .messages(
            History::DirectMessages {
                before: Some(MessageId(500)),
            },
            1000,
        )
        .await
        .unwrap();

    let queries = server
        .requests()
        .iter()
        .map(|request| {
            url::form_urlencoded::parse(request.query.as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(queries[0]["anchor"], "newest");
    assert_eq!(queries[0]["include_anchor"], "true");
    assert_eq!(
        serde_json::from_str::<Value>(&queries[0]["narrow"]).unwrap(),
        json!([{"operator": "is", "operand": "mentioned"}])
    );
    assert_eq!(queries[1]["anchor"], "500");
    assert_eq!(queries[1]["include_anchor"], "false");
    assert_eq!(
        serde_json::from_str::<Value>(&queries[1]["narrow"]).unwrap(),
        json!([{"operator": "is", "operand": "dm"}])
    );
}

#[tokio::test]
async fn a_proxy_error_page_is_reported_without_its_body() {
    let (_, client) = FakeServer::start([]).await;

    let error = client.own_user().await.unwrap_err();

    assert!(
        matches!(error, ZulipError::UnexpectedStatus { status } if status == StatusCode::NOT_FOUND),
        "{error}"
    );
    assert_eq!(error.code(), None);
    assert!(!error.is_retryable());
}

mod channels {
    use super::*;

    #[tokio::test]
    async fn the_subscription_state_reads_the_flag_or_a_hidden_channel() {
        let (_, client) = FakeServer::start([
            (
                Method::GET,
                "/chat/api/v1/users/99/subscriptions/1",
                StatusCode::OK,
                success(json!({"is_subscribed": true})),
            ),
            (
                Method::GET,
                "/chat/api/v1/users/99/subscriptions/2",
                StatusCode::OK,
                success(json!({"is_subscribed": false})),
            ),
            (
                Method::GET,
                "/chat/api/v1/users/99/subscriptions/3",
                StatusCode::BAD_REQUEST,
                error("BAD_REQUEST"),
            ),
            (
                Method::GET,
                "/chat/api/v1/users/99/subscriptions/4",
                StatusCode::INTERNAL_SERVER_ERROR,
                error("INTERNAL"),
            ),
        ])
        .await;

        let state = |id| client.subscription(UserId(99), ChannelId(id));
        assert_eq!(state(1).await.unwrap(), Subscription::Subscribed);
        assert_eq!(state(2).await.unwrap(), Subscription::Unsubscribed);
        assert_eq!(state(3).await.unwrap(), Subscription::Hidden);
        assert!(state(4).await.unwrap_err().is_retryable());
    }

    #[tokio::test]
    async fn a_subscription_request_names_the_channel() {
        let (server, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/users/me/subscriptions",
            StatusCode::OK,
            success(json!({"subscribed": {"99": ["engineering"]}, "already_subscribed": {}})),
        )])
        .await;

        client.subscribe("engineering").await.unwrap();

        let body = &server.requests()[0].body;
        let form = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            serde_json::from_str::<Value>(&form["subscriptions"]).unwrap(),
            json!([{"name": "engineering"}])
        );
    }

    #[tokio::test]
    async fn a_refused_subscription_reports_a_bad_request() {
        let (_, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/users/me/subscriptions",
            StatusCode::BAD_REQUEST,
            error("BAD_REQUEST"),
        )])
        .await;

        let error = client.subscribe("secret").await.unwrap_err();

        assert_eq!(error.code(), Some(ErrorCode::BadRequest));
        assert!(!error.is_retryable());
    }
}

mod uploads {
    use super::*;

    const NOTES: &str = "/user_uploads/2/5a/abc/r%C3%A9sum%C3%A9.txt";

    #[tokio::test]
    async fn the_links_come_from_the_rendered_message() {
        let (server, client) = FakeServer::start([(
            Method::GET,
            "/chat/api/v1/messages/10",
            StatusCode::OK,
            success(json!({"message": {"content": format!(
                "<p><a href=\"{NOTES}\">notes</a> <code>/user_uploads/2/x/y/z.txt</code></p>"
            )}})),
        )])
        .await;

        let uploads = client.linked_uploads(MessageId(10)).await.unwrap();

        assert_eq!(uploads, [UploadPath::parse(NOTES).unwrap()]);
        assert_eq!(server.requests()[0].query, "apply_markdown=true");
    }

    #[tokio::test]
    async fn a_download_returns_the_bytes_and_the_bare_media_type() {
        let (server, client) = FakeServer::start([]).await;
        server.serve_file(
            "/chat/user_uploads/2/5a/abc/r%C3%A9sum%C3%A9.txt",
            StatusCode::OK,
            "text/plain; charset=utf-8",
            b"hello",
        );

        let download = client
            .download(&UploadPath::parse(NOTES).unwrap(), 5)
            .await
            .unwrap();

        assert_eq!(
            download,
            Download {
                media_type: Some("text/plain".to_owned()),
                bytes: b"hello".to_vec(),
            }
        );
        assert!(server.requests()[0].authorization.starts_with("Basic "));
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_refused() {
        let (server, client) = FakeServer::start([]).await;
        server.serve_file(
            "/chat/user_uploads/2/5a/abc/r%C3%A9sum%C3%A9.txt",
            StatusCode::OK,
            "text/plain",
            b"hello",
        );

        let error = client
            .download(&UploadPath::parse(NOTES).unwrap(), 4)
            .await
            .unwrap_err();

        assert!(matches!(error, ZulipError::UploadTooLarge { limit: 4 }));
    }

    #[tokio::test]
    async fn a_forbidden_file_reports_its_status() {
        let (server, client) = FakeServer::start([]).await;
        server.serve_file(
            "/chat/user_uploads/2/5a/abc/r%C3%A9sum%C3%A9.txt",
            StatusCode::FORBIDDEN,
            "text/html",
            b"<p>You are not authorized to view this file.</p>",
        );

        let error = client
            .download(&UploadPath::parse(NOTES).unwrap(), 1024)
            .await
            .unwrap_err();

        assert_eq!(error.status(), Some(StatusCode::FORBIDDEN));
        assert!(!error.is_retryable());
    }
}

#[tokio::test]
async fn a_user_lookup_asks_for_the_profile_fields() {
    let (server, client) = FakeServer::start([(
        Method::GET,
        "/chat/api/v1/users/7",
        StatusCode::OK,
        success(json!({"user": {"user_id": 7, "full_name": "Ada", "is_bot": false}})),
    )])
    .await;

    client.user(UserId(7)).await.unwrap();

    assert_eq!(
        server.requests()[0].query,
        "include_custom_profile_fields=true"
    );
}

mod uploading {
    use super::*;

    #[tokio::test]
    async fn an_upload_sends_the_file_and_returns_its_path() {
        let (server, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/user_uploads",
            StatusCode::OK,
            success(json!({"url": "/user_uploads/2/64/abc/chart-final.png", "filename": "chart final.png"})),
        )])
        .await;

        let upload = client
            .upload("chart final.png", "image/png", &b"png"[..], 3)
            .await
            .unwrap();

        assert_eq!(upload.to_string(), "/user_uploads/2/64/abc/chart-final.png");
        let body = &server.requests()[0].body;
        assert!(body.contains("filename=\"chart final.png\""), "{body}");
        assert!(body.contains("image/png"), "{body}");
    }

    #[tokio::test]
    async fn an_upload_path_outside_the_upload_area_is_an_error() {
        let (_, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/user_uploads",
            StatusCode::OK,
            success(json!({"url": "/api/v1/users/me"})),
        )])
        .await;

        let error = client
            .upload("a.txt", "text/plain", &b"a"[..], 1)
            .await
            .unwrap_err();

        assert!(matches!(error, ZulipError::InvalidUploadPath(_)), "{error}");
    }

    #[tokio::test]
    async fn a_direct_recipient_lists_its_user_ids() {
        let (server, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/messages",
            StatusCode::OK,
            success(json!({"id": 5})),
        )])
        .await;

        let id = client
            .send_message_to(
                &Recipient::Direct {
                    user_ids: vec![UserId(7), UserId(9)],
                },
                "hi",
            )
            .await
            .unwrap();

        assert_eq!(id, MessageId(5));
        assert!(server.requests()[0].body.contains("to=%5B7%2C9%5D"));
    }
}

mod site {
    use super::*;

    fn client(site: &str) -> Result<ZulipClient, ZulipError> {
        ZulipClient::new(&ZulipCredentials {
            site: Url::parse(site).unwrap(),
            email: "bot@example.test".to_owned(),
            api_key: "key".to_owned(),
        })
    }

    #[test]
    fn the_api_and_upload_urls_keep_the_site_path() {
        for (site, api, uploads) in [
            (
                "https://chat.example.test",
                "https://chat.example.test/api/v1/",
                "https://chat.example.test/user_uploads/",
            ),
            (
                "https://example.test/chat/",
                "https://example.test/chat/api/v1/",
                "https://example.test/chat/user_uploads/",
            ),
        ] {
            let client = client(site).unwrap();
            assert_eq!(client.api_url.as_str(), api, "{site}");
            assert_eq!(client.uploads_url.as_str(), uploads, "{site}");
        }
    }

    #[test]
    fn a_site_that_cannot_hold_a_path_is_rejected() {
        let error = client("mailto:bot@example.test").unwrap_err();
        assert!(matches!(error, ZulipError::InvalidSite(_)), "{error}");
    }
}

mod typing {
    use super::*;

    #[tokio::test]
    async fn a_topic_notification_names_the_channel_and_topic() {
        let (server, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/typing",
            StatusCode::OK,
            success(json!({})),
        )])
        .await;

        client.set_typing(&topic(), true).await.unwrap();

        assert_eq!(
            server.requests()[0].body,
            "type=channel&stream_id=42&topic=Deploys&op=start"
        );
    }

    #[tokio::test]
    async fn a_direct_notification_lists_the_participants() {
        let (server, client) = FakeServer::start([(
            Method::POST,
            "/chat/api/v1/typing",
            StatusCode::OK,
            success(json!({})),
        )])
        .await;
        let direct = Conversation::Direct {
            participants: vec![Participant {
                id: UserId(7),
                full_name: "Ada".to_owned(),
            }],
        };

        client.set_typing(&direct, false).await.unwrap();

        assert_eq!(server.requests()[0].body, "type=direct&to=%5B7%5D&op=stop");
    }
}

mod errors {
    use super::*;

    #[tokio::test]
    async fn a_server_error_page_is_retryable() {
        let (server, client) = FakeServer::start([]).await;
        server.serve_file(
            "/chat/api/v1/users/me",
            StatusCode::BAD_GATEWAY,
            "text/html",
            b"<h1>502</h1>",
        );

        let error = client.own_user().await.unwrap_err();

        assert_eq!(error.status(), Some(StatusCode::BAD_GATEWAY));
        assert!(error.is_retryable());
        // The server can have acted on the request before the proxy failed.
        assert!(!error.is_unsent());
    }

    #[tokio::test]
    async fn a_refused_connection_is_unsent() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = ZulipClient::new(&ZulipCredentials {
            site: Url::parse(&format!("http://{address}")).unwrap(),
            email: "bot@example.test".to_owned(),
            api_key: "key".to_owned(),
        })
        .unwrap();

        let error = client.own_user().await.unwrap_err();

        assert!(error.is_unsent(), "{error}");
        assert!(error.is_retryable());
    }

    #[tokio::test]
    async fn an_event_without_an_id_is_dropped() {
        let (_, client) = FakeServer::start([(
            Method::GET,
            "/chat/api/v1/events",
            StatusCode::OK,
            success(json!({"events": [
                {"type": "heartbeat"},
                {"id": 5, "type": "heartbeat"},
            ]})),
        )])
        .await;

        let events = client
            .events(&QueueId("q1".to_owned()), EventId(4))
            .await
            .unwrap();

        assert_eq!(
            events.iter().map(|event| event.id).collect::<Vec<_>>(),
            [EventId(5)]
        );
    }
}
