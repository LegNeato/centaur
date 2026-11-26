use super::*;

fn stop(id: u64) -> Message {
    topic_message(id, ADA, "@**Centaur** stop", true)
}

#[tokio::test]
async fn a_stop_interrupts_the_running_execution_and_gets_a_check_mark() {
    let trigger = stop(20);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_running_execution(),
    );

    bot.handle(trigger).await;

    let interrupts = bot.sessions.interrupts();
    assert_eq!(interrupts.len(), 1);
    assert_eq!(interrupts[0].0.as_str(), "zulip:channel:42:deploys");
    assert_eq!(interrupts[0].1, "Stopped from Zulip by Ada Lovelace");
    assert_eq!(bot.zulip.reactions(20.into()), ["check"]);
    assert!(bot.sessions.created().is_empty());
    assert!(bot.zulip.sent().is_empty());
}

#[tokio::test]
async fn a_stop_without_a_running_execution_changes_nothing() {
    let trigger = stop(20);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.sessions.interrupts().len(), 1);
    assert!(bot.zulip.reactions(20.into()).is_empty());
    assert!(bot.zulip.sent().is_empty());
}

#[tokio::test]
async fn the_scan_does_not_replay_a_stop() {
    let trigger = stop(20);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_running_execution(),
    );

    bot.handle_recovered(trigger).await;

    assert!(bot.sessions.interrupts().is_empty());
    assert!(bot.sessions.created().is_empty());
}

#[tokio::test]
async fn an_interrupted_execution_posts_no_answer_and_gets_a_stop_sign() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_end(
            "exe-1",
            ExecutionEnd::Cancelled(ExecutionCancelled {
                execution_id: "exe-1".to_owned(),
                thread_key: ThreadKey::parse("zulip:channel:42:deploys").unwrap(),
                reason: "turn_interrupted".to_owned(),
            }),
        ),
    );

    bot.handle(trigger).await;

    assert!(bot.zulip.sent().is_empty());
    assert_eq!(bot.zulip.reactions(10.into()), ["stop_sign"]);
}
