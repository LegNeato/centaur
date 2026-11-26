//! The Zulip API objects that Centaur reads.
//!
//! The types keep only the fields that Centaur uses. Serde ignores all other
//! fields. A newer server that adds fields therefore does not break parsing.

mod event;
mod ids;
mod message;
mod registration;
mod user;

pub use event::{Event, EventKind};
pub use ids::{ChannelId, EventId, MessageId, ProfileFieldId, QueueId, UserId};
pub use message::{Message, MessageRecipient, Participant, Reaction};
pub use registration::{Registration, ServerSettings};
pub use user::{Profile, User};

/// The subscription state of a user in one channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Subscription {
    Subscribed,
    /// The user can see the channel but is not subscribed. A subscription
    /// can still fail for a private channel.
    Unsubscribed,
    /// The user cannot see the channel. The channel is private or does not
    /// exist.
    Hidden,
}

#[cfg(test)]
mod tests;
