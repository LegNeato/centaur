//! The part of the Zulip REST API that Centaur uses.
//!
//! The Zulip ingress depends only on [`ZulipApi`]. [`ZulipClient`]
//! implements it over HTTP. The tests of the ingress implement it in memory.
//! api-rs uses [`ZulipClient`] to post for the agent.

mod client;
mod conversation;
mod error;
mod model;
mod upload;

use std::future::Future;

pub use client::{ZulipClient, ZulipCredentials};
pub use conversation::{Conversation, Recipient};
pub use error::{ErrorCode, ZulipError};
pub use model::{
    ChannelId, Event, EventId, EventKind, Message, MessageId, MessageRecipient, Participant,
    Profile, ProfileFieldId, QueueId, Reaction, Registration, ServerSettings, Subscription, User,
    UserId,
};
pub use upload::{Download, UploadPath};

/// The oldest supported server feature level. Feature level 250 (Zulip 9.0)
/// adds the `channel` message type and narrow operator that the bot uses.
pub const MIN_FEATURE_LEVEL: u64 = 250;

/// The messages that a history request selects. A request returns the
/// newest selected messages, oldest first.
#[derive(Clone, Copy, Debug)]
pub enum History<'a> {
    /// The messages of `conversation` before `before`.
    Conversation {
        conversation: &'a Conversation,
        before: MessageId,
    },
    /// The messages that mention the bot, including wildcard mentions. A
    /// `before` selects only older messages.
    Mentions { before: Option<MessageId> },
    /// The direct messages that the bot sent or received. A `before` selects
    /// only older messages.
    DirectMessages { before: Option<MessageId> },
}

/// The Zulip calls of the bot account.
pub trait ZulipApi: Send + Sync + 'static {
    fn own_user(&self) -> impl Future<Output = Result<User, ZulipError>> + Send;

    fn user(&self, user_id: UserId) -> impl Future<Output = Result<User, ZulipError>> + Send;

    /// Registers an event queue for new messages.
    fn register_queue(&self) -> impl Future<Output = Result<Registration, ZulipError>> + Send;

    /// Waits for the events after `last_event_id`. The server holds the
    /// request open until an event arrives or a heartbeat is due.
    fn events(
        &self,
        queue_id: &QueueId,
        last_event_id: EventId,
    ) -> impl Future<Output = Result<Vec<Event>, ZulipError>> + Send;

    /// Returns up to `limit` messages of `history`, oldest first.
    fn messages(
        &self,
        history: History<'_>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Message>, ZulipError>> + Send;

    fn send_message(
        &self,
        conversation: &Conversation,
        content: &str,
    ) -> impl Future<Output = Result<MessageId, ZulipError>> + Send;

    /// Adds a reaction of the bot. The call succeeds when the reaction exists.
    fn add_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> impl Future<Output = Result<(), ZulipError>> + Send;

    /// Removes a reaction of the bot. The call succeeds when the reaction is
    /// missing.
    fn remove_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> impl Future<Output = Result<(), ZulipError>> + Send;

    /// Returns the subscription state of `user_id` in `channel_id`.
    fn subscription(
        &self,
        user_id: UserId,
        channel_id: ChannelId,
    ) -> impl Future<Output = Result<Subscription, ZulipError>> + Send;

    /// Subscribes the bot to the channel named `channel_name`. Zulip
    /// identifies the channel of a subscription request by name. The call
    /// succeeds when the bot is subscribed already.
    fn subscribe(&self, channel_name: &str) -> impl Future<Output = Result<(), ZulipError>> + Send;

    /// Returns the uploads that a message links to. The server resolves the
    /// links when it renders the message.
    fn linked_uploads(
        &self,
        message_id: MessageId,
    ) -> impl Future<Output = Result<Vec<UploadPath>, ZulipError>> + Send;

    /// Downloads `upload` with the bot's permissions. A file larger than
    /// `max_bytes` fails with [`ZulipError::UploadTooLarge`].
    fn download(
        &self,
        upload: &UploadPath,
        max_bytes: u64,
    ) -> impl Future<Output = Result<Download, ZulipError>> + Send;

    /// Starts or stops the bot's typing notification in `conversation`.
    fn set_typing(
        &self,
        conversation: &Conversation,
        typing: bool,
    ) -> impl Future<Output = Result<(), ZulipError>> + Send;
}
