use std::time::Duration;

use serde_json::json;

use super::{
    ChannelId, Event, EventId, EventKind, Message, MessageId, MessageRecipient, Participant,
    ProfileFieldId, Registration, User, UserId,
};

fn channel_message() -> serde_json::Value {
    json!({
        "id": 501,
        "sender_id": 7,
        "sender_full_name": "Ada Lovelace",
        "sender_email": "ada@example.test",
        "content": "@**Centaur** why did the deploy fail?",
        "content_type": "text/x-markdown",
        "timestamp": 1_700_000_000,
        "type": "stream",
        "stream_id": 42,
        "subject": "Deploys",
        "display_recipient": "engineering",
        "reactions": [{"emoji_name": "eyes", "emoji_code": "1f440", "reaction_type": "unicode_emoji", "user_id": 99}],
        "client": "website",
    })
}

#[test]
fn reads_a_channel_message_event_and_its_flags() {
    let event: Event = serde_json::from_value(json!({
        "id": 3,
        "type": "message",
        "message": channel_message(),
        "flags": ["mentioned", "read"],
    }))
    .unwrap();
    assert_eq!(event.id, EventId(3));

    let message = event.into_message().unwrap();
    assert_eq!(message.id, MessageId(501));
    assert_eq!(message.sender_id, UserId(7));
    assert!(message.mentions_bot);
    assert_eq!(
        message.recipient,
        MessageRecipient::Topic {
            channel_id: ChannelId(42),
            channel_name: "engineering".to_owned(),
            topic: "Deploys".to_owned(),
        }
    );
    assert!(message.has_reaction(UserId(99), "eyes"));
    assert!(!message.has_reaction(UserId(7), "eyes"));
}

#[test]
fn wildcard_mentions_do_not_mention_the_bot() {
    let event: Event = serde_json::from_value(json!({
        "id": 3,
        "type": "message",
        "message": channel_message(),
        "flags": ["stream_wildcard_mentioned", "wildcard_mentioned"],
    }))
    .unwrap();
    assert!(!event.into_message().unwrap().mentions_bot);
}

#[test]
fn reads_direct_message_participants() {
    let message: Message = serde_json::from_value(json!({
        "id": 502,
        "sender_id": 7,
        "sender_full_name": "Ada Lovelace",
        "content": "hello",
        "timestamp": 1_700_000_000,
        "type": "private",
        "subject": "",
        "display_recipient": [
            {"id": 7, "email": "ada@example.test", "full_name": "Ada Lovelace", "is_mirror_dummy": false},
            {"id": 99, "email": "centaur-bot@example.test", "full_name": "Centaur", "is_mirror_dummy": false},
        ],
        "flags": ["read"],
    }))
    .unwrap();
    assert_eq!(
        message.recipient,
        MessageRecipient::Direct {
            participants: vec![
                Participant {
                    id: UserId(7),
                    full_name: "Ada Lovelace".to_owned(),
                },
                Participant {
                    id: UserId(99),
                    full_name: "Centaur".to_owned(),
                },
            ],
        }
    );
    assert!(!message.mentions_bot);
}

#[test]
fn fetched_messages_carry_their_own_flags() {
    let mut message = channel_message();
    message["flags"] = json!(["mentioned"]);
    let message: Message = serde_json::from_value(message).unwrap();
    assert!(message.mentions_bot);
}

#[test]
fn rejects_a_channel_message_without_a_channel_id() {
    let mut message = channel_message();
    message.as_object_mut().unwrap().remove("stream_id");
    let error = serde_json::from_value::<Message>(message).unwrap_err();
    assert!(error.to_string().contains("no channel ID"), "{error}");
}

#[test]
fn rejects_a_direct_message_that_names_a_channel() {
    let mut message = channel_message();
    message["type"] = json!("private");
    let error = serde_json::from_value::<Message>(message).unwrap_err();
    assert!(error.to_string().contains("names a channel"), "{error}");
}

#[test]
fn heartbeats_and_unknown_events_carry_no_message() {
    let heartbeat: Event = serde_json::from_value(json!({"id": 4, "type": "heartbeat"})).unwrap();
    assert_eq!(heartbeat.kind, EventKind::Heartbeat);
    assert_eq!(heartbeat.into_message(), None);

    let other: Event =
        serde_json::from_value(json!({"id": 5, "type": "presence", "user_id": 7})).unwrap();
    assert_eq!(other.kind, EventKind::Other);
}

#[test]
fn reads_the_registration_and_realm_settings() {
    let registration: Registration = serde_json::from_value(json!({
        "result": "success",
        "queue_id": "fb67bf8a-c031-47cc-84cf-ed80accacda8",
        "last_event_id": -1,
        "zulip_feature_level": 400,
        "zulip_version": "11.0",
        "max_message_length": 10000,
        "server_typing_started_wait_period_milliseconds": 10000,
        "realm_name": "Example",
    }))
    .unwrap();
    assert_eq!(
        registration.queue_id.0,
        "fb67bf8a-c031-47cc-84cf-ed80accacda8"
    );
    assert_eq!(registration.last_event_id, EventId(-1));
    assert_eq!(registration.server.feature_level, 400);
    assert_eq!(registration.server.max_message_length, 10_000);
    assert_eq!(
        registration.server.typing_refresh_interval,
        Duration::from_secs(10)
    );
    assert!(registration.server.github_fields.is_empty());
}

#[test]
fn github_account_fields_come_before_fields_named_github() {
    let registration: Registration = serde_json::from_value(json!({
        "queue_id": "q",
        "last_event_id": -1,
        "zulip_feature_level": 400,
        "max_message_length": 10000,
        "server_typing_started_wait_period_milliseconds": 10000,
        "custom_profile_fields": [
            {"id": 3, "name": "My GitHub page", "type": 5, "field_data": ""},
            {"id": 4, "name": "Phone", "type": 1, "field_data": ""},
            {"id": 5, "name": "GitHub username", "type": 7, "field_data": "{\"subtype\":\"github\"}"},
            {"id": 6, "name": "Twitter", "type": 7, "field_data": "{\"subtype\":\"twitter\"}"},
        ],
    }))
    .unwrap();

    assert_eq!(
        registration.server.github_fields,
        [ProfileFieldId(5), ProfileFieldId(3)]
    );
}

#[test]
fn a_user_lookup_reads_the_profile_values() {
    let user: User = serde_json::from_value(json!({
        "user_id": 7,
        "full_name": "Ada Lovelace",
        "is_bot": false,
        "profile_data": {"5": {"value": "octocat", "rendered_value": "<p>octocat</p>"}},
    }))
    .unwrap();

    assert_eq!(user.profile.value(ProfileFieldId(5)), Some("octocat"));
    assert_eq!(user.profile.value(ProfileFieldId(6)), None);
}
