use super::*;

#[tokio::test(start_paused = true)]
async fn a_send_that_may_have_reached_zulip_is_not_repeated() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_send_errors([ZulipError::UnexpectedStatus {
                status: StatusCode::GATEWAY_TIMEOUT,
            }]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.send_attempts(), 1);
    assert!(bot.zulip.sent().is_empty());
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
}

#[tokio::test(start_paused = true)]
async fn a_rate_limited_send_is_repeated() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_send_errors([ZulipError::Api {
                status: StatusCode::TOO_MANY_REQUESTS,
                code: ErrorCode::RateLimitHit,
                message: "API usage exceeded rate limit".to_owned(),
                retry_after: Some(Duration::from_secs(1)),
            }]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.send_attempts(), 2);
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test]
async fn a_failed_hand_off_posts_a_notice_and_a_cross_mark() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_create_error(ClientError::api(
            StatusCode::FORBIDDEN,
            "internal detail".to_owned(),
        )),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [START_FAILED_NOTICE]);
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
    assert!(bot.sessions.executed().is_empty());
}

#[tokio::test]
async fn a_failed_execution_reports_its_error() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_end(
            "exe-1",
            ExecutionEnd::Failed(ExecutionFailed {
                execution_id: "exe-1".to_owned(),
                thread_key: ThreadKey::parse("zulip:channel:42:deploys").unwrap(),
                error: "max duration exceeded".to_owned(),
                reason: Some("max_duration_exceeded".to_owned()),
                max_duration_ms: Some(1),
            }),
        ),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), ["The run failed: max duration exceeded"]);
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
}

#[tokio::test]
async fn an_answer_that_cannot_be_posted_is_marked_failed() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .failing_sends(1),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert!(bot.zulip.sent().is_empty());
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
}
