//! The handling of one Zulip message from receipt to answer.
//!
//! The bot receives each message in the order of the event queue. A received
//! message takes a place in the queue of its conversation. A message that
//! starts work then goes through two phases.
//!
//! 1. The hand-off runs in the turn of the message. It marks the message
//!    with [`Status::Working`] and stores the new messages in the session.
//!    Then it steers the running execution or starts a new one. The next
//!    message of the conversation waits until the execution exists. The
//!    next message then steers the execution.
//! 2. The delivery runs after the turn passes on. It waits for the execution
//!    to end and keeps the typing notification alive in the meantime. Then
//!    it posts the answer and replaces [`Status::Working`] with a final
//!    status.
//!
//! Every session API call is idempotent. The bot can therefore repeat both
//! phases for a message after a restart.
//!
//! A stop command interrupts the running execution of the conversation. The
//! interrupted message gets [`Status::Stopped`] and no answer.
//!
//! The bot subscribes to each channel where an allowed user mentions it. A
//! subscription gives the bot the history, the files, and the reactions of a
//! private channel.

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use centaur_session_core::ThreadKey;
use tracing::{debug, error, info, warn};

use crate::{
    command,
    footer::{HarnessDefaults, MetadataMode},
    policy::{Ignored, Policy},
    retry::Backoff,
    selection::Choices,
    sessions::Sessions,
    split::split_message,
    status::Status,
    steered::SteeredMessages,
    sync::{Claim, Claims, ExpiringMap, Lanes, Place},
    turn::{self, ExecutionLimits},
};
use centaur_zulip::{
    ChannelId, Conversation, ErrorCode, Message, MessageId, Participant, ServerSettings,
    Subscription, User, UserId, ZulipApi, ZulipError,
};

/// The number of recent message IDs that the bot records. The bot records
/// only the messages that the policy admits. A message whose ID is in the
/// record is not handled again.
const SEEN_MESSAGES: usize = 4096;
/// The number of ended executions whose final status the bot keeps for a
/// late steering message.
const ENDED_EXECUTIONS: usize = 256;
/// The lifetime of a sender lookup. A member can become a guest. The bot
/// therefore reads the role again after this time.
const SENDER_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct Settings {
    /// The harness, model, and persona choices of the deployment.
    pub choices: Choices,
    pub metadata: MetadataMode,
    pub harness_defaults: HarnessDefaults,
    /// The maximum number of earlier messages that one turn reads for context.
    pub context_messages: u32,
    pub limits: ExecutionLimits,
    /// The age limit of the messages that the recovery scan reads. The scan
    /// resumes every running answer only when the window is at least as long
    /// as the longest execution.
    pub recovery_window: Duration,
    /// The maximum total size of the files that one turn attaches. api-rs
    /// stores the bytes in the session messages and in the execution
    /// request.
    pub file_bytes: u64,
}

pub struct Bot<Z, S> {
    zulip: Z,
    sessions: S,
    identity: User,
    policy: Policy,
    settings: Settings,
    /// The settings of the last queue registration.
    server: RwLock<Arc<ServerSettings>>,
    /// The senders that the bot looked up recently.
    senders: ExpiringMap<UserId, User>,
    /// The order of the messages of each conversation.
    conversations: Lanes<ThreadKey>,
    /// The messages in progress and the recently received ones.
    claims: Claims<MessageId>,
    /// The messages that steered each execution. Each message gets the final
    /// status of the answer of its execution.
    steered: SteeredMessages,
}

/// Where the bot found a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    EventQueue,
    /// The recovery scan. A notice that the bot sent for the message before a
    /// restart is not sent again.
    Scan,
}

/// The result of the hand-off phase.
#[derive(Debug, Eq, PartialEq)]
enum HandOff {
    /// The message started the execution. A message handled again after a
    /// restart rejoins its execution. The footer is the metadata line of the
    /// answer.
    Execution {
        execution_id: String,
        footer: Option<String>,
    },
    /// The message names a model that the catalog cannot resolve. The bot
    /// rejects it instead of running it on another model.
    Rejected,
    /// The message steered a running execution.
    Steered,
    /// The deployment admits only preapproved conversations. An operator has
    /// not approved the conversation of the message.
    NotAdmitted,
    Failed,
}

