use std::{fmt, num::ParseIntError, str::FromStr};

use serde::{Deserialize, Serialize};

macro_rules! id_types {
    ($($(#[$attr:meta])* $name:ident),* $(,)?) => {
        $(
            $(#[$attr])*
            #[derive(
                Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
            )]
            #[serde(transparent)]
            pub struct $name(pub u64);

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    self.0.fmt(f)
                }
            }

            impl From<u64> for $name {
                fn from(id: u64) -> Self {
                    Self(id)
                }
            }

            impl FromStr for $name {
                type Err = ParseIntError;

                fn from_str(value: &str) -> Result<Self, Self::Err> {
                    value.parse().map(Self)
                }
            }
        )*
    };
}

id_types! {
    /// A message ID. Message IDs increase with send time across the server.
    MessageId,
    UserId,
    /// A channel ID. Older Zulip documentation calls channels streams.
    ChannelId,
    /// The ID of a custom profile field of the organization.
    ProfileFieldId,
}

/// The ID of a registered event queue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(transparent)]
pub struct QueueId(pub String);

/// The position of an event in its queue. A new queue starts at -1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EventId(pub i64);

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
