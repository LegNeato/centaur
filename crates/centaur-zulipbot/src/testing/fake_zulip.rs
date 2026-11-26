use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Duration,
};

use reqwest::StatusCode;

use super::{BOT, BOT_NAME, SENT_AT};
use centaur_zulip::{
    ChannelId, Conversation, Download, ErrorCode, Event, EventId, History, Message, MessageId,
    MessageRecipient, Participant, Profile, QueueId, Reaction, Registration, ServerSettings,
    Subscription, UploadPath, User, UserId, ZulipApi, ZulipError,
};

/// An in-memory Zulip server with one bot account. Sent messages and
/// reactions change the stored messages. Later history reads therefore see
/// the changes.
#[derive(Default)]
pub struct FakeZulip {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    bots: Vec<UserId>,
    guests: Vec<UserId>,
    profiles: HashMap<UserId, Profile>,
    unknown_users: Vec<UserId>,
    messages: Vec<Message>,
    sent: Vec<(Conversation, String)>,
    typing: Vec<bool>,
    registrations: VecDeque<Result<Registration, ZulipError>>,
    events: VecDeque<Result<Vec<Event>, ZulipError>>,
    queue_registrations: usize,
    /// The errors of the next history reads, in order.
    history_errors: VecDeque<ZulipError>,
    /// The errors of the next sends, in order.
    send_errors: VecDeque<ZulipError>,
    send_attempts: usize,
    /// The channels with a scripted subscription. The bot is subscribed to
    /// every other channel.
    channels: HashMap<ChannelId, FakeChannel>,
    subscribe_requests: Vec<String>,
    /// The uploads that each message links to.
    links: HashMap<MessageId, Vec<UploadPath>>,
    /// The messages whose rendering fails.
    unrenderable: Vec<MessageId>,
    /// The file or the error status of each upload.
    files: HashMap<UploadPath, Result<Download, StatusCode>>,
    downloads: Vec<UploadPath>,
}

struct FakeChannel {
    name: String,
    subscription: Subscription,
    /// True when a subscription request succeeds.
    joinable: bool,
}

pub const SERVER: ServerSettings = ServerSettings {
    feature_level: 400,
    max_message_length: 10_000,
    typing_refresh_interval: Duration::from_secs(10),
    github_fields: Vec::new(),
};

/// A Zulip API error with the given code.
pub fn api_error(code: ErrorCode) -> ZulipError {
    ZulipError::Api {
        status: StatusCode::BAD_REQUEST,
        code,
        message: format!("{code:?}"),
        retry_after: None,
    }
}

pub fn registration(queue_id: &str, last_event_id: i64) -> Registration {
    Registration {
        queue_id: QueueId(queue_id.to_owned()),
        last_event_id: EventId(last_event_id),
        server: SERVER,
    }
}

impl FakeZulip {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Stores messages the bot can read in history and scans.
    pub fn with_messages(self, messages: impl IntoIterator<Item = Message>) -> Self {
        self.state().messages.extend(messages);
        self
    }

    pub fn with_bot_account(self, user_id: UserId) -> Self {
        self.state().bots.push(user_id);
        self
    }

    pub fn with_guest_account(self, user_id: UserId) -> Self {
        self.state().guests.push(user_id);
        self
    }

    pub fn with_profile(self, user_id: UserId, profile: Profile) -> Self {
        self.state().profiles.insert(user_id, profile);
        self
    }

    /// Makes lookups of `user_id` fail with a permanent error.
    pub fn with_unknown_user(self, user_id: UserId) -> Self {
        self.state().unknown_users.push(user_id);
        self
    }

    /// Answers the next queue registrations with `results`.
    pub fn with_registrations(
        self,
        results: impl IntoIterator<Item = Result<Registration, ZulipError>>,
    ) -> Self {
        self.state().registrations.extend(results);
        self
    }

    /// Answers the next event polls with `results`. A poll after the last
    /// result waits until the test ends.
    pub fn with_events(
        self,
        results: impl IntoIterator<Item = Result<Vec<Event>, ZulipError>>,
    ) -> Self {
        self.state().events.extend(results);
        self
    }

    /// Makes the bot unsubscribed from `channel_id`. A subscription request
    /// for `name` succeeds when `joinable` is true.
    pub fn with_unsubscribed_channel(
        self,
        channel_id: ChannelId,
        name: &str,
        joinable: bool,
    ) -> Self {
        self.with_channel(channel_id, name, Subscription::Unsubscribed, joinable)
    }

    /// Makes `channel_id` a private channel that the bot cannot see.
    pub fn with_hidden_channel(self, channel_id: ChannelId, name: &str) -> Self {
        self.with_channel(channel_id, name, Subscription::Hidden, false)
    }

    fn with_channel(
        self,
        channel_id: ChannelId,
        name: &str,
        subscription: Subscription,
        joinable: bool,
    ) -> Self {
        self.state().channels.insert(
            channel_id,
            FakeChannel {
                name: name.to_owned(),
                subscription,
                joinable,
            },
        );
        self
    }

