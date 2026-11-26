use std::time::SystemTime;

use super::*;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

#[tokio::test]
async fn fresh_updates_are_posted_before_the_answer_and_stale_ones_are_dropped() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default()
            .with_progress("exe-1", "Replayed from long ago", now_ms() - 600_000)
            .with_progress("exe-1", "Reading the deploy logs", now_ms()),
    );

    bot.handle(trigger).await;

    assert_eq!(
        bot.zulip.sent(),
        [
            "Reading the deploy logs".to_owned(),
            default_answer("exe-1")
        ]
    );
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test(start_paused = true)]
async fn a_wait_after_an_outage_gets_the_answer_without_repeating_updates() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default()
            .with_progress("exe-1", "Reading the deploy logs", now_ms())
            .with_wait_error(ClientError::api(
                StatusCode::SERVICE_UNAVAILABLE,
                String::new(),
            )),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.sessions.waits(), 2);
    assert_eq!(
        bot.zulip.sent(),
        [
            "Reading the deploy logs".to_owned(),
            default_answer("exe-1")
        ]
    );
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test]
async fn a_lost_execution_is_reported_once() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default()
            .with_wait_error(ClientError::api(StatusCode::NOT_FOUND, String::new())),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.sessions.waits(), 1);
    assert_eq!(bot.zulip.sent(), [LOST_EXECUTION_NOTICE]);
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
}
