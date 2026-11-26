use serde_json::{Value, json};

use super::{ChannelId, Message, MessageRecipient, Participant, UserId};

/// The place that a new message goes to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Recipient {
    Topic {
        channel_id: ChannelId,
        topic: String,
    },
    /// The participants other than the sender.
    Direct { user_ids: Vec<UserId> },
}

/// A conversation that the bot takes part in. A conversation is a topic in a
/// channel or a direct-message conversation. The bot reads its history and
/// posts its answers there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Conversation {
    Topic {
        channel_id: ChannelId,
        channel_name: String,
        /// The topic spelling of the triggering message.
        topic: String,
    },
    /// The participants other than the bot in user ID order. The list is never
    /// empty.
    Direct { participants: Vec<Participant> },
}

impl Conversation {
    /// Returns the conversation of `message` with the bot left out of the
    /// participants. It returns `None` for a direct message that the bot sent
    /// to itself.
    pub fn of(message: &Message, bot: UserId) -> Option<Self> {
        match &message.recipient {
            MessageRecipient::Topic {
                channel_id,
                channel_name,
                topic,
            } => Some(Self::Topic {
                channel_id: *channel_id,
                channel_name: channel_name.clone(),
                topic: topic.clone(),
            }),
            MessageRecipient::Direct { participants } => {
                let mut participants = participants
                    .iter()
                    .filter(|participant| participant.id != bot)
                    .cloned()
                    .collect::<Vec<_>>();
                participants.sort_by_key(|participant| participant.id);
                participants.dedup_by_key(|participant| participant.id);
                (!participants.is_empty()).then_some(Self::Direct { participants })
            }
        }
    }

    /// Returns the recipient of a reply in the conversation.
    pub fn recipient(&self) -> Recipient {
        match self {
            Self::Topic {
                channel_id, topic, ..
            } => Recipient::Topic {
                channel_id: *channel_id,
                topic: topic.clone(),
            },
            Self::Direct { .. } => Recipient::Direct {
                user_ids: self.participant_ids().collect(),
            },
        }
    }

    /// Returns the channel of a topic.
    pub fn channel_id(&self) -> Option<ChannelId> {
        match self {
            Self::Topic { channel_id, .. } => Some(*channel_id),
            Self::Direct { .. } => None,
        }
    }

    /// Returns true for a direct-message conversation with one other person.
    /// Every message of such a conversation is addressed to the bot.
    pub fn is_one_to_one(&self) -> bool {
        matches!(self, Self::Direct { participants } if participants.len() == 1)
    }

    /// Returns the channel name for a topic and the participant names for a
    /// direct-message conversation.
    pub fn display_name(&self) -> String {
        match self {
            Self::Topic { channel_name, .. } => channel_name.clone(),
            Self::Direct { participants } => participants
                .iter()
                .map(|participant| participant.full_name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    pub fn participant_ids(&self) -> impl Iterator<Item = UserId> + '_ {
        let participants = match self {
            Self::Topic { .. } => [].as_slice(),
            Self::Direct { participants } => participants.as_slice(),
        };
        participants.iter().map(|participant| participant.id)
    }

    /// Returns the Zulip narrow that selects the messages of the conversation.
    pub(super) fn narrow(&self) -> Value {
        match self {
            Self::Topic {
                channel_id, topic, ..
            } => json!([
                {"operator": "channel", "operand": channel_id},
                {"operator": "topic", "operand": topic},
            ]),
            Self::Direct { .. } => json!([
                {"operator": "dm", "operand": self.participant_ids().collect::<Vec<_>>()},
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Conversation;
    use crate::{ChannelId, Message, MessageId, MessageRecipient, Participant, UserId};

    const BOT: UserId = UserId(99);

    fn participant(id: u64, full_name: &str) -> Participant {
        Participant {
            id: UserId(id),
            full_name: full_name.to_owned(),
        }
    }

    fn direct_message(participants: Vec<Participant>) -> Message {
        Message {
            id: MessageId(1),
            sender_id: UserId(7),
            sender_full_name: "Ada".to_owned(),
            content: "hi".to_owned(),
            timestamp: 0,
            recipient: MessageRecipient::Direct { participants },
            mentions_bot: false,
            reactions: Vec::new(),
        }
    }

    #[test]
    fn direct_conversation_leaves_out_the_bot_and_orders_participants() {
        let message = direct_message(vec![
            participant(9, "Grace"),
            participant(99, "Centaur"),
            participant(7, "Ada"),
        ]);
        let conversation = Conversation::of(&message, BOT).unwrap();
        assert_eq!(
            conversation,
            Conversation::Direct {
                participants: vec![participant(7, "Ada"), participant(9, "Grace")],
            }
        );
        assert!(!conversation.is_one_to_one());
        assert_eq!(conversation.display_name(), "Ada, Grace");
        assert_eq!(
            conversation.narrow(),
            json!([{"operator": "dm", "operand": [7, 9]}])
        );
    }

    #[test]
    fn direct_message_to_itself_has_no_conversation() {
        let message = direct_message(vec![participant(99, "Centaur")]);
        assert_eq!(Conversation::of(&message, BOT), None);
    }

    #[test]
    fn one_to_one_conversation_has_one_other_participant() {
        let message = direct_message(vec![participant(7, "Ada"), participant(99, "Centaur")]);
        assert!(Conversation::of(&message, BOT).unwrap().is_one_to_one());
    }

    #[test]
    fn topic_narrow_selects_the_channel_and_topic() {
        let conversation = Conversation::Topic {
            channel_id: ChannelId(42),
            channel_name: "engineering".to_owned(),
            topic: "Deploys".to_owned(),
        };
        assert!(!conversation.is_one_to_one());
        assert_eq!(conversation.display_name(), "engineering");
        assert_eq!(
            conversation.narrow(),
            json!([
                {"operator": "channel", "operand": 42},
                {"operator": "topic", "operand": "Deploys"},
            ])
        );
    }
}
