use super::*;

#[tokio::test]
async fn the_scan_finds_working_and_recent_unmarked_messages() {
    let working = reacted(mention(10), &["eyes"]);
    let answered = reacted(mention(11), &["eyes", "check"]);
    let steered = reacted(mention(12), &["time_ticking"]);
    let unmarked = mention(13);
    let mut old_unmarked = mention(14);
    old_unmarked.timestamp = SENT_AT - 7200;
    let mut old_working = reacted(direct_message(15, ADA, &[], "hi"), &["eyes"]);
    old_working.timestamp = SENT_AT - 86_400;
    let own_reply = direct_message(16, BOT, &[ADA], "an earlier answer");
    let bot = bot(
        FakeZulip::default().with_messages([
            working,
            answered,
            steered,
            unmarked,
            old_unmarked,
            old_working,
            own_reply,
        ]),
        FakeSessions::default(),
    );

    let now = UNIX_EPOCH + Duration::from_secs(SENT_AT + 60);
    let unanswered = bot.unanswered_messages(now).await.unwrap();

    let ids = unanswered
        .iter()
        .map(|message| message.id.0)
        .collect::<Vec<_>>();
    assert_eq!(ids, [10, 13]);
}

#[tokio::test]
async fn the_scan_pages_through_every_mention_in_the_window() {
    let mentions = (1..=1500).map(mention).collect::<Vec<_>>();
    let bot = bot(
        FakeZulip::default().with_messages(mentions),
        FakeSessions::default(),
    );

    let now = UNIX_EPOCH + Duration::from_secs(SENT_AT);
    let unanswered = bot.unanswered_messages(now).await.unwrap();

    assert_eq!(unanswered.len(), 1500);
    assert_eq!(unanswered.first().map(|message| message.id.0), Some(1));
    assert_eq!(unanswered.last().map(|message| message.id.0), Some(1500));
}

#[tokio::test]
async fn resuming_a_working_message_rejoins_its_execution() {
    let working = reacted(mention(10), &["eyes"]);
    let bot = bot(
        FakeZulip::default().with_messages([working.clone()]),
        FakeSessions::default(),
    );

    bot.handle(working).await;

    assert_eq!(
        bot.sessions.executed()[0].1.idempotency_key.as_deref(),
        Some("10")
    );
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
    assert_eq!(bot.zulip.sent().len(), 1);
}
