use serde::Deserialize;

use super::{EventId, Message, message::MessageFlag};

/// One event from an event queue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(from = "WireEvent")]
pub struct Event {
    pub id: EventId,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// A new message. The mention flag of the event is applied.
    Message(Message),
    Heartbeat,
    Other,
}

impl Event {
    /// Returns the message of a message event.
    pub fn into_message(self) -> Option<Message> {
        match self.kind {
            EventKind::Message(message) => Some(message),
            EventKind::Heartbeat | EventKind::Other => None,
        }
    }
}

#[derive(Deserialize)]
struct WireEvent {
    id: EventId,
    #[serde(flatten)]
    kind: WireEventKind,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireEventKind {
    /// An event carries the flags for the reading user next to the message.
    Message {
        message: Message,
        #[serde(default)]
        flags: Vec<MessageFlag>,
    },
    Heartbeat,
    #[serde(other)]
    Other,
}

impl From<WireEvent> for Event {
    fn from(wire: WireEvent) -> Self {
        let kind = match wire.kind {
            WireEventKind::Message { mut message, flags } => {
                message.mentions_bot = MessageFlag::mention(&flags);
                EventKind::Message(message)
            }
            WireEventKind::Heartbeat => EventKind::Heartbeat,
            WireEventKind::Other => EventKind::Other,
        };
        Self { id: wire.id, kind }
    }
}
