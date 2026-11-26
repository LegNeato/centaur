use super::*;

#[tokio::test]
async fn a_mention_gets_the_answer_and_a_check_mark() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([
            topic_message(5, GRACE, "the deploy is red", false),
            trigger.clone(),
        ]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
    assert_eq!(bot.zulip.typing(), [true, false]);

    let created = bot.sessions.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].0.as_str(), "zulip:channel:42:deploys");
    let appended = &bot.sessions.appended()[0].1.messages;
    let ids = appended
        .iter()
        .map(|message| message.client_message_id.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["5", "10"]);
    let executed = bot.sessions.executed();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].1.idempotency_key.as_deref(), Some("10"));
}

#[tokio::test]
async fn a_one_to_one_direct_message_is_answered_in_the_same_conversation() {
    let trigger = direct_message(10, ADA, &[], "status?");
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.sessions.created()[0].0.as_str(), "zulip:dm:7");
    let sent_to = bot.zulip.sent_to();
    assert!(matches!(
        &sent_to[..],
        [Conversation::Direct { participants }]
            if participants.len() == 1 && participants[0].id == ADA
    ));
}

#[tokio::test]
async fn a_long_answer_is_sent_in_parts() {
    let trigger = mention(10);
    let answer = "line one is here\nline two is here\nline three is here";
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_end("exe-1", completed("exe-1", Some(answer))),
    );
    bot.set_server_settings(ServerSettings {
        max_message_length: 20,
        ..SERVER
    });

    bot.handle(trigger).await;

    assert_eq!(
        bot.zulip.sent(),
        ["line one is here", "line two is here", "line three is here"]
    );
}

#[tokio::test]
async fn an_answer_without_text_says_so() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_end("exe-1", completed("exe-1", Some("  "))),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [EMPTY_ANSWER_NOTICE]);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test(start_paused = true)]
async fn typing_is_renewed_while_the_execution_runs() {
    let trigger = mention(10);
    let bot = Arc::new(bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().holding_executions(),
    ));
    let run = tokio::spawn({
        let bot = bot.clone();
        async move { bot.handle(trigger).await }
    });
    while bot.sessions.waits() == 0 {
        tokio::task::yield_now().await;
    }

    // The fake server asks for a renewal every 10 seconds.
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(bot.zulip.typing(), [true, true, true]);

    bot.sessions.release(1);
    run.await.unwrap();
    assert_eq!(bot.zulip.typing().last(), Some(&false));
}

#[tokio::test]
async fn a_zero_typing_period_from_the_server_still_answers() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.set_server_settings(ServerSettings {
        typing_refresh_interval: Duration::ZERO,
        ..SERVER
    });

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn the_same_message_is_handled_once() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );

    bot.handle(trigger.clone()).await;
    bot.handle(trigger).await;

    assert_eq!(bot.sessions.executed().len(), 1);
    assert_eq!(bot.zulip.sent().len(), 1);
}
