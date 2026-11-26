use centaur_session_core::{HarnessType, MessageRole, api::OnHarnessConflict};
use serde_json::{Value, json};

use super::{ExecutionLimits, Request, Turn, create_request, thread_key};
use crate::{
    files::Files,
    flags::Flags,
    requester::Requester,
    selection::Selection,
    testing::{
        ADA, BOT, FakeZulip, GRACE, choices, conversation, direct_message, reacted, topic_message,
    },
};
use centaur_zulip::MessageId;

/// The requester of every test turn. Ada has no GitHub login.
static ADA_REQUESTER: std::sync::LazyLock<Requester> = std::sync::LazyLock::new(|| Requester {
    user_id: ADA,
    full_name: "Ada Lovelace".to_owned(),
    github_login: None,
});

fn request<'a>(
    trigger: &'a centaur_zulip::Message,
    flags: &'a Flags,
    selection: &'a Selection,
) -> Request<'a> {
    Request {
        message: trigger,
        text: &flags.text,
        selection,
        requester: &ADA_REQUESTER,
    }
}

fn input_line(turn: &Turn<'_>, files: &Files) -> Value {
    let request = turn.execute_request(ExecutionLimits::default(), files);
    assert_eq!(request.input_lines.len(), 1);
    serde_json::from_str(&request.input_lines[0]).unwrap()
}

#[test]
fn thread_keys_name_the_topic_or_the_direct_participants() {
    let topic = topic_message(10, ADA, "@**Centaur** hi", true);
    assert_eq!(
        thread_key(&conversation(&topic)).unwrap().as_str(),
        "zulip:channel:42:deploys"
    );
    let direct = direct_message(11, GRACE, &[ADA], "hi");
    assert_eq!(
        thread_key(&conversation(&direct)).unwrap().as_str(),
        "zulip:dm:7,9"
    );
}

#[test]
fn context_starts_after_the_last_handled_message() {
    let trigger = topic_message(10, ADA, "@**Centaur** and now?", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let history = vec![
        topic_message(1, GRACE, "old chatter", false),
        reacted(
            topic_message(2, ADA, "@**Centaur** first", true),
            &["check"],
        ),
        topic_message(3, BOT, "first answer", false),
        topic_message(4, GRACE, "I disagree\nstrongly", false),
    ];

    let flags = Flags::parse(&trigger.content);

    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        history,
        BOT,
        false,
    );

    let appended = turn.append_request(&Files::default()).messages;
    let ids = appended
        .iter()
        .map(|message| message.client_message_id.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["4", "10"]);
    assert!(
        appended
            .iter()
            .all(|message| message.role == MessageRole::User)
    );
    assert_eq!(appended[0].metadata["user_id"], "9");

    let context = input_line(&turn, &Files::default())["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        context.contains("from Ada Lovelace (Zulip user 7)"),
        "{context}"
    );
    assert!(context.contains("the topic \"Deploys\" in the channel #engineering"));
    assert!(context.contains("Messages sent since the previous request"));
    assert!(context.contains("1. Grace Hopper (user 9):\n   I disagree\n   strongly"));
    assert!(!context.contains("old chatter"));
    assert!(!context.contains("first answer"));
}

/// Returns the IDs of the messages that `history` adds to a new request.
fn stored_ids(history: Vec<centaur_zulip::Message>) -> Vec<String> {
    let trigger = topic_message(10, ADA, "@**Centaur** and now?", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let flags = Flags::parse(&trigger.content);
    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        history,
        BOT,
        false,
    );
    turn.append_request(&Files::default())
        .messages
        .into_iter()
        .map(|message| message.client_message_id.unwrap())
        .collect()
}

#[test]
fn a_stop_command_neither_ends_the_context_nor_joins_it() {
    let history = vec![
        reacted(
            topic_message(2, ADA, "@**Centaur** first", true),
            &["check"],
        ),
        topic_message(3, BOT, "first answer", false),
        topic_message(4, GRACE, "the logs are in #ops", false),
        reacted(topic_message(5, ADA, "@**Centaur** stop", true), &["check"]),
    ];

    assert_eq!(stored_ids(history), ["4", "10"]);
}

