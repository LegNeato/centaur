//! Command-line and environment configuration.

use std::{fmt, net::SocketAddr, time::Duration};

use centaur_session_core::HarnessType;
use clap::Parser;
use url::Url;

use crate::{
    bot::Settings,
    footer::{HarnessDefaults, MetadataMode, SandboxDefaults},
    policy::{Allowlist, Policy},
    selection::{ChannelDefaults, Choices, DisabledDefaultHarness, ModelCatalog},
    turn::ExecutionLimits,
};
use centaur_zulip::{ChannelId, UserId, ZulipCredentials};

/// The default of [`Config::turn_file_bytes`]. A turn with more bytes is
/// rare in chat. api-rs keeps two copies of the bytes.
const DEFAULT_TURN_FILE_BYTES: u64 = 25 * 1024 * 1024;

/// The bot configuration. Each setting is both a flag and an environment
/// variable.
#[derive(Clone, Parser)]
#[command(
    name = "centaur-zulipbot",
    about = "Answer Zulip mentions and direct messages with Centaur sessions"
)]
pub struct Config {
    /// The URL of the Zulip organization, for example `https://chat.example.com`.
    #[arg(long, env = "ZULIP_SITE")]
    pub zulip_site: Url,
    /// The bot account's email address.
    #[arg(long, env = "ZULIP_EMAIL")]
    pub zulip_email: String,
    #[arg(long, env = "ZULIP_API_KEY", hide_env_values = true)]
    pub zulip_api_key: String,
    #[arg(long, env = "CENTAUR_API_URL", default_value = "http://127.0.0.1:8080")]
    pub centaur_api_url: Url,
    /// The ingress key that api-rs accepts for `zulip:` sessions.
    #[arg(
        long = "centaur-api-key",
        env = "ZULIPBOT_API_KEY",
        hide_env_values = true
    )]
    pub centaur_api_key: String,
    /// The channel IDs whose topics the bot answers in. `*` selects every
    /// channel. An empty list selects no channel.
    #[arg(long, env = "ZULIPBOT_CHANNEL_ALLOWLIST", default_value = "")]
    pub channel_allowlist: Allowlist<ChannelId>,
    /// The user IDs that may send the bot direct messages. `*` selects every
    /// member. An empty list selects no user.
    #[arg(long, env = "ZULIPBOT_DIRECT_MESSAGE_ALLOWLIST", default_value = "")]
    pub direct_message_allowlist: Allowlist<UserId>,
    /// The bot account IDs whose messages may start work. An empty list
    /// selects no bot.
    #[arg(long, env = "ZULIPBOT_TRIGGER_BOT_ALLOWLIST", default_value = "")]
    pub trigger_bot_allowlist: Allowlist<UserId>,
    /// The guest account IDs whose messages may start work. `*` selects every
    /// guest. An empty list selects no guest.
    #[arg(long, env = "ZULIPBOT_GUEST_ALLOWLIST", default_value = "")]
    pub guest_allowlist: Allowlist<UserId>,
    /// The harness that new sessions run.
    #[arg(long, env = "ZULIPBOT_HARNESS", default_value = "codex")]
    pub harness: HarnessType,
    /// The harnesses that a harness flag can select. `*` selects every
    /// harness.
    #[arg(long, env = "ZULIPBOT_ENABLED_HARNESSES", default_value = "*")]
    pub enabled_harnesses: Allowlist<HarnessType>,
    /// The defaults of each channel as a JSON object keyed by channel ID.
    #[arg(long, env = "ZULIPBOT_CHANNEL_DEFAULTS", default_value = "")]
    pub channel_defaults: ChannelDefaults,
    /// The custom Codex providers and their models as JSON. The chart renders
    /// `codex.customProviders` here.
    #[arg(long, env = "CODEX_CUSTOM_PROVIDERS", default_value = "")]
    pub codex_custom_providers: ModelCatalog,
    /// When an answer gets the model and harness line.
    #[arg(
        long,
        env = "ZULIPBOT_RESPONSE_METADATA_MODE",
        value_enum,
        default_value_t
    )]
    pub response_metadata_mode: MetadataMode,
    /// The Claude Code model of the sandboxes. It replaces the baked default
    /// in the metadata line.
    #[arg(long, env = "CLAUDE_MODEL")]
    pub claude_model: Option<String>,
    /// The Codex and Nanocode model of the sandboxes.
    #[arg(long, env = "CODEX_MODEL")]
    pub codex_model: Option<String>,
    /// The Codex and Nanocode reasoning effort of the sandboxes.
    #[arg(long, env = "CODEX_MODEL_REASONING_EFFORT")]
    pub codex_model_reasoning_effort: Option<String>,
    /// The Pi model of the sandboxes.
    #[arg(long, env = "CENTAUR_PI_MODEL")]
    pub pi_model: Option<String>,
    /// The OMP model of the sandboxes.
    #[arg(long, env = "CENTAUR_OMP_MODEL")]
    pub omp_model: Option<String>,
    /// The maximum number of earlier messages that one turn reads for context.
    #[arg(
        long,
        env = "ZULIPBOT_CONTEXT_MESSAGES",
        default_value_t = 50,
        value_parser = clap::value_parser!(u32).range(1..=1000)
    )]
    pub context_messages: u32,
    /// The maximum age in seconds of an unanswered message that the startup
    /// scan answers.
    #[arg(long, env = "ZULIPBOT_RECOVERY_WINDOW_SECONDS", default_value_t = 3600)]
    pub recovery_window_seconds: u64,
    /// The maximum total size in bytes of the files that one turn attaches.
    /// Zero attaches no files.
    #[arg(long, env = "ZULIPBOT_TURN_FILE_BYTES", default_value_t = DEFAULT_TURN_FILE_BYTES)]
    pub turn_file_bytes: u64,
    /// The execution idle timeout that the bot sends to api-rs. The api-rs
    /// default applies when it is not set.
    #[arg(long, env = "SESSION_IDLE_TIMEOUT_MS")]
    pub idle_timeout_ms: Option<u64>,
    /// The maximum execution duration that the bot sends to api-rs. The
    /// api-rs default applies when it is not set.
    #[arg(long, env = "SESSION_MAX_DURATION_MS")]
    pub max_duration_ms: Option<u64>,
    /// The address of the `/healthz` endpoint.
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8080")]
    pub bind_addr: SocketAddr,
}

