use super::*;

#[tokio::test]
async fn the_bot_joins_an_unsubscribed_channel_and_answers() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_unsubscribed_channel(ENGINEERING, "engineering", true),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.subscribe_requests(), ["engineering"]);
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test]
async fn a_subscribed_channel_is_not_joined_again() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert!(bot.zulip.subscribe_requests().is_empty());
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn an_unreadable_channel_gets_a_direct_notice_and_no_work() {
    for zulip in [
        FakeZulip::default().with_hidden_channel(ENGINEERING, "engineering"),
        FakeZulip::default().with_unsubscribed_channel(ENGINEERING, "engineering", false),
    ] {
        let trigger = mention(10);
        let bot = bot(
            zulip.with_messages([trigger.clone()]),
            FakeSessions::default(),
        );

        bot.handle(trigger).await;

        assert_eq!(bot.zulip.sent(), [unreadable_channel_notice("engineering")]);
        assert!(matches!(
            &bot.zulip.sent_to()[..],
            [Conversation::Direct { participants }]
            if participants.len() == 1 && participants[0].id == ADA
        ));
        assert!(bot.zulip.reactions(10.into()).is_empty());
        assert!(bot.sessions.created().is_empty());
    }
}

#[tokio::test]
async fn the_scan_does_not_repeat_the_notice() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_hidden_channel(ENGINEERING, "engineering"),
        FakeSessions::default(),
    );

    bot.handle_recovered(trigger).await;

    assert!(bot.zulip.sent().is_empty());
    assert!(bot.sessions.created().is_empty());
}

#[tokio::test]
async fn the_policy_runs_before_a_join() {
    let guest_mention = topic_message(10, GRACE, "@**Centaur** hi", true);
    let bot = bot(
        FakeZulip::default()
            .with_guest_account(GRACE)
            .with_unsubscribed_channel(ENGINEERING, "engineering", true),
        FakeSessions::default(),
    );

    bot.handle(guest_mention).await;

    assert!(bot.zulip.subscribe_requests().is_empty());
}
