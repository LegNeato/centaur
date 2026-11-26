use super::*;

#[tokio::test]
async fn messages_of_one_conversation_hand_off_in_the_order_of_receipt() {
    let first = mention(10);
    let second = topic_message(11, ADA, "@**Centaur** and the logs?", true);
    let bot = Arc::new(bot(
        FakeZulip::default().with_messages([first.clone(), second.clone()]),
        FakeSessions::default(),
    ));
    let first = bot.receive(first).unwrap();
    let second = bot.receive(second).unwrap();

    // The second message starts first. It waits for the turn of the
    // first one.
    let later = tokio::spawn({
        let bot = bot.clone();
        async move { bot.process(second).await }
    });
    tokio::task::yield_now().await;
    assert!(bot.sessions.appended().is_empty());
    bot.process(first).await;
    later.await.unwrap();

    let triggers = bot
        .sessions
        .executed()
        .into_iter()
        .map(|(_, request)| request.idempotency_key.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(triggers, ["10", "11"]);
}

#[tokio::test]
async fn other_conversations_do_not_wait() {
    let topic = mention(10);
    let direct = direct_message(11, ADA, &[], "status?");
    let bot = bot(
        FakeZulip::default().with_messages([topic.clone(), direct.clone()]),
        FakeSessions::default(),
    );
    let _waiting = bot.receive(topic).unwrap();

    bot.handle(direct).await;

    assert_eq!(bot.sessions.executed().len(), 1);
    assert_eq!(bot.zulip.reactions(11.into()), ["check"]);
}

#[tokio::test]
async fn a_message_in_progress_is_not_received_again() {
    let trigger = mention(10);
    let bot = Arc::new(bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().holding_executions(),
    ));
    let run = tokio::spawn({
        let bot = bot.clone();
        let trigger = trigger.clone();
        async move { bot.handle(trigger).await }
    });
    while bot.sessions.waits() == 0 {
        tokio::task::yield_now().await;
    }

    assert!(bot.receive_recovered(trigger.clone()).is_none());
    bot.sessions.release(1);
    run.await.unwrap();
    assert!(bot.receive_recovered(trigger).is_none());
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn a_message_the_policy_rejects_is_not_recorded() {
    let bot = bot(FakeZulip::default(), FakeSessions::default());
    let unaddressed = topic_message(10, ADA, "the deploy is red", false);

    assert!(bot.receive(unaddressed.clone()).is_none());
    // The rejected message took no claim. The same ID with a mention is
    // received.
    assert!(
        bot.receive(Message {
            mentions_bot: true,
            ..unaddressed
        })
        .is_some()
    );
}