#[test]
fn a_failed_request_does_not_end_the_context() {
    let history = vec![
        reacted(
            topic_message(2, ADA, "@**Centaur** first", true),
            &["check"],
        ),
        topic_message(3, BOT, "first answer", false),
        topic_message(4, GRACE, "the logs are in #ops", false),
        reacted(
            topic_message(5, ADA, "@**Centaur** --model nope again", true),
            &["cross_mark"],
        ),
    ];

    assert_eq!(stored_ids(history), ["4", "5", "10"]);
}

#[test]
fn a_stopped_or_steered_request_ends_the_context() {
    for emoji in ["stop_sign", "time_ticking"] {
        let history = vec![
            topic_message(4, GRACE, "the logs are in #ops", false),
            reacted(topic_message(5, ADA, "@**Centaur** look", true), &[emoji]),
            topic_message(6, GRACE, "and staging", false),
        ];

        assert_eq!(stored_ids(history), ["6", "10"], "{emoji}");
    }
}

#[test]
fn context_without_a_handled_message_is_the_fetched_history() {
    let trigger = topic_message(10, ADA, "@**Centaur** summarize", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let history = vec![
        topic_message(1, GRACE, "one", false),
        topic_message(2, ADA, "two", false),
    ];

    let flags = Flags::parse(&trigger.content);

    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        history,
        BOT,
        false,
    );

    assert_eq!(turn.append_request(&Files::default()).messages.len(), 3);
    let context = input_line(&turn, &Files::default())["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(context.contains("Earlier messages in this conversation"));
}

#[test]
fn a_direct_message_without_history_names_only_the_requester() {
    let trigger = direct_message(10, ADA, &[], "hello");
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();

    let flags = Flags::parse(&trigger.content);

    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        Vec::new(),
        BOT,
        false,
    );

    let context = input_line(&turn, &Files::default())["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(context.contains("in a direct message to you"));
    assert!(!context.contains("oldest first"));
}

#[test]
fn execute_request_keys_on_the_trigger_and_carries_its_text() {
    let trigger = topic_message(10, ADA, "@**Centaur** deploy status?", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let flags = Flags::parse(&trigger.content);
    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        Vec::new(),
        BOT,
        false,
    );

    let request = turn.execute_request(
        ExecutionLimits {
            idle_timeout_ms: Some(1_000),
            max_duration_ms: Some(60_000),
        },
        &Files::default(),
    );

    assert_eq!(request.idempotency_key.as_deref(), Some("10"));
    assert_eq!(request.idle_timeout_ms, Some(1_000));
    assert_eq!(request.max_duration_ms, Some(60_000));
    let line: Value = serde_json::from_str(&request.input_lines[0]).unwrap();
    assert_eq!(line["type"], "user");
    assert_eq!(line["thread_key"], "zulip:channel:42:deploys");
    assert_eq!(line["trace_metadata"]["action"], "execute");
    assert_eq!(
        line["message"]["content"][1],
        json!({"type": "text", "text": "@**Centaur** deploy status?"})
    );
}

#[test]
fn create_request_names_the_conversation() {
    let trigger = direct_message(10, ADA, &[GRACE], "@**Centaur** hi");
    let conversation = conversation(&trigger);
    let request = create_request(&conversation, HarnessType::Codex, None, false);

    assert_eq!(request.harness_type, HarnessType::Codex);
    assert_eq!(request.on_harness_conflict, OnHarnessConflict::Reject);
    let metadata = request.metadata.unwrap();
    assert_eq!(metadata["platform"], "zulip");
    assert_eq!(
        metadata["zulip_conversation_name"],
        "Ada Lovelace, Grace Hopper"
    );
}

#[tokio::test]
async fn files_follow_each_stored_message_and_the_request_once() {
    const LOG: &str = "/user_uploads/2/aa/bbb/log.txt";
    const SCREEN: &str = "/user_uploads/2/cc/ddd/screen.png";
    let earlier = topic_message(4, GRACE, &format!("[log.txt]({LOG})"), false);
    let trigger = topic_message(
        10,
        ADA,
        &format!("@**Centaur** see [screen.png]({SCREEN}) and > [log.txt]({LOG})"),
        true,
    );
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let flags = Flags::parse(&trigger.content);
    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        vec![earlier],
        BOT,
        false,
    );
    let zulip = FakeZulip::default()
        .with_file(MessageId(4), LOG, "text/plain", b"boom")
        .with_file(MessageId(10), SCREEN, "image/png", b"png")
        .with_file(MessageId(10), LOG, "text/plain", b"boom");

    let files = Files::fetch(&zulip, turn.by_file_priority(), 1024).await;

    assert_eq!(zulip.downloads(), [SCREEN, LOG]);
    let appended = turn.append_request(&files).messages;
    let kinds = |parts: &[Value]| {
        parts
            .iter()
            .map(|part| part["type"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(&appended[0].parts), ["text", "text", "attachment"]);
    assert_eq!(
        kinds(&appended[1].parts),
        ["text", "text", "attachment", "text", "attachment"]
    );
    let content = input_line(&turn, &files)["message"]["content"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(content.len(), 6);
    assert_eq!(
        content[2]["text"],
        format!("The Zulip upload {LOG} follows.")
    );
    assert_eq!(content[3]["name"], "log.txt");
    assert_eq!(
        content[4]["text"],
        format!("The Zulip upload {SCREEN} follows.")
    );
    assert_eq!(content[5]["attachment_type"], "image");
}

#[test]
fn flags_leave_the_stored_and_executed_text_and_ride_the_input_line() {
    let trigger = topic_message(10, ADA, "@**Centaur** --opus -rsn high fix it", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let flags = Flags::parse(&trigger.content);
    let selection = choices().select(&flags, None).unwrap();
    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        Vec::new(),
        BOT,
        false,
    );

    let stored = turn.append_request(&Files::default()).messages;
    assert_eq!(
        stored[0].parts[0]["text"],
        "Ada Lovelace (Zulip user 7):\n@**Centaur** fix it"
    );
    let line = input_line(&turn, &Files::default());
    assert_eq!(line["message"]["content"][1]["text"], "@**Centaur** fix it");
    assert_eq!(line["model"], "claude-opus-5-5");
    assert_eq!(line["reasoning"], "high");
    assert!(line.get("provider").is_none());
}

#[test]
fn a_turn_after_a_restart_lists_the_whole_history_with_the_answers() {
    let trigger = topic_message(10, ADA, "@**Centaur** --claude and now?", true);
    let conversation = conversation(&trigger);
    let key = thread_key(&conversation).unwrap();
    let history = vec![
        reacted(
            topic_message(2, ADA, "@**Centaur** first", true),
            &["check"],
        ),
        topic_message(3, BOT, "first answer", false),
    ];
    let flags = Flags::parse(&trigger.content);
    let selection = choices().select(&flags, None).unwrap();

    let turn = Turn::new(
        &key,
        &conversation,
        request(&trigger, &flags, &selection),
        history,
        BOT,
        true,
    );

    let context = input_line(&turn, &Files::default())["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        context.contains("moved to a new agent harness"),
        "{context}"
    );
    assert!(context.contains("1. Ada Lovelace (user 7):\n   @**Centaur** first"));
    assert!(context.contains("2. You, in an earlier answer:\n   first answer"));
    let stored = turn.append_request(&Files::default()).messages;
    let ids = stored
        .iter()
        .map(|message| message.client_message_id.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["2", "10"]);
}

#[test]
fn a_harness_flag_asks_for_a_restart() {
    let trigger = topic_message(10, ADA, "hi", true);

    let request = create_request(
        &conversation(&trigger),
        HarnessType::ClaudeCode,
        Some("reviewer".to_owned()),
        true,
    );

    assert_eq!(request.on_harness_conflict, OnHarnessConflict::Restart);
    assert_eq!(request.persona_id.as_deref(), Some("reviewer"));
}
