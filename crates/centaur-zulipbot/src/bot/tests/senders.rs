use super::*;

#[tokio::test]
async fn a_guest_is_ignored_unless_allowed() {
    let from_guest = topic_message(10, GRACE, "@**Centaur** hello", true);
    let bot = bot(
        FakeZulip::default()
            .with_messages([from_guest.clone()])
            .with_guest_account(GRACE),
        FakeSessions::default(),
    );

    bot.handle(from_guest).await;

    assert!(bot.sessions.created().is_empty());
    assert!(bot.zulip.reactions(10.into()).is_empty());
}

#[tokio::test]
async fn an_allowed_guest_is_answered() {
    let from_guest = topic_message(10, GRACE, "@**Centaur** hello", true);
    let mut bot = bot(
        FakeZulip::default()
            .with_messages([from_guest.clone()])
            .with_guest_account(GRACE),
        FakeSessions::default(),
    );
    bot.policy.trigger_guests = Allowlist::Only(BTreeSet::from([GRACE]));

    bot.handle(from_guest).await;

    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}

#[tokio::test]
async fn a_sender_that_cannot_be_looked_up_is_ignored() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_unknown_user(ADA),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert!(bot.sessions.created().is_empty());
}