impl Config {
    pub fn zulip_credentials(&self) -> ZulipCredentials {
        ZulipCredentials {
            site: self.zulip_site.clone(),
            email: self.zulip_email.clone(),
            api_key: self.zulip_api_key.clone(),
        }
    }

    pub fn policy(&self) -> Policy {
        Policy {
            channels: self.channel_allowlist.clone(),
            direct_message_senders: self.direct_message_allowlist.clone(),
            trigger_bots: self.trigger_bot_allowlist.clone(),
            trigger_guests: self.guest_allowlist.clone(),
        }
    }

    /// Returns the bot settings. A channel default that selects a disabled
    /// harness is an error.
    pub fn settings(&self) -> Result<Settings, DisabledDefaultHarness> {
        Ok(Settings {
            choices: Choices::new(
                self.harness,
                self.enabled_harnesses.clone(),
                self.codex_custom_providers.clone(),
                self.channel_defaults.clone(),
            )?,
            metadata: self.response_metadata_mode,
            harness_defaults: HarnessDefaults::new(SandboxDefaults {
                claude_model: self.claude_model.clone(),
                codex_model: self.codex_model.clone(),
                pi_model: self.pi_model.clone(),
                omp_model: self.omp_model.clone(),
                codex_reasoning: self.codex_model_reasoning_effort.clone(),
            }),
            context_messages: self.context_messages,
            limits: ExecutionLimits {
                idle_timeout_ms: self.idle_timeout_ms,
                max_duration_ms: self.max_duration_ms,
            },
            recovery_window: Duration::from_secs(self.recovery_window_seconds),
            file_bytes: self.turn_file_bytes,
        })
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("zulip_site", &self.zulip_site.as_str())
            .field("zulip_email", &self.zulip_email)
            .field("centaur_api_url", &self.centaur_api_url.as_str())
            .field("policy", &self.policy())
            .field("settings", &self.settings())
            .field("bind_addr", &self.bind_addr)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use centaur_session_core::HarnessType;
    use clap::Parser;

    use super::Config;
    use crate::policy::Allowlist;
    use centaur_zulip::{ChannelId, UserId};

    const REQUIRED: [&str; 7] = [
        "centaur-zulipbot",
        "--zulip-site=https://chat.example.test",
        "--zulip-email=centaur-bot@chat.example.test",
        "--zulip-api-key=zulip-secret",
        "--centaur-api-key=centaur-secret",
        "--channel-allowlist=42,43",
        "--direct-message-allowlist=*",
    ];

    #[test]
    fn enabled_harnesses_default_to_every_harness_and_parse_a_list() {
        let config = Config::try_parse_from(REQUIRED.to_vec()).unwrap();
        assert_eq!(config.enabled_harnesses, Allowlist::Everyone);

        let mut args = REQUIRED.to_vec();
        args.push("--enabled-harnesses=codex,claudecode");
        let config = Config::try_parse_from(args).unwrap();
        assert!(config.enabled_harnesses.allows(&HarnessType::ClaudeCode));
        assert!(!config.enabled_harnesses.allows(&HarnessType::Amp));

        let mut args = REQUIRED.to_vec();
        args.push("--enabled-harnesses=claude");
        assert!(Config::try_parse_from(args).is_err());
    }

    #[test]
    fn parses_allowlists_and_defaults() {
        let config = Config::try_parse_from(REQUIRED).unwrap();
        let policy = config.policy();
        assert_eq!(
            policy.channels,
            Allowlist::Only(BTreeSet::from([ChannelId(42), ChannelId(43)]))
        );
        assert_eq!(policy.direct_message_senders, Allowlist::Everyone);
        assert!(!policy.trigger_bots.allows(&UserId(1)));
        assert!(policy.trigger_guests.is_empty());
        assert_eq!(config.context_messages, 50);
        assert_eq!(config.centaur_api_url.as_str(), "http://127.0.0.1:8080/");
    }

    #[test]
    fn parses_the_guest_allowlist() {
        let mut args = REQUIRED.to_vec();
        args.push("--guest-allowlist=60");
        let config = Config::try_parse_from(args).unwrap();
        assert_eq!(
            config.policy().trigger_guests,
            Allowlist::Only(BTreeSet::from([UserId(60)]))
        );
    }

    #[test]
    fn rejects_a_malformed_allowlist() {
        let mut args = REQUIRED.to_vec();
        args.push("--trigger-bot-allowlist=alerts-bot");
        assert!(Config::try_parse_from(args).is_err());
    }

    #[test]
    fn rejects_a_context_limit_out_of_range() {
        let mut args = REQUIRED.to_vec();
        args.push("--context-messages=0");
        assert!(Config::try_parse_from(args).is_err());
    }

    #[test]
    fn debug_output_leaves_out_secrets() {
        let config = Config::try_parse_from(REQUIRED).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("zulip-secret"), "{debug}");
        assert!(!debug.contains("centaur-secret"), "{debug}");
    }
}
