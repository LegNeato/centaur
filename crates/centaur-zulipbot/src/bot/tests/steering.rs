use super::*;

#[tokio::test]
async fn a_steered_message_gets_the_status_of_the_answer_it_joined() {
    let first = mention(10);
    let second = topic_message(11, GRACE, "@**Centaur** also check staging", true);
    let bot = Arc::new(bot(
        FakeZulip::default().with_messages([first.clone(), second.clone()]),
        FakeSessions::default()
            .holding_executions()
            .with_steering(steering(SteeringStatus::NoActiveExecution, None))
            .with_steering(steering(SteeringStatus::Delivered, Some("exe-1"))),
    ));
    let first_run = tokio::spawn({
        let bot = bot.clone();
        async move { bot.handle(first).await }
    });
    while bot.sessions.waits() == 0 {
        tokio::task::yield_now().await;
    }
    bot.handle(second).await;

    assert_eq!(bot.zulip.reactions(11.into()), ["time_ticking"]);
    assert_eq!(bot.sessions.executed().len(), 1);

    bot.sessions.release(1);
    first_run.await.unwrap();

    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
    assert_eq!(bot.zulip.reactions(11.into()), ["check"]);
}

#[tokio::test]
async fn a_message_that_steers_an_ended_execution_gets_its_final_status() {
    let first = mention(10);
    let second = topic_message(11, GRACE, "@**Centaur** also check staging", true);
    let bot = bot(
        FakeZulip::default().with_messages([first.clone(), second.clone()]),
        FakeSessions::default()
            .with_steering(steering(SteeringStatus::NoActiveExecution, None))
            .with_steering(steering(SteeringStatus::Delivered, Some("exe-1"))),
    );
    bot.handle(first).await;

    // api-rs delivered the steer just before the execution ended.
    bot.handle(second).await;

    assert_eq!(bot.zulip.reactions(11.into()), ["check"]);
}

#[tokio::test]
async fn a_steering_message_with_flags_says_that_they_do_not_apply() {
    let trigger = topic_message(11, GRACE, "@**Centaur** --opus also check staging", true);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_steering(steering(SteeringStatus::Delivered, Some("exe-1"))),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [STEERED_FLAGS_NOTICE]);
    let stored = &bot.sessions.appended()[0].1.messages;
    assert_eq!(
        stored.last().unwrap().parts[0]["text"],
        "Grace Hopper (Zulip user 9):\n@**Centaur** also check staging"
    );
}

#[tokio::test]
async fn a_steering_message_without_flags_posts_nothing() {
    let trigger = topic_message(11, GRACE, "@**Centaur** also check staging", true);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_steering(steering(SteeringStatus::Delivered, Some("exe-1"))),
    );

    bot.handle(trigger).await;

    assert!(bot.zulip.sent().is_empty());
}

#[tokio::test]
async fn an_unknown_steering_delivery_does_not_start_a_second_execution() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default()
            .with_steering(steering(SteeringStatus::DeliveryUnknown, Some("exe-9"))),
    );

    bot.handle(trigger).await;

    assert!(bot.sessions.executed().is_empty());
    assert_eq!(bot.zulip.reactions(10.into()), ["time_ticking"]);
}

#[tokio::test]
async fn an_undelivered_steer_starts_an_execution() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_steering(steering(SteeringStatus::NotDelivered, None)),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.sessions.executed().len(), 1);
    assert_eq!(bot.zulip.reactions(10.into()), ["check"]);
}