    /// Makes `message_id` link to the upload `link` with `bytes`.
    pub fn with_file(
        self,
        message_id: MessageId,
        link: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Self {
        let download = Download {
            media_type: Some(media_type.to_owned()),
            bytes: bytes.to_vec(),
        };
        self.with_upload(message_id, link, Ok(download))
    }

    /// Makes `message_id` link to the upload `link`, which fails with `status`.
    pub fn with_unavailable_file(
        self,
        message_id: MessageId,
        link: &str,
        status: StatusCode,
    ) -> Self {
        self.with_upload(message_id, link, Err(status))
    }

    fn with_upload(
        self,
        message_id: MessageId,
        link: &str,
        file: Result<Download, StatusCode>,
    ) -> Self {
        let upload = UploadPath::parse(link).expect("test upload links are valid");
        {
            let mut state = self.state();
            state
                .links
                .entry(message_id)
                .or_default()
                .push(upload.clone());
            state.files.insert(upload, file);
        }
        self
    }

    /// Makes the rendering of `message_id` fail with a permanent error.
    pub fn with_unrenderable_message(self, message_id: MessageId) -> Self {
        self.state().unrenderable.push(message_id);
        self
    }

    /// The channel names of the subscription requests in order.
    pub fn subscribe_requests(&self) -> Vec<String> {
        self.state().subscribe_requests.clone()
    }

    /// The downloaded uploads in order.
    pub fn downloads(&self) -> Vec<String> {
        self.state()
            .downloads
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Makes the next `count` sends fail with a permanent error.
    pub fn failing_sends(self, count: usize) -> Self {
        self.with_send_errors((0..count).map(|_| api_error(ErrorCode::Other)))
    }

    /// Makes the next history reads fail with `errors` in order.
    pub fn with_history_errors(self, errors: impl IntoIterator<Item = ZulipError>) -> Self {
        self.state().history_errors.extend(errors);
        self
    }

    /// Makes the next sends fail with `errors` in order.
    pub fn with_send_errors(self, errors: impl IntoIterator<Item = ZulipError>) -> Self {
        self.state().send_errors.extend(errors);
        self
    }

    /// How many sends the bot attempted, including the failed ones.
    pub fn send_attempts(&self) -> usize {
        self.state().send_attempts
    }

    /// The text of every message the bot sent, in order.
    pub fn sent(&self) -> Vec<String> {
        self.state()
            .sent
            .iter()
            .map(|(_, text)| text.clone())
            .collect()
    }

    pub fn sent_to(&self) -> Vec<Conversation> {
        self.state()
            .sent
            .iter()
            .map(|(conversation, _)| conversation.clone())
            .collect()
    }

    /// The bot's reactions on `message_id`.
    pub fn reactions(&self, message_id: MessageId) -> Vec<String> {
        self.state()
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .map(|message| {
                message
                    .reactions
                    .iter()
                    .filter(|reaction| reaction.user_id == BOT)
                    .map(|reaction| reaction.emoji_name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every typing notification in order. True means start. False means stop.
    pub fn typing(&self) -> Vec<bool> {
        self.state().typing.clone()
    }

    pub fn queue_registrations(&self) -> usize {
        self.state().queue_registrations
    }
}

fn in_conversation(message: &Message, conversation: &Conversation) -> bool {
    match (Conversation::of(message, BOT), conversation) {
        (
            Some(Conversation::Topic {
                channel_id, topic, ..
            }),
            Conversation::Topic {
                channel_id: wanted_channel,
                topic: wanted_topic,
                ..
            },
        ) => channel_id == *wanted_channel && topic.to_lowercase() == wanted_topic.to_lowercase(),
        (Some(found @ Conversation::Direct { .. }), Conversation::Direct { .. }) => {
            found.participant_ids().eq(conversation.participant_ids())
        }
        _ => false,
    }
}

impl ZulipApi for FakeZulip {
    async fn own_user(&self) -> Result<User, ZulipError> {
        Ok(User {
            user_id: BOT,
            full_name: BOT_NAME.to_owned(),
            is_bot: true,
            is_guest: false,
            profile: Default::default(),
        })
    }

    async fn user(&self, user_id: UserId) -> Result<User, ZulipError> {
        let state = self.state();
        if state.unknown_users.contains(&user_id) {
            return Err(api_error(ErrorCode::Other));
        }
        Ok(User {
            user_id,
            full_name: super::name_of(user_id).to_owned(),
            is_bot: user_id == BOT || state.bots.contains(&user_id),
            is_guest: state.guests.contains(&user_id),
            profile: state.profiles.get(&user_id).cloned().unwrap_or_default(),
        })
    }

    async fn register_queue(&self) -> Result<Registration, ZulipError> {
        let mut state = self.state();
        state.queue_registrations += 1;
        state
            .registrations
            .pop_front()
            .unwrap_or_else(|| Ok(registration("queue", -1)))
    }

    async fn events(
        &self,
        _queue_id: &QueueId,
        _last_event_id: EventId,
    ) -> Result<Vec<Event>, ZulipError> {
        let next = self.state().events.pop_front();
        match next {
            Some(result) => result,
            None => std::future::pending().await,
        }
    }

    async fn messages(&self, history: History<'_>, limit: u32) -> Result<Vec<Message>, ZulipError> {
        let mut state = self.state();
        if let Some(error) = state.history_errors.pop_front() {
            return Err(error);
        }
        let mut found = state
            .messages
            .iter()
            .filter(|message| match history {
                History::Conversation {
                    conversation,
                    before,
                } => message.id < before && in_conversation(message, conversation),
                History::Mentions { before } => {
                    message.mentions_bot && before.is_none_or(|before| message.id < before)
                }
                History::DirectMessages { before } => {
                    matches!(message.recipient, MessageRecipient::Direct { .. })
                        && before.is_none_or(|before| message.id < before)
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        found.sort_by_key(|message| message.id);
        let skip = found.len().saturating_sub(limit as usize);
        Ok(found.split_off(skip))
    }

    async fn send_message(
        &self,
        conversation: &Conversation,
        content: &str,
    ) -> Result<MessageId, ZulipError> {
        let mut state = self.state();
        state.send_attempts += 1;
        if let Some(error) = state.send_errors.pop_front() {
            return Err(error);
        }
        let id = MessageId(
            state
                .messages
                .iter()
                .map(|message| message.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let recipient = match conversation {
            Conversation::Topic {
                channel_id,
                channel_name,
                topic,
            } => MessageRecipient::Topic {
                channel_id: *channel_id,
                channel_name: channel_name.clone(),
                topic: topic.clone(),
            },
            Conversation::Direct { participants } => MessageRecipient::Direct {
                participants: participants
                    .iter()
                    .cloned()
                    .chain([Participant {
                        id: BOT,
                        full_name: BOT_NAME.to_owned(),
                    }])
                    .collect(),
            },
        };
        state.messages.push(Message {
            id,
            sender_id: BOT,
            sender_full_name: BOT_NAME.to_owned(),
            content: content.to_owned(),
            timestamp: SENT_AT,
            recipient,
            mentions_bot: false,
            reactions: Vec::new(),
        });
        state.sent.push((conversation.clone(), content.to_owned()));
        Ok(id)
    }

    async fn add_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> Result<(), ZulipError> {
        let mut state = self.state();
        let message = state
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .ok_or_else(|| api_error(ErrorCode::Other))?;
        if !message.has_reaction(BOT, emoji_name) {
            message.reactions.push(Reaction {
                emoji_name: emoji_name.to_owned(),
                user_id: BOT,
            });
        }
        Ok(())
    }

    async fn remove_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> Result<(), ZulipError> {
        let mut state = self.state();
        if let Some(message) = state
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            message
                .reactions
                .retain(|reaction| !(reaction.user_id == BOT && reaction.emoji_name == emoji_name));
        }
        Ok(())
    }

    async fn subscription(
        &self,
        _user_id: UserId,
        channel_id: ChannelId,
    ) -> Result<Subscription, ZulipError> {
        Ok(self
            .state()
            .channels
            .get(&channel_id)
            .map_or(Subscription::Subscribed, |channel| channel.subscription))
    }

    async fn subscribe(&self, channel_name: &str) -> Result<(), ZulipError> {
        let mut state = self.state();
        state.subscribe_requests.push(channel_name.to_owned());
        let Some(channel) = state
            .channels
            .values_mut()
            .find(|channel| channel.name == channel_name)
        else {
            return Ok(());
        };
        if !channel.joinable {
            return Err(api_error(ErrorCode::BadRequest));
        }
        channel.subscription = Subscription::Subscribed;
        Ok(())
    }

    async fn linked_uploads(&self, message_id: MessageId) -> Result<Vec<UploadPath>, ZulipError> {
        let state = self.state();
        if state.unrenderable.contains(&message_id) {
            return Err(api_error(ErrorCode::Other));
        }
        Ok(state.links.get(&message_id).cloned().unwrap_or_default())
    }

    async fn download(&self, upload: &UploadPath, max_bytes: u64) -> Result<Download, ZulipError> {
        let mut state = self.state();
        state.downloads.push(upload.clone());
        match state.files.get(upload).cloned() {
            Some(Ok(download)) if download.bytes.len() as u64 > max_bytes => {
                Err(ZulipError::UploadTooLarge { limit: max_bytes })
            }
            Some(Ok(download)) => Ok(download),
            // Zulip answers a failed download with an HTML page.
            Some(Err(status)) => Err(ZulipError::UnexpectedStatus { status }),
            None => Err(ZulipError::UnexpectedStatus {
                status: StatusCode::NOT_FOUND,
            }),
        }
    }

    async fn set_typing(
        &self,
        _conversation: &Conversation,
        typing: bool,
    ) -> Result<(), ZulipError> {
        self.state().typing.push(typing);
        Ok(())
    }
}