impl<Z: ZulipApi, S: Sessions> Bot<Z, S> {
    pub fn new(zulip: Z, sessions: S, identity: User, policy: Policy, settings: Settings) -> Self {
        Self {
            zulip,
            sessions,
            identity,
            policy,
            settings,
            server: RwLock::default(),
            senders: ExpiringMap::new(SENDER_CACHE_TTL),
            conversations: Lanes::default(),
            claims: Claims::new(SEEN_MESSAGES),
            steered: SteeredMessages::new(ENDED_EXECUTIONS),
        }
    }

    pub fn zulip(&self) -> &Z {
        &self.zulip
    }

    pub fn set_server_settings(&self, server: ServerSettings) {
        *self
            .server
            .write()
            .expect("server settings lock is not poisoned") = Arc::new(server);
    }

    fn server_settings(&self) -> Arc<ServerSettings> {
        self.server
            .read()
            .expect("server settings lock is not poisoned")
            .clone()
    }

    /// Receives a message from the event queue. It returns `None` for a
    /// message that the bot does not handle. The caller passes the result to
    /// [`Bot::process`]. The order of the `receive` calls is the order in
    /// which the bot handles the messages of a conversation.
    pub fn receive(&self, message: Message) -> Option<Received> {
        self.receive_from(message, Source::EventQueue)
    }

    /// Receives a message from the recovery scan.
    pub fn receive_recovered(&self, message: Message) -> Option<Received> {
        self.receive_from(message, Source::Scan)
    }

    /// Receives and processes a message from the event queue.
    #[cfg(test)]
    pub async fn handle(&self, message: Message) {
        if let Some(received) = self.receive(message) {
            self.process(received).await;
        }
    }

    /// Receives and processes a message from the recovery scan.
    #[cfg(test)]
    pub async fn handle_recovered(&self, message: Message) {
        if let Some(received) = self.receive_recovered(message) {
            self.process(received).await;
        }
    }

    /// Applies the checks that need no request. Then it claims the message
    /// and joins the queue of its conversation. A message that is in
    /// progress or was received recently is not received again. A message
    /// from both the queue and the recovery scan is therefore handled once.
    fn receive_from(&self, message: Message, source: Source) -> Option<Received> {
        let conversation = Conversation::of(&message, self.identity.user_id)?;
        if let Err(ignored) =
            self.policy
                .check_message(&message, &conversation, self.identity.user_id)
        {
            debug!(
                event = "zulipbot_message_ignored",
                message_id = %message.id,
                reason = ignored.as_str(),
            );
            return None;
        }
        let Some(thread_key) = turn::thread_key(&conversation) else {
            warn!(event = "zulipbot_conversation_unkeyable", message_id = %message.id);
            return None;
        };
        let Some(claim) = self.claims.claim(message.id) else {
            debug!(event = "zulipbot_message_duplicate", message_id = %message.id);
            return None;
        };
        let place = self.conversations.join(thread_key.clone());
        Some(Received {
            message,
            source,
            conversation,
            thread_key,
            claim,
            place,
        })
    }

