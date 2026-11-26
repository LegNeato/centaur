use super::*;

const LOG: &str = "/user_uploads/2/aa/bbb/deploy.log";

#[tokio::test]
async fn a_linked_file_reaches_the_execution_and_the_stored_message() {
    let trigger = topic_message(
        10,
        ADA,
        &format!("@**Centaur** why? [deploy.log]({LOG})"),
        true,
    );
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_file(10.into(), LOG, "text/plain", b"exit 1"),
        FakeSessions::default(),
    );

    bot.handle(trigger).await;

    let stored = &bot.sessions.appended()[0].1.messages[0].parts;
    assert_eq!(stored[2]["dataBase64"], "ZXhpdCAx");
    let line: serde_json::Value =
        serde_json::from_str(&bot.sessions.executed()[0].1.input_lines[0]).unwrap();
    let content = line["message"]["content"].as_array().unwrap();
    assert_eq!(content[3]["name"], "deploy.log");
    assert_eq!(content[3]["dataBase64"], "ZXhpdCAx");
}

#[tokio::test]
async fn a_steering_message_stores_its_file_for_the_running_execution() {
    let trigger = topic_message(
        11,
        ADA,
        &format!("@**Centaur** also [deploy.log]({LOG})"),
        true,
    );
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_file(11.into(), LOG, "text/plain", b"exit 1"),
        FakeSessions::default().with_steering(steering(SteeringStatus::Delivered, Some("exe-1"))),
    );

    bot.handle(trigger).await;

    assert!(bot.sessions.executed().is_empty());
    let stored = &bot.sessions.appended()[0].1.messages[0].parts;
    assert_eq!(stored[2]["type"], "attachment");
}
