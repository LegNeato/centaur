use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, SystemTime},
};

use reqwest::StatusCode;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{Health, LIVENESS_WINDOW, Listener};
use crate::{
    bot::Bot,
    policy::{Allowlist, Policy},
    testing::{
        ADA, BOT, BOT_NAME, ENGINEERING, FakeSessions, FakeZulip,
        fake_zulip::{api_error, registration},
        reacted, settings, topic_message,
    },
};
use centaur_zulip::{ErrorCode, Event, EventId, EventKind, Message, User, ZulipError};

type TestListener = Listener<FakeZulip, FakeSessions>;

fn listener(zulip: FakeZulip) -> (TestListener, Arc<Bot<FakeZulip, FakeSessions>>, Arc<Health>) {
    let policy = Policy {
        channels: Allowlist::Only(BTreeSet::from([ENGINEERING])),
        ..Policy::default()
    };
    let bot = Arc::new(Bot::new(
        zulip,
        FakeSessions::default(),
        User {
            user_id: BOT,
            full_name: BOT_NAME.to_owned(),
            is_bot: true,
            is_guest: false,
            profile: Default::default(),
        },
        policy,
        settings(),
    ));
    let health = Arc::new(Health::default());
    let listener = Listener::new(bot.clone(), TaskTracker::new(), health.clone());
    (listener, bot, health)
}

fn message_event(id: i64, mut message: Message) -> Event {
    message.mentions_bot = true;
    Event {
        id: EventId(id),
        kind: EventKind::Message(message),
    }
}

/// Runs the listener until `done` returns true. Then it shuts the listener
/// down.
async fn run_until(listener: &TestListener, done: impl Fn() -> bool) -> Result<(), ZulipError> {
    let shutdown = CancellationToken::new();
    let run = listener.run(&shutdown);
    tokio::pin!(run);
    loop {
        tokio::select! {
            result = &mut run => return result,
            () = tokio::time::sleep(Duration::from_millis(1)) => {
                if done() {
                    shutdown.cancel();
                }
            }
        }
    }
}

#[tokio::test]
async fn message_events_reach_the_bot() {
    let mention = topic_message(10, ADA, "@**Centaur** hi", true);
    let (listener, bot, health) = listener(
        FakeZulip::default()
            .with_messages([mention.clone()])
            .with_events([Ok(vec![
                Event {
                    id: EventId(1),
                    kind: EventKind::Heartbeat,
                },
                message_event(2, mention),
            ])]),
    );

    run_until(&listener, || !bot.zulip().sent().is_empty())
        .await
        .unwrap();

    assert_eq!(bot.zulip().reactions(10.into()), ["check"]);
    assert!(health.is_live(SystemTime::now()));
}

#[tokio::test]
async fn an_expired_queue_is_registered_again() {
    let (listener, bot, _) =
        listener(FakeZulip::default().with_events([Err(api_error(ErrorCode::BadEventQueueId))]));

    run_until(&listener, || bot.zulip().queue_registrations() == 2)
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_transient_registration_failure_is_retried() {
    let unavailable = ZulipError::Api {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ErrorCode::Other,
        message: "down".to_owned(),
        retry_after: None,
    };
    let (listener, bot, _) = listener(
        FakeZulip::default().with_registrations([Err(unavailable), Ok(registration("queue", -1))]),
    );

    run_until(&listener, || bot.zulip().queue_registrations() == 2)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_permanent_failure_ends_the_listener() {
    let (listener, _, health) =
        listener(FakeZulip::default().with_registrations([Err(api_error(ErrorCode::Other))]));

    let error = run_until(&listener, || false).await.unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::Other));
    assert!(!health.is_live(SystemTime::now()));
}

#[tokio::test]
async fn registration_starts_the_recovery_scan() {
    let mut interrupted = reacted(topic_message(10, ADA, "@**Centaur** hi", true), &["eyes"]);
    // The scan reads only the recovery window before the current time.
    interrupted.timestamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (listener, bot, _) = listener(FakeZulip::default().with_messages([interrupted]));

    run_until(&listener, || !bot.zulip().sent().is_empty())
        .await
        .unwrap();

    assert_eq!(bot.zulip().reactions(10.into()), ["check"]);
}

#[tokio::test(start_paused = true)]
async fn a_failed_recovery_scan_is_retried() {
    let mut interrupted = reacted(topic_message(10, ADA, "@**Centaur** hi", true), &["eyes"]);
    interrupted.timestamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // The three failures use every attempt of the first scan.
    let unavailable = || ZulipError::UnexpectedStatus {
        status: StatusCode::SERVICE_UNAVAILABLE,
    };
    let (listener, bot, _) = listener(
        FakeZulip::default()
            .with_messages([interrupted])
            .with_history_errors([unavailable(), unavailable(), unavailable()]),
    );

    run_until(&listener, || !bot.zulip().sent().is_empty())
        .await
        .unwrap();

    assert_eq!(bot.zulip().reactions(10.into()), ["check"]);
}

#[test]
fn health_needs_a_recent_success() {
    let health = Health::default();
    let now = SystemTime::now();
    assert!(!health.is_live(now));

    health.record_success(now);
    assert!(health.is_live(now + LIVENESS_WINDOW));
    assert!(!health.is_live(now + LIVENESS_WINDOW + Duration::from_secs(1)));
}
