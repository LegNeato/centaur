//! Builders and in-memory fakes for the bot's tests.

pub mod fake_sessions;
pub mod fake_zulip;

pub use fake_sessions::FakeSessions;
pub use fake_zulip::FakeZulip;

use std::time::Duration;

use centaur_session_core::HarnessType;

use crate::{
    bot::Settings,
    footer::{HarnessDefaults, MetadataMode, SandboxDefaults},
    policy::Allowlist,
    selection::{ChannelDefaults, Choices, ModelCatalog},
    turn::ExecutionLimits,
};
use centaur_zulip::{
    ChannelId, Conversation, Message, MessageId, MessageRecipient, Participant, Reaction, UserId,
};

pub const BOT: UserId = UserId(99);
pub const BOT_NAME: &str = "Centaur";
pub const ADA: UserId = UserId(7);
pub const GRACE: UserId = UserId(9);
pub const ENGINEERING: ChannelId = ChannelId(42);
/// The Unix send time of every built message. A test can change it.
pub const SENT_AT: u64 = 1_700_000_000;

/// The choices of the test deployment. Codex is the default harness. Every
/// harness is enabled. The catalog and the channel defaults are empty.
pub fn choices() -> Choices {
    Choices::new(
        HarnessType::Codex,
        Allowlist::Everyone,
        ModelCatalog::default(),
        ChannelDefaults::default(),
    )
    .unwrap()
}

/// The settings of the test bot. Answers have no metadata line unless a test
/// asks for one.
pub fn settings() -> Settings {
    Settings {
        choices: choices(),
        metadata: MetadataMode::Never,
        harness_defaults: HarnessDefaults::new(SandboxDefaults::default()),
        context_messages: 50,
        limits: ExecutionLimits::default(),
        recovery_window: Duration::from_secs(3600),
        file_bytes: 1024,
    }
}

pub fn participant(id: UserId) -> Participant {
    Participant {
        id,
        full_name: name_of(id).to_owned(),
    }
}

pub fn name_of(id: UserId) -> &'static str {
    match id {
        BOT => BOT_NAME,
        ADA => "Ada Lovelace",
        GRACE => "Grace Hopper",
        _ => "Someone",
    }
}

/// A message in the `Deploys` topic of the engineering channel.
pub fn topic_message(id: u64, sender: UserId, content: &str, mentions_bot: bool) -> Message {
    Message {
        id: MessageId(id),
        sender_id: sender,
        sender_full_name: name_of(sender).to_owned(),
        content: content.to_owned(),
        timestamp: SENT_AT,
        recipient: MessageRecipient::Topic {
            channel_id: ENGINEERING,
            channel_name: "engineering".to_owned(),
            topic: "Deploys".to_owned(),
        },
        mentions_bot,
        reactions: Vec::new(),
    }
}

/// A direct message from `sender` to the bot and `others`.
pub fn direct_message(id: u64, sender: UserId, others: &[UserId], content: &str) -> Message {
    let participants = [sender, BOT]
        .into_iter()
        .chain(others.iter().copied())
        .map(participant)
        .collect();
    Message {
        id: MessageId(id),
        sender_id: sender,
        sender_full_name: name_of(sender).to_owned(),
        content: content.to_owned(),
        timestamp: SENT_AT,
        recipient: MessageRecipient::Direct { participants },
        mentions_bot: false,
        reactions: Vec::new(),
    }
}

pub fn conversation(message: &Message) -> Conversation {
    Conversation::of(message, BOT).expect("test message has a conversation")
}

pub fn reacted(mut message: Message, emoji_names: &[&str]) -> Message {
    message
        .reactions
        .extend(emoji_names.iter().map(|emoji_name| Reaction {
            emoji_name: (*emoji_name).to_owned(),
            user_id: BOT,
        }));
    message
}