    /// Handles a received message. The hand-off waits for the turn of the
    /// message in its conversation. The delivery runs after the turn passes
    /// on.
    pub async fn process(&self, received: Received) {
        let Received {
            message,
            source,
            conversation,
            thread_key,
            claim: _claim,
            place,
        } = received;
        place.turn().await;
        let sender = match self.admit_sender(&message).await {
            Ok(sender) => sender,
            Err(ignored) => {
                debug!(
                    event = "zulipbot_message_ignored",
                    message_id = %message.id,
                    reason = ignored.as_str(),
                );
                return;
            }
        };
        if let Conversation::Topic {
            channel_id,
            channel_name,
            ..
        } = &conversation
            && !self.join(&message, *channel_id, channel_name, source).await
        {
            return;
        }
        if command::is_stop(&message.content) {
            // A stop is for the run of that moment. The recovery scan does
            // not replay it.
            if source == Source::EventQueue {
                self.stop(&message, &thread_key).await;
            }
            return;
        }
        info!(
            event = "zulipbot_message_accepted",
            message_id = %message.id,
            thread_key = %thread_key,
        );
        let handoff = self
            .hand_off(&message, &sender, &conversation, &thread_key)
            .await;
        drop(place);
        match handoff {
            HandOff::Execution {
                execution_id,
                footer,
            } => {
                let delivery = delivery::Delivery {
                    message: &message,
                    conversation: &conversation,
                    thread_key: &thread_key,
                    execution_id: &execution_id,
                    footer: footer.as_deref(),
                };
                self.deliver(delivery).await;
            }
            HandOff::Steered | HandOff::Rejected | HandOff::NotAdmitted | HandOff::Failed => {}
        }
    }

    /// Applies the sender rules of the policy. It returns the sender of an
    /// admitted message.
    async fn admit_sender(&self, message: &Message) -> Result<User, Ignored> {
        let sender = self.sender(message.sender_id).await.map_err(|error| {
            // The policy fails closed. A sender of unknown type can be a bot
            // or a guest.
            warn!(
                event = "zulipbot_sender_lookup_failed",
                message_id = %message.id,
                sender_id = %message.sender_id,
                %error,
            );
            Ignored::SenderUnknown
        })?;
        self.policy.check_sender(&sender)?;
        Ok(sender)
    }

    async fn sender(&self, user_id: UserId) -> Result<User, ZulipError> {
        if let Some(user) = self.senders.get(&user_id) {
            return Ok(user);
        }
        let user = Backoff::REQUEST
            .retry("get_user", || self.zulip.user(user_id))
            .await?;
        self.senders.insert(user_id, user.clone());
        Ok(user)
    }

    /// Subscribes the bot to the channel of `message`. It returns false when
    /// the bot cannot read the channel. The bot then tells the sender in a
    /// direct message. A failed check or a failed subscription for another
    /// reason returns true. The turn then runs with what the bot can read.
    async fn join(
        &self,
        message: &Message,
        channel_id: ChannelId,
        channel_name: &str,
        source: Source,
    ) -> bool {
        let subscription = Backoff::REQUEST
            .retry("get_subscription", || {
                self.zulip.subscription(self.identity.user_id, channel_id)
            })
            .await;
        let readable = match subscription {
            Ok(Subscription::Subscribed) => return true,
            Ok(Subscription::Unsubscribed) => match Backoff::REQUEST
                .retry("subscribe", || self.zulip.subscribe(channel_name))
                .await
            {
                Ok(()) => {
                    info!(event = "zulipbot_channel_joined", channel_id = %channel_id);
                    true
                }
                // Zulip refuses a private channel without a grant.
                Err(error) if error.code() == Some(ErrorCode::BadRequest) => false,
                Err(error) => {
                    warn!(event = "zulipbot_subscribe_failed", channel_id = %channel_id, %error);
                    true
                }
            },
            Ok(Subscription::Hidden) => false,
            Err(error) => {
                warn!(event = "zulipbot_subscription_unknown", channel_id = %channel_id, %error);
                true
            }
        };
        if !readable {
            info!(
                event = "zulipbot_channel_unreadable",
                message_id = %message.id,
                channel_id = %channel_id,
            );
            if source == Source::EventQueue {
                let sender = Conversation::Direct {
                    participants: vec![Participant {
                        id: message.sender_id,
                        full_name: message.sender_full_name.clone(),
                    }],
                };
                self.post(&sender, &unreadable_channel_notice(channel_name))
                    .await;
            }
        }
        readable
    }

