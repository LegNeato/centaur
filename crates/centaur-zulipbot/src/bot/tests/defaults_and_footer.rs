use super::*;
use crate::{footer::MetadataMode, selection::SELECTION_ERROR_NOTICE};

fn channel_choices(defaults: &str) -> Choices {
    Choices::new(
        HarnessType::Codex,
        Allowlist::Everyone,
        ModelCatalog::default(),
        defaults.parse().unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_channel_default_selects_the_harness_and_model_of_a_new_session() {
    let trigger = mention(10);
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.settings.choices = channel_choices(r#"{"42": {"harness": "claude", "model": "sonnet"}}"#);

    bot.handle(trigger).await;

    let created = bot.sessions.created();
    assert_eq!(created[0].1.harness_type, HarnessType::ClaudeCode);
    assert_eq!(created[0].1.on_harness_conflict, OnHarnessConflict::Reject);
    let line: serde_json::Value =
        serde_json::from_str(&bot.sessions.executed()[0].1.input_lines[0]).unwrap();
    assert_eq!(line["model"], "claude-sonnet-5-5");
}

#[tokio::test]
async fn a_channel_default_model_does_not_follow_the_session_to_another_harness() {
    let trigger = mention(10);
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_existing_session(HarnessType::Codex),
    );
    bot.settings.choices = channel_choices(r#"{"42": {"harness": "claude", "model": "sonnet"}}"#);

    bot.handle(trigger).await;

    assert_eq!(
        bot.sessions.created().last().unwrap().1.harness_type,
        HarnessType::Codex
    );
    let line: serde_json::Value =
        serde_json::from_str(&bot.sessions.executed()[0].1.input_lines[0]).unwrap();
    assert!(line.get("model").is_none(), "{line}");
    assert_eq!(bot.zulip.sent(), [default_answer("exe-1")]);
}

#[tokio::test]
async fn a_rejected_request_opens_a_new_session_with_its_persona() {
    let trigger = topic_message(
        10,
        ADA,
        "@**Centaur** --model shared --persona ops go",
        true,
    );
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.settings.choices = Choices::new(
        HarnessType::Codex,
        Allowlist::Everyone,
        r#"{"a": {"models": {"shared": {}}}, "b": {"models": {"shared": {}}}}"#
            .parse()
            .unwrap(),
        ChannelDefaults::default(),
    )
    .unwrap();

    bot.handle(trigger).await;

    let created = bot.sessions.created();
    assert_eq!(created[0].1.persona_id.as_deref(), Some("ops"));
    assert_eq!(created[0].1.on_harness_conflict, OnHarnessConflict::Reject);
    assert!(bot.sessions.executed().is_empty());
}

#[tokio::test]
async fn an_unresolvable_catalog_model_is_rejected_with_a_notice() {
    let trigger = topic_message(10, ADA, "@**Centaur** --model shared go", true);
    let mut bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default(),
    );
    bot.settings.choices = Choices::new(
        HarnessType::Codex,
        Allowlist::Everyone,
        r#"{"a": {"models": {"shared": {}}}, "b": {"models": {"shared": {}}}}"#
            .parse()
            .unwrap(),
        ChannelDefaults::default(),
    )
    .unwrap();

    bot.handle(trigger).await;

    assert_eq!(bot.zulip.sent(), [SELECTION_ERROR_NOTICE]);
    assert_eq!(bot.zulip.reactions(10.into()), ["cross_mark"]);
    assert!(bot.sessions.executed().is_empty());
}

#[tokio::test]
async fn the_first_answer_and_a_model_change_get_the_metadata_line() {
    let first = mention(10);
    let later = topic_message(12, ADA, "@**Centaur** and now?", true);
    let flagged = topic_message(14, ADA, "@**Centaur** --model gpt-5.4-pro and now?", true);
    let mut bot = bot(
        FakeZulip::default().with_messages([first.clone(), later.clone(), flagged.clone()]),
        FakeSessions::default(),
    );
    bot.settings.metadata = MetadataMode::First;

    bot.handle(first).await;
    bot.handle(later).await;
    bot.handle(flagged).await;

    assert_eq!(
        bot.zulip.sent(),
        [
            format!("{}\n\n*Sol 5.6 · Codex · Medium*", default_answer("exe-1")),
            default_answer("exe-2"),
            format!(
                "{}\n\n*GPT 5.4 Pro · Codex · Medium*",
                default_answer("exe-3")
            ),
        ]
    );
}
