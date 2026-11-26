//! The rules that decide whether a message starts work.
//!
//! A message starts work when it is addressed to the bot and the deployment
//! allows its channel and its sender. Every list fails closed. An empty list
//! allows nothing.

use std::{collections::BTreeSet, fmt, str::FromStr};

use thiserror::Error;

use centaur_zulip::{ChannelId, Conversation, Message, User, UserId};

/// A set of allowed IDs. The text `*` parses to [`Allowlist::Everyone`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Allowlist<T> {
    Everyone,
    Only(BTreeSet<T>),
}

impl<T: Ord> Allowlist<T> {
    pub fn allows(&self, id: &T) -> bool {
        match self {
            Self::Everyone => true,
            Self::Only(ids) => ids.contains(id),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Only(ids) if ids.is_empty())
    }
}

impl<T> Default for Allowlist<T> {
    fn default() -> Self {
        Self::Only(BTreeSet::new())
    }
}

/// An allowlist entry that is not an ID.
#[derive(Debug, Error, Eq, PartialEq)]
#[error("invalid ID {id:?}: {reason}")]
pub struct InvalidId {
    pub id: String,
    pub reason: String,
}

/// Parses `*` or a list of IDs. Commas and whitespace separate the IDs.
impl<T> FromStr for Allowlist<T>
where
    T: FromStr + Ord,
    T::Err: fmt::Display,
{
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.trim() == "*" {
            return Ok(Self::Everyone);
        }
        value
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|id| !id.is_empty())
            .map(|id| {
                id.parse().map_err(|error: T::Err| InvalidId {
                    id: id.to_owned(),
                    reason: error.to_string(),
                })
            })
            .collect::<Result<_, _>>()
            .map(Self::Only)
    }
}

/// The reason that a message does not start work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ignored {
    /// The bot sent the message.
    OwnMessage,
    /// A channel message or a group direct message does not @-mention the
    /// bot.
    NotAddressed,
    ChannelNotAllowed,
    /// The direct-message allowlist does not include the sender.
    SenderNotAllowed,
    /// The sender is a bot that the trigger-bot allowlist does not include.
    BotNotAllowed,
    /// The sender is a guest that the guest allowlist does not include.
    GuestNotAllowed,
    /// The bot could not read the sender's account. The account type is
    /// unknown.
    SenderUnknown,
}

impl Ignored {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnMessage => "own_message",
            Self::NotAddressed => "not_addressed",
            Self::ChannelNotAllowed => "channel_not_allowed",
            Self::SenderNotAllowed => "sender_not_allowed",
            Self::BotNotAllowed => "bot_not_allowed",
            Self::GuestNotAllowed => "guest_not_allowed",
            Self::SenderUnknown => "sender_unknown",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Policy {
    /// The channels whose topics the bot answers in.
    pub channels: Allowlist<ChannelId>,
    /// The users that may send the bot direct messages.
    pub direct_message_senders: Allowlist<UserId>,
    /// The bot accounts whose messages may start work. The bot ignores all
    /// other bots. Two bots therefore cannot answer each other in a loop.
    pub trigger_bots: Allowlist<UserId>,
    /// The guest accounts whose messages may start work. The list applies in
    /// channels and in direct messages.
    pub trigger_guests: Allowlist<UserId>,
}

impl Policy {
    /// Checks the rules that depend only on the message. A message that
    /// passes must also pass [`Policy::check_sender`].
    pub fn check_message(
        &self,
        message: &Message,
        conversation: &Conversation,
        bot: UserId,
    ) -> Result<(), Ignored> {
        if message.sender_id == bot {
            return Err(Ignored::OwnMessage);
        }
        if !conversation.is_one_to_one() && !message.mentions_bot {
            return Err(Ignored::NotAddressed);
        }
        match conversation {
            Conversation::Topic { channel_id, .. } if !self.channels.allows(channel_id) => {
                Err(Ignored::ChannelNotAllowed)
            }
            Conversation::Direct { .. }
                if !self.direct_message_senders.allows(&message.sender_id) =>
            {
                Err(Ignored::SenderNotAllowed)
            }
            Conversation::Topic { .. } | Conversation::Direct { .. } => Ok(()),
        }
    }

    /// Checks the sender's account type. A message does not include the
    /// account type. The caller therefore reads it from Zulip.
    pub fn check_sender(&self, sender: &User) -> Result<(), Ignored> {
        if sender.is_bot && !self.trigger_bots.allows(&sender.user_id) {
            return Err(Ignored::BotNotAllowed);
        }
        if sender.is_guest && !self.trigger_guests.allows(&sender.user_id) {
            return Err(Ignored::GuestNotAllowed);
        }
        Ok(())
    }

    /// Returns true when no channel and no direct-message sender is allowed.
    pub fn is_inert(&self) -> bool {
        self.channels.is_empty() && self.direct_message_senders.is_empty()
    }
}

#[cfg(test)]
mod tests;
