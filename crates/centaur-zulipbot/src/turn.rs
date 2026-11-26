//! The session API requests for one message that starts work.

use std::fmt::Write as _;

use centaur_session_core::{
    HarnessType, MessageRole, SessionMessageInput, ThreadKey,
    api::{AppendMessagesRequest, CreateSessionRequest, ExecuteSessionRequest, OnHarnessConflict},
    destination::{ZulipDestination, ZulipDirect, ZulipTopic},
};
use serde_json::{Value, json};

use crate::{command, files::Files, requester::Requester, selection::Selection, status::Status};
use centaur_zulip::{Conversation, Message, UserId};

/// The `source` value in session and message metadata.
const SOURCE: &str = "zulipbot";
const PLATFORM: &str = "zulip";

/// Returns the thread key of `conversation`. It returns `None` when the
/// conversation does not fit in a thread key. Zulip's own limits on topics
/// and participants prevent that case.
pub fn thread_key(conversation: &Conversation) -> Option<ThreadKey> {
    let destination = match conversation {
        Conversation::Topic {
            channel_id, topic, ..
        } => ZulipDestination::Topic(ZulipTopic::new(channel_id.0, topic)),
        Conversation::Direct { .. } => ZulipDestination::Direct(ZulipDirect::new(
            conversation.participant_ids().map(|user_id| user_id.0),
        )?),
    };
    destination.thread_key().ok()
}

/// The execution limits that the bot sends to api-rs. A `None` field keeps
/// the api-rs default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionLimits {
    pub idle_timeout_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
}

/// Returns the request that creates or reuses the session of
/// `conversation`. `restart` lets api-rs restart a session that runs another
/// harness. The persona applies only to a new session.
pub fn create_request(
    conversation: &Conversation,
    harness_type: HarnessType,
    persona_id: Option<String>,
    restart: bool,
) -> CreateSessionRequest {
    CreateSessionRequest {
        harness_type,
        persona_id,
        metadata: Some(json!({
            "source": SOURCE,
            "platform": PLATFORM,
            // iron-control uses this name for the session principal.
            "zulip_conversation_name": conversation.display_name(),
        })),
        on_harness_conflict: if restart {
            OnHarnessConflict::Restart
        } else {
            OnHarnessConflict::Reject
        },
        principal: None,
    }
}

/// Where the context of a turn starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextStart {
    /// After the last message that the bot handled. The session holds the
    /// earlier messages.
    AfterHandled,
    /// At the oldest fetched message. The bot handled none of the fetched
    /// messages.
    Oldest,
    /// At the oldest fetched message, with the bot's own answers. A harness
    /// restart left the session without history.
    Restart,
}

/// The message that starts work, with its choices and its sender.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub message: &'a Message,
    /// The text of the message without its flags.
    pub text: &'a str,
    pub selection: &'a Selection,
    pub requester: &'a Requester,
}

/// A message that starts work, together with its conversation and the
/// messages that the session does not hold yet.
#[derive(Debug)]
pub struct Turn<'a> {
    pub thread_key: &'a ThreadKey,
    pub conversation: &'a Conversation,
    pub request: Request<'a>,
    bot: UserId,
    /// The earlier messages that the turn lists, oldest first. Only a turn
    /// after a restart lists the bot's own messages.
    context: Vec<Message>,
    start: ContextStart,
}

impl<'a> Turn<'a> {
    /// Builds a turn from `history`. `history` holds the messages before the
    /// request in oldest-first order. The session holds every message up
    /// to the last request that the bot stored. The turn therefore keeps
    /// only the later messages. After a harness restart the turn keeps all of
    /// `history`. A stop command is not part of the conversation. The turn
    /// leaves it out.
    pub fn new(
        thread_key: &'a ThreadKey,
        conversation: &'a Conversation,
        request: Request<'a>,
        history: Vec<Message>,
        bot: UserId,
        restarted: bool,
    ) -> Self {
        let history = history
            .into_iter()
            .filter(|message| !command::is_stop(&message.content));
        let (context, start) = if restarted {
            (history.collect(), ContextStart::Restart)
        } else {
            let history = history.collect::<Vec<_>>();
            let last_handled = history
                .iter()
                .rposition(|message| Status::of(message, bot).is_some_and(Status::marks_stored));
            let context = history
                .into_iter()
                .skip(last_handled.map_or(0, |index| index + 1))
                .filter(|message| message.sender_id != bot)
                .collect();
            let start = match last_handled {
                Some(_) => ContextStart::AfterHandled,
                None => ContextStart::Oldest,
            };
            (context, start)
        };
        Self {
            thread_key,
            conversation,
            request,
            bot,
            context,
            start,
        }
    }

    /// Returns true when the bot answered none of the fetched messages of the
    /// conversation.
    pub fn is_first(&self) -> bool {
        self.start == ContextStart::Oldest
    }

    /// Returns the messages in the order of their download priority. The
    /// trigger comes first. The context follows from the newest message to
    /// the oldest. A download budget therefore favors the newest files.
    pub fn by_file_priority(&self) -> impl Iterator<Item = &Message> {
        [self.request.message]
            .into_iter()
            .chain(self.others().rev())
    }

