use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use thiserror::Error;

use super::{ChannelId, MessageId, UserId};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(try_from = "WireMessage")]
pub struct Message {
    pub id: MessageId,
    pub sender_id: UserId,
    pub sender_full_name: String,
    /// The Markdown source. Centaur registers and fetches with
    /// `apply_markdown=false`. Zulip therefore does not render the content to
    /// HTML.
    pub content: String,
    /// Unix time in seconds.
    pub timestamp: u64,
    pub recipient: MessageRecipient,
    /// True when the message @-mentions the reading user directly or through
    /// a user group. A wildcard mention like `@**all**` does not count.
    pub mentions_bot: bool,
    pub reactions: Vec<Reaction>,
}

impl Message {
    /// Returns true when `user` reacted with `emoji_name`.
    pub fn has_reaction(&self, user: UserId, emoji_name: &str) -> bool {
        self.reactions
            .iter()
            .any(|reaction| reaction.user_id == user && reaction.emoji_name == emoji_name)
    }

    /// Returns the time the message was sent.
    pub fn sent_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.timestamp)
    }
}

/// The recipient of a message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageRecipient {
    Topic {
        channel_id: ChannelId,
        channel_name: String,
        /// The topic spelling of the sender. Zulip matches topics without regard
        /// to case.
        topic: String,
    },
    /// Every participant in the order of the server. The list includes the
    /// sender and the reading user.
    Direct { participants: Vec<Participant> },
}

/// One participant of a direct-message conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Participant {
    pub id: UserId,
    pub full_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Reaction {
    pub emoji_name: String,
    pub user_id: UserId,
}

/// The message flags that Centaur reads. Each flag describes the message for
/// the reading user.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MessageFlag {
    Mentioned,
    #[serde(other)]
    Other,
}

impl MessageFlag {
    pub(super) fn mention(flags: &[Self]) -> bool {
        flags.contains(&Self::Mentioned)
    }
}

/// A message in the Zulip wire format. The `type` and `display_recipient`
/// fields together describe the recipient.
#[derive(Deserialize)]
struct WireMessage {
    id: MessageId,
    sender_id: UserId,
    sender_full_name: String,
    content: String,
    timestamp: u64,
    #[serde(rename = "type")]
    kind: WireMessageKind,
    stream_id: Option<ChannelId>,
    #[serde(default)]
    subject: String,
    display_recipient: WireDisplayRecipient,
    /// The flags of a `GET /messages` result. An event carries its flags next
    /// to the message instead.
    #[serde(default)]
    flags: Vec<MessageFlag>,
    #[serde(default)]
    reactions: Vec<Reaction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireMessageKind {
    Stream,
    Private,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireDisplayRecipient {
    ChannelName(String),
    Participants(Vec<Participant>),
}

#[derive(Debug, Error)]
enum MessageShapeError {
    #[error("channel message has no channel ID")]
    MissingChannelId,
    #[error("channel message lists direct-message participants")]
    ChannelWithParticipants,
    #[error("direct message names a channel")]
    DirectWithChannelName,
}

impl TryFrom<WireMessage> for Message {
    type Error = MessageShapeError;

    fn try_from(wire: WireMessage) -> Result<Self, Self::Error> {
        let recipient = match (wire.kind, wire.display_recipient) {
            (WireMessageKind::Stream, WireDisplayRecipient::ChannelName(channel_name)) => {
                MessageRecipient::Topic {
                    channel_id: wire.stream_id.ok_or(MessageShapeError::MissingChannelId)?,
                    channel_name,
                    topic: wire.subject,
                }
            }
            (WireMessageKind::Private, WireDisplayRecipient::Participants(participants)) => {
                MessageRecipient::Direct { participants }
            }
            (WireMessageKind::Stream, WireDisplayRecipient::Participants(_)) => {
                return Err(MessageShapeError::ChannelWithParticipants);
            }
            (WireMessageKind::Private, WireDisplayRecipient::ChannelName(_)) => {
                return Err(MessageShapeError::DirectWithChannelName);
            }
        };
        Ok(Self {
            id: wire.id,
            sender_id: wire.sender_id,
            sender_full_name: wire.sender_full_name,
            content: wire.content,
            timestamp: wire.timestamp,
            recipient,
            mentions_bot: MessageFlag::mention(&wire.flags),
            reactions: wire.reactions,
        })
    }
}
