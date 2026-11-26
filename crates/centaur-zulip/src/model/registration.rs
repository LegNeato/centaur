use std::time::Duration;

use serde::Deserialize;

use super::{EventId, ProfileFieldId, QueueId};

/// A registered event queue and the server settings that Centaur uses.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(from = "WireRegistration")]
pub struct Registration {
    pub queue_id: QueueId,
    pub last_event_id: EventId,
    pub server: ServerSettings,
}

/// The server settings that Centaur reads at each queue registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerSettings {
    pub feature_level: u64,
    /// The maximum message length in characters.
    pub max_message_length: usize,
    /// The interval at which a typing client renews its typing notification.
    pub typing_refresh_interval: Duration,
    /// The custom profile fields that can hold a GitHub handle. The GitHub
    /// account fields come first. The fields whose name mentions GitHub
    /// follow.
    pub github_fields: Vec<ProfileFieldId>,
}

impl Default for ServerSettings {
    /// The Zulip defaults. A client uses them until its first registration.
    fn default() -> Self {
        Self {
            feature_level: 0,
            max_message_length: 10_000,
            typing_refresh_interval: Duration::from_secs(10),
            github_fields: Vec::new(),
        }
    }
}

/// The register response. The `realm` and `custom_profile_fields` initial
/// states supply the server settings.
#[derive(Deserialize)]
struct WireRegistration {
    queue_id: QueueId,
    last_event_id: EventId,
    zulip_feature_level: u64,
    max_message_length: usize,
    server_typing_started_wait_period_milliseconds: u64,
    #[serde(default)]
    custom_profile_fields: Vec<WireProfileField>,
}

impl From<WireRegistration> for Registration {
    fn from(wire: WireRegistration) -> Self {
        Self {
            queue_id: wire.queue_id,
            last_event_id: wire.last_event_id,
            server: ServerSettings {
                feature_level: wire.zulip_feature_level,
                max_message_length: wire.max_message_length,
                typing_refresh_interval: Duration::from_millis(
                    wire.server_typing_started_wait_period_milliseconds,
                ),
                github_fields: github_fields(&wire.custom_profile_fields),
            },
        }
    }
}

/// The Zulip field type of an external account field.
const EXTERNAL_ACCOUNT_FIELD: u8 = 7;

/// A custom profile field definition in the register response.
#[derive(Deserialize)]
struct WireProfileField {
    id: ProfileFieldId,
    name: String,
    #[serde(rename = "type")]
    kind: u8,
    /// A JSON object as a string. An external account field names its
    /// account kind in `subtype`.
    #[serde(default)]
    field_data: String,
}

impl WireProfileField {
    fn is_github_account(&self) -> bool {
        #[derive(Deserialize)]
        struct ExternalAccount {
            subtype: String,
        }
        self.kind == EXTERNAL_ACCOUNT_FIELD
            && serde_json::from_str::<ExternalAccount>(&self.field_data)
                .is_ok_and(|data| data.subtype == "github")
    }

    fn mentions_github(&self) -> bool {
        self.name.to_lowercase().contains("github")
    }
}

/// Returns the fields that can hold a GitHub handle in the order of
/// [`ServerSettings::github_fields`].
fn github_fields(fields: &[WireProfileField]) -> Vec<ProfileFieldId> {
    let (accounts, others): (Vec<_>, Vec<_>) =
        fields.iter().partition(|field| field.is_github_account());
    let named = others.into_iter().filter(|field| field.mentions_github());
    accounts
        .into_iter()
        .chain(named)
        .map(|field| field.id)
        .collect()
}
