use super::*;

#[tokio::test]
async fn a_message_the_policy_rejects_touches_nothing() {
    let chatter = topic_message(10, ADA, "no mention here", false);
    let bot = bot(
        FakeZulip::default().with_messages([chatter.clone()]),
        FakeSessions::default(),
    );

    bot.handle(chatter).await;

    assert!(bot.sessions.created().is_empty());
    assert!(bot.zulip.reactions(10.into()).is_empty());
}

#[tokio::test]
async fn another_bot_is_ignored_unless_allowed() {
    let from_bot = topic_message(10, GRACE, "@**Centaur** loop?", true);
    let bot = bot(
        FakeZulip::default()
            .with_messages([from_bot.clone()])
            .with_bot_account(GRACE),
        FakeSessions::default(),
    );

    bot.handle(from_bot).await;

    assert!(bot.sessions.created().is_empty());
}
