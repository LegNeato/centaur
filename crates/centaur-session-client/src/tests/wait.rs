use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    extract::{RawQuery, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use centaur_session_core::{
    ThreadKey,
    event::{ExecutionEnd, SessionEventType},
};
use url::Url;

use crate::{ClientError, Reconnect, SessionClient};

/// One scripted answer of the fake events endpoint.
struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn events(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.into(),
        }
    }

    fn status(status: StatusCode) -> Self {
        Self {
            status,
            body: String::new(),
        }
    }
}

#[derive(Clone, Default)]
struct FakeEvents {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    queries: Arc<Mutex<Vec<String>>>,
}

impl FakeEvents {
    async fn serve(replies: impl IntoIterator<Item = Reply>) -> (Self, SessionClient) {
        let fake = Self {
            replies: Arc::new(Mutex::new(replies.into_iter().collect())),
            queries: Arc::default(),
        };
        let app = Router::new()
            .route("/api/session/{thread_key}/events", get(Self::events))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let client = SessionClient::new(Url::parse(&format!("http://{address}")).unwrap()).unwrap();
        (fake, client)
    }

    async fn events(State(fake): State<Self>, RawQuery(query): RawQuery) -> Response {
        fake.queries.lock().unwrap().push(query.unwrap_or_default());
        let reply = fake
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::status(StatusCode::GONE));
        (
            reply.status,
            [(header::CONTENT_TYPE, "text/event-stream")],
            reply.body,
        )
            .into_response()
    }

    fn queries(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

fn thread_key() -> ThreadKey {
    ThreadKey::parse("zulip:channel:42:deploys").unwrap()
}

fn quick_reconnect(max_reconnects: u32) -> Reconnect {
    Reconnect {
        max_reconnects,
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
    }
}

const COMPLETED: &str = "id: 4\nevent: session.execution_completed\ndata: {\"execution_id\":\"exe-1\",\"thread_key\":\"zulip:channel:42:deploys\",\"completion_reason\":\"turn_completed\",\"result_text\":\"PONG\"}\n\n";

#[tokio::test]
async fn returns_the_end_and_skips_other_events() {
    let body = format!(
        "id: 3\nevent: session.output.line\ndata: {{\"method\":\"item/started\"}}\n\n{COMPLETED}"
    );
    let (fake, client) = FakeEvents::serve([Reply::events(body)]).await;

    let end = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(0), |_| {})
        .await
        .unwrap();

    let ExecutionEnd::Completed(completed) = end else {
        panic!("expected completion, got {end:?}");
    };
    assert_eq!(completed.result_text.as_deref(), Some("PONG"));
    assert_eq!(fake.queries(), ["execution_id=exe-1"]);
}

#[tokio::test]
async fn reopens_the_stream_after_the_last_delivered_event() {
    let (fake, client) = FakeEvents::serve([
        Reply::events("id: 3\nevent: session.output.line\ndata: line\n\n"),
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
        Reply::events("event: session.stream_error\ndata: {\"error\":\"event stream failed\"}\n\n"),
        Reply::events(COMPLETED),
    ])
    .await;

    let end = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(3), |_| {})
        .await
        .unwrap();

    assert!(matches!(end, ExecutionEnd::Completed(_)));
    assert_eq!(
        fake.queries(),
        [
            "execution_id=exe-1",
            "after_event_id=3&execution_id=exe-1",
            "after_event_id=3&execution_id=exe-1",
            "after_event_id=3&execution_id=exe-1",
        ]
    );
}

#[tokio::test]
async fn repeated_stream_errors_use_up_the_failure_budget() {
    let stream_error = "event: session.stream_error\ndata: {\"error\":\"event stream failed\"}\n\n";
    let (fake, client) = FakeEvents::serve([
        Reply::events(stream_error),
        Reply::events(stream_error),
        Reply::events(stream_error),
        Reply::events(COMPLETED),
    ])
    .await;

    let error = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(2), |_| {})
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::StreamEnded));
    assert_eq!(fake.queries().len(), 3);
}

#[tokio::test]
async fn returns_a_client_error_without_reconnecting() {
    let (fake, client) = FakeEvents::serve([Reply::status(StatusCode::FORBIDDEN)]).await;

    let error = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(5), |_| {})
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ClientError::Api {
            status: StatusCode::FORBIDDEN,
            ..
        }
    ));
    assert_eq!(fake.queries().len(), 1);
}

#[tokio::test]
async fn a_stored_event_resets_the_failure_count() {
    let event = "id: 3\nevent: session.output.line\ndata: line\n\n";
    let (fake, client) = FakeEvents::serve([
        Reply::events(event),
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
        Reply::events(event),
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
        Reply::events(COMPLETED),
    ])
    .await;

    let end = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(1), |_| {})
        .await;

    assert!(matches!(end, Ok(ExecutionEnd::Completed(_))), "{end:?}");
    assert_eq!(fake.queries().len(), 5);
}

#[tokio::test]
async fn gives_up_after_the_allowed_consecutive_failures() {
    let (fake, client) = FakeEvents::serve([
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
        Reply::status(StatusCode::SERVICE_UNAVAILABLE),
    ])
    .await;

    let error = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(2), |_| {})
        .await
        .unwrap_err();

    assert!(error.is_retryable());
    assert_eq!(fake.queries().len(), 3);
}

#[tokio::test]
async fn rejects_a_malformed_end_payload() {
    let (_fake, client) = FakeEvents::serve([Reply::events(
        "id: 4\nevent: session.execution_failed\ndata: {\"error\":1}\n\n",
    )])
    .await;

    let error = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(5), |_| {})
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ClientError::MalformedEvent {
            event_type: SessionEventType::ExecutionFailed,
            ..
        }
    ));
}

#[test]
fn reconnect_delay_doubles_up_to_the_maximum() {
    let reconnect = Reconnect {
        max_reconnects: 10,
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(350),
    };
    let delays = (0..4).map(|failures| reconnect.delay(failures));
    assert_eq!(
        delays.collect::<Vec<_>>(),
        [100, 200, 350, 350].map(Duration::from_millis)
    );
}

#[tokio::test]
async fn passes_progress_updates_in_order_and_skips_a_malformed_one() {
    let body = "id: 2\nevent: session.progress_update\ndata: {\"execution_id\":\"exe-1\",\"text\":\"Reading the logs\",\"created_at_ms\":1,\"elapsed_secs\":5,\"model\":\"m\"}\n\n\
                id: 3\nevent: session.progress_update\ndata: {\"text\":7}\n\n\
                id: 4\nevent: session.progress_update\ndata: {\"execution_id\":\"exe-1\",\"text\":\"Writing the fix\",\"created_at_ms\":2,\"elapsed_secs\":9,\"model\":\"m\"}\n\n";
    let (_, client) = FakeEvents::serve([Reply::events(body), Reply::events(COMPLETED)]).await;
    let mut updates = Vec::new();

    let end = client
        .wait_for_execution_end(&thread_key(), "exe-1", quick_reconnect(1), |update| {
            updates.push(update.text);
        })
        .await;

    assert!(matches!(end, Ok(ExecutionEnd::Completed(_))));
    assert_eq!(updates, ["Reading the logs", "Writing the fix"]);
}