    /// Returns the request that stores the context and the trigger. Each
    /// message has its Zulip message ID as the client message ID. A repeated
    /// append therefore stores nothing new. Each message holds its sender,
    /// its text, and its files. api-rs steers a running execution with the
    /// stored parts.
    pub fn append_request(&self, files: &Files) -> AppendMessagesRequest {
        AppendMessagesRequest {
            messages: self
                .messages()
                .map(|message| {
                    let mut parts =
                        vec![json!({"type": "text", "text": self.stored_text(message)})];
                    parts.extend(files.parts([message]));
                    SessionMessageInput {
                        client_message_id: Some(message.id.to_string()),
                        role: MessageRole::User,
                        parts,
                        metadata: message_metadata(message),
                    }
                })
                .collect(),
        }
    }

    /// Returns the request that starts the execution. The trigger's message
    /// ID is the idempotency key. A repeated request therefore returns the
    /// same execution.
    pub fn execute_request(&self, limits: ExecutionLimits, files: &Files) -> ExecuteSessionRequest {
        let mut metadata = message_metadata(self.request.message);
        metadata["action"] = json!("execute");
        let mut content = vec![
            json!({"type": "text", "text": self.context_text()}),
            json!({"type": "text", "text": self.request.text}),
        ];
        content.extend(files.parts(self.messages()));
        let mut input_line = json!({
            "type": "user",
            "thread_key": self.thread_key.as_str(),
            "trace_metadata": metadata,
            "message": {
                "role": "user",
                "content": content,
            },
        });
        // The harness applies these blocks-protocol fields to the turn.
        for (field, value) in [
            ("model", self.request.selection.model.as_deref()),
            ("provider", self.request.selection.provider.as_deref()),
            ("reasoning", self.request.selection.reasoning.as_deref()),
        ] {
            if let Some(value) = value {
                input_line[field] = json!(value);
            }
        }
        ExecuteSessionRequest {
            idempotency_key: Some(self.request.message.id.to_string()),
            metadata: Some(metadata),
            input_lines: vec![input_line.to_string()],
            idle_timeout_ms: limits.idle_timeout_ms,
            max_duration_ms: limits.max_duration_ms,
        }
    }

    /// Returns the context messages of other senders, oldest first.
    fn others(&self) -> impl DoubleEndedIterator<Item = &Message> {
        self.context
            .iter()
            .filter(|message| message.sender_id != self.bot)
    }

    /// Returns the messages that the session stores. These are the context
    /// messages of other senders and then the trigger.
    fn messages(&self) -> impl Iterator<Item = &Message> {
        self.others().chain([self.request.message])
    }

    /// Returns the text that the session stores for `message`: a line that
    /// names the sender and then the text. The trigger keeps its text
    /// without the flags.
    fn stored_text(&self, message: &Message) -> String {
        let text = if message.id == self.request.message.id {
            self.request.text
        } else {
            &message.content
        };
        format!(
            "{} (Zulip user {}):\n{text}",
            message.sender_full_name, message.sender_id
        )
    }

    /// Returns the text that precedes the request. The text names the sender
    /// and the conversation. It also lists the messages that the agent has
    /// not seen.
    fn context_text(&self) -> String {
        let place = match self.conversation {
            Conversation::Topic {
                channel_name,
                topic,
                ..
            } => format!("the topic \"{topic}\" in the channel #{channel_name}"),
            Conversation::Direct { .. } if self.conversation.is_one_to_one() => {
                "a direct message to you".to_owned()
            }
            Conversation::Direct { .. } => format!(
                "a group direct message with {}",
                self.conversation.display_name()
            ),
        };
        let mut text = format!(
            "# Zulip Conversation\n\nThis request is from {} (Zulip user {}) in {place}.\n\n{}",
            self.request.message.sender_full_name,
            self.request.message.sender_id,
            self.request.requester.context(),
        );
        if !self.context.is_empty() {
            text.push_str(match self.start {
                ContextStart::AfterHandled => {
                    "\nMessages sent since the previous request, oldest first:\n"
                }
                ContextStart::Oldest => "\nEarlier messages in this conversation, oldest first:\n",
                ContextStart::Restart => {
                    "\nThis conversation moved to a new agent harness. The harness has no \
                     history. Earlier messages in this conversation, oldest first:\n"
                }
            });
            for (index, message) in self.context.iter().enumerate() {
                // Writing to a `String` cannot fail.
                let _ = if message.sender_id == self.bot {
                    write!(text, "\n{}. You, in an earlier answer:\n", index + 1)
                } else {
                    write!(
                        text,
                        "\n{}. {} (user {}):\n",
                        index + 1,
                        message.sender_full_name,
                        message.sender_id
                    )
                };
                for line in message.content.lines() {
                    let _ = writeln!(text, "   {line}");
                }
            }
        }
        text.push_str(
            "\nThe request follows in the next content block. The files of these \
             messages follow the request.\n---",
        );
        text
    }
}

fn message_metadata(message: &Message) -> Value {
    json!({
        "source": SOURCE,
        "platform": PLATFORM,
        "message_id": message.id.to_string(),
        // iron-control reads the acting user from `user_id`.
        "user_id": message.sender_id.to_string(),
        "user_name": message.sender_full_name,
        "timestamp": message.timestamp,
    })
}

#[cfg(test)]
mod tests;