    /// Interrupts the running execution of the conversation. The stop runs
    /// in its turn. A stop after a message whose hand-off is in progress
    /// therefore reaches the execution that the hand-off starts. A stop
    /// without a running execution changes nothing and posts nothing.
    async fn stop(&self, message: &Message, thread_key: &ThreadKey) {
        let reason = format!("Stopped from Zulip by {}", message.sender_full_name);
        match Backoff::REQUEST
            .retry("interrupt_execution", || {
                self.sessions.interrupt(thread_key, &reason)
            })
            .await
        {
            Ok(true) => {
                info!(
                    event = "zulipbot_execution_stopped",
                    message_id = %message.id,
                    thread_key = %thread_key,
                );
                self.add_reaction(message.id, Status::Answered).await;
            }
            Ok(false) => {
                debug!(
                    event = "zulipbot_stop_without_execution",
                    message_id = %message.id,
                    thread_key = %thread_key,
                );
            }
            Err(error) => {
                error!(
                    event = "zulipbot_stop_failed",
                    message_id = %message.id,
                    thread_key = %thread_key,
                    %error,
                );
                self.add_reaction(message.id, Status::Failed).await;
            }
        }
    }

    /// Sets the typing notification. A typing notification is only a hint.
    /// A failure is therefore only logged.
    async fn set_typing(&self, conversation: &Conversation, typing: bool) {
        if let Err(error) = self.zulip.set_typing(conversation, typing).await {
            debug!(event = "zulipbot_typing_failed", %error);
        }
    }

    /// Posts `text` in parts that fit the server's length limit. It returns
    /// true when every part was posted. A send repeats only when Zulip surely
    /// did not post the part. A repeat after a timeout could post it twice.
    async fn post(&self, conversation: &Conversation, text: &str) -> bool {
        let limit = self.server_settings().max_message_length;
        for part in split_message(text, limit) {
            if let Err(error) = Backoff::REQUEST
                .retry_when("send_message", ZulipError::is_unsent, || {
                    self.zulip.send_message(conversation, &part)
                })
                .await
            {
                error!(event = "zulipbot_send_failed", %error);
                return false;
            }
        }
        true
    }

    /// Marks `message_id` with a final status. The bot adds the final reaction
    /// before it removes [`Status::Working`]. A restart between the two
    /// requests still finds the final status.
    async fn finish(&self, message_id: MessageId, status: Status) {
        self.replace_reaction(message_id, Status::Working, status)
            .await;
    }

    async fn replace_reaction(&self, message_id: MessageId, old: Status, new: Status) {
        self.add_reaction(message_id, new).await;
        if old != new {
            self.remove_reaction(message_id, old).await;
        }
    }

    async fn remove_reaction(&self, message_id: MessageId, status: Status) {
        if let Err(error) = Backoff::REQUEST
            .retry("remove_reaction", || {
                self.zulip.remove_reaction(message_id, status.emoji_name())
            })
            .await
        {
            warn!(event = "zulipbot_reaction_failed", message_id = %message_id, %error);
        }
    }

    /// Adds a status reaction. A failure removes a signal for the user and a
    /// hint for the recovery scan. The bot logs the failure and continues.
    async fn add_reaction(&self, message_id: MessageId, status: Status) {
        if let Err(error) = Backoff::REQUEST
            .retry("add_reaction", || {
                self.zulip.add_reaction(message_id, status.emoji_name())
            })
            .await
        {
            warn!(event = "zulipbot_reaction_failed", message_id = %message_id, %error);
        }
    }
}

/// Returns the direct message to a sender who mentions the bot in a private
/// channel that the bot cannot read.
fn unreadable_channel_notice(channel_name: &str) -> String {
    format!(
        "I cannot read the private channel #**{channel_name}**. Please subscribe me to the \
         channel. Then mention me again."
    )
}

/// A message that the bot received and has not processed yet. It holds the
/// claim on the message and its place in the queue of its conversation.
pub struct Received {
    message: Message,
    source: Source,
    conversation: Conversation,
    thread_key: ThreadKey,
    claim: Claim<MessageId>,
    place: Place<ThreadKey>,
}

mod delivery;
mod hand_off;
mod scan;

#[cfg(test)]
mod tests;
