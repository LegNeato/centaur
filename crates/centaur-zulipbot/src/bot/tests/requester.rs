use super::*;
use centaur_zulip::{Profile, ProfileFieldId};

#[tokio::test]
async fn the_github_login_from_the_profile_credits_the_requester() {
    let trigger = mention(10);
    let bot = bot(
        FakeZulip::default()
            .with_messages([trigger.clone()])
            .with_profile(ADA, Profile::from([(ProfileFieldId(5), "octocat")])),
        FakeSessions::default(),
    );
    bot.set_server_settings(ServerSettings {
        github_fields: vec![ProfileFieldId(5)],
        ..SERVER
    });

    bot.handle(trigger).await;

    let line: serde_json::Value =
        serde_json::from_str(&bot.sessions.executed()[0].1.input_lines[0]).unwrap();
    let context = line["message"]["content"][0]["text"].as_str().unwrap();
    assert!(context.contains("`Prompted by: @octocat`"), "{context}");
}
