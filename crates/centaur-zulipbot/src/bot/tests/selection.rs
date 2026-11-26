use super::*;

fn input_line(bot: &TestBot) -> serde_json::Value {
    serde_json::from_str(&bot.sessions.executed()[0].1.input_lines[0]).unwrap()
}

#[tokio::test]
async fn a_harness_flag_restarts_a_session_on_another_harness_with_the_history() {
    let earlier = reacted(mention(5), &["check"]);
    let trigger = topic_message(10, ADA, "@**Centaur** --claude try again", true);
    let bot = bot(
        FakeZulip::default().with_messages([earlier, trigger.clone()]),
        FakeSessions::default().with_existing_session(HarnessType::Codex),
    );

    bot.handle(trigger).await;

    let created = bot.sessions.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].1.harness_type, HarnessType::ClaudeCode);
    let line = input_line(&bot);
    assert!(
        line["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("why did the deploy fail?")
    );
    assert_eq!(
        line["message"]["content"][1]["text"],
        "@**Centaur** try again"
    );
}

#[tokio::test]
async fn without_a_flag_the_session_keeps_its_harness() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_existing_session(HarnessType::ClaudeCode),
    );

    bot.handle(trigger).await;

    let harnesses = bot
        .sessions
        .created()
        .into_iter()
        .map(|(_, request)| request.harness_type)
        .collect::<Vec<_>>();
    assert_eq!(harnesses, [HarnessType::Codex, HarnessType::ClaudeCode]);
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn an_unavailable_persona_and_a_pinned_model_are_explained() {
    let trigger = topic_message(
        10,
        ADA,
        "@**Centaur** --persona=ghost --model gpt-6 go",
        true,
    );
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default()
            .offering_persona("reviewer")
            .with_persona_model("claude-opus-5-5"),
    );

    bot.handle(trigger).await;

    assert_eq!(
        bot.zulip.sent(),
        [
            "The persona `ghost` is not available. This conversation uses the default persona."
                .to_owned(),
            "The persona of this conversation runs the model `claude-opus-5-5`. The \
             `--model` flag has no effect."
                .to_owned(),
            default_answer("exe-1"),
        ]
    );
    assert_eq!(
        bot.sessions.created()[0].1.persona_id.as_deref(),
        Some("ghost")
    );
}

#[tokio::test]
async fn a_disabled_harness_flag_runs_on_the_current_harness_and_says_so() {
    let trigger = topic_message(10, ADA, "@**Centaur** --opus go", true);
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.settings.choices = Choices::new(
        HarnessType::Codex,
        Allowlist::Only(BTreeSet::from([HarnessType::Codex])),
        ModelCatalog::default(),
        ChannelDefaults::default(),
    )
    .unwrap();

    bot.handle(trigger).await;

    let created = bot.sessions.created();
    assert_eq!(created[0].1.harness_type, HarnessType::Codex);
    assert_eq!(created[0].1.on_harness_conflict, OnHarnessConflict::Reject);
    assert!(input_line(&bot).get("model").is_none());
    assert_eq!(
        bot.zulip.sent()[0],
        "The harness `claudecode` is not enabled. The request runs on the harness of the \
         conversation."
    );
}

#[tokio::test]
async fn a_resumed_message_does_not_repeat_its_notices() {
    let trigger = reacted(
        topic_message(10, ADA, "@**Centaur** --opus go", true),
        &["eyes"],
    );
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.settings.choices = Choices::new(
        HarnessType::Codex,
        Allowlist::Only(BTreeSet::from([HarnessType::Codex])),
        ModelCatalog::default(),
        ChannelDefaults::default(),
    )
    .unwrap();

    bot.handle_recovered(trigger).await;

    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn an_offered_persona_posts_no_notice() {
    let trigger = topic_message(10, ADA, "@**Centaur** --persona reviewer go", true);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().offering_persona("reviewer"),
    );

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
    assert_eq!(
        input_line(&bot)["message"]["content"][1]["text"],
        "@**Centaur** go"
    );
}
