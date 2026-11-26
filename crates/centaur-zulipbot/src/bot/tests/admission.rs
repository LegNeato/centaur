use super::*;

#[tokio::test]
async fn an_unapproved_conversation_gets_no_answer_and_no_reaction() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default().with_messages([trigger.clone()]),
        FakeSessions::default().with_create_error(ClientError::api(
            StatusCode::FORBIDDEN,
            r#"{"error":"forbidden","code":"session_principal_not_preapproved"}"#.to_owned(),
        )),
    );

    bot.handle(trigger).await;

    assert!(bot.zulip.sent().is_empty());
    assert!(bot.zulip.reactions(10.into()).is_empty());
    assert!(bot.sessions.executed().is_empty());
}
