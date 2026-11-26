//! The operator's defaults for each channel.
//!
//! `ZULIPBOT_CHANNEL_DEFAULTS` holds a JSON object keyed by channel ID. Each
//! value names a `persona`, a `harness`, a `model`, a `provider`, or a
//! `reasoning` effort. The values use the vocabulary of the inline flags, for
//! example `"harness": "claude"` or `"model": "opus"`. slackbotv2 has the same
//! setting (`SLACKBOTV2_CHANNEL_DEFAULTS`).
//!
//! ```json
//! {"42": {"persona": "eng", "harness": "claude", "model": "opus", "reasoning": "high"}}
//! ```

use std::{collections::BTreeMap, str::FromStr, sync::LazyLock};

use centaur_session_core::HarnessType;
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;

use crate::flags::{harness_named, model_named, provider_named, reasoning_named};
use centaur_zulip::ChannelId;

/// The persona ID pattern of the `--persona` flag.
static PERSONA_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$").expect("the persona pattern is valid")
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChannelDefaults(BTreeMap<ChannelId, ChannelDefault>);

/// The defaults of one channel. Each field is independent.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChannelDefault {
    pub persona_id: Option<String>,
    pub harness: Option<HarnessType>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning: Option<&'static str>,
}

#[derive(Debug, Error)]
pub enum ChannelDefaultsError {
    #[error("channel defaults are not a JSON object of channel IDs: {0}")]
    Json(#[from] serde_json::Error),
    #[error("channel {channel}: {problem}")]
    Entry { channel: ChannelId, problem: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefault {
    persona: Option<String>,
    harness: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    reasoning: Option<String>,
}

impl ChannelDefaults {
    pub fn get(&self, channel: ChannelId) -> Option<&ChannelDefault> {
        self.0.get(&channel)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ChannelId, &ChannelDefault)> {
        self.0.iter().map(|(channel, default)| (*channel, default))
    }
}

impl FromStr for ChannelDefaults {
    type Err = ChannelDefaultsError;

    /// Parses the JSON object. An empty text has no defaults.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let raw: BTreeMap<ChannelId, RawDefault> = serde_json::from_str(text)?;
        raw.into_iter()
            .map(|(channel, raw)| {
                ChannelDefault::normalize(raw)
                    .map(|default| (channel, default))
                    .map_err(|problem| ChannelDefaultsError::Entry { channel, problem })
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

impl ChannelDefault {
    fn normalize(raw: RawDefault) -> Result<Self, String> {
        let clean = |value: Option<String>| {
            value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let mut default = Self::default();
        if let Some(harness) = clean(raw.harness) {
            default.harness =
                Some(harness_named(&harness).ok_or(format!("unknown harness {harness:?}"))?);
        }
        if let Some(provider) = clean(raw.provider) {
            default.provider =
                Some(provider_named(&provider).ok_or(format!("invalid provider {provider:?}"))?);
            // A provider selects Codex, as `--provider` does.
            default.harness.get_or_insert(HarnessType::Codex);
        }
        default.model = clean(raw.model).map(|model| model_named(&model));
        if let Some(reasoning) = clean(raw.reasoning) {
            default.reasoning = Some(
                reasoning_named(&reasoning)
                    .ok_or(format!("unknown reasoning effort {reasoning:?}"))?,
            );
        }
        if let Some(persona) = clean(raw.persona) {
            if !PERSONA_ID.is_match(&persona) {
                return Err(format!("invalid persona ID {persona:?}"));
            }
            default.persona_id = Some(persona);
        }
        if default == Self::default() {
            return Err("no persona, harness, model, provider, or reasoning".to_owned());
        }
        Ok(default)
    }
}

#[cfg(test)]
mod tests {
    use centaur_session_core::HarnessType;

    use super::{ChannelDefault, ChannelDefaults};
    use centaur_zulip::ChannelId;

    #[test]
    fn entries_use_the_flag_vocabulary() {
        let defaults: ChannelDefaults = r#"{
            "42": {"persona": "eng", "harness": "claude", "model": "opus", "reasoning": "hi"},
            "43": {"provider": "bedrock", "model": "gpt-5.2"}
        }"#
        .parse()
        .unwrap();

        assert_eq!(
            defaults.get(ChannelId(42)),
            Some(&ChannelDefault {
                persona_id: Some("eng".to_owned()),
                harness: Some(HarnessType::ClaudeCode),
                model: Some("claude-opus-5-5".to_owned()),
                provider: None,
                reasoning: Some("high"),
            })
        );
        let bedrock = defaults.get(ChannelId(43)).unwrap();
        assert_eq!(bedrock.provider.as_deref(), Some("amazon-bedrock"));
        assert_eq!(bedrock.harness, Some(HarnessType::Codex));
        assert_eq!(defaults.get(ChannelId(44)), None);
    }

    #[test]
    fn an_invalid_entry_names_its_channel() {
        for (text, problem) in [
            (
                r#"{"42": {"harness": "emacs"}}"#,
                "channel 42: unknown harness",
            ),
            (
                r#"{"42": {"reasoning": "loud"}}"#,
                "channel 42: unknown reasoning effort",
            ),
            (
                r#"{"42": {"persona": "a b"}}"#,
                "channel 42: invalid persona ID",
            ),
            (r#"{"42": {}}"#, "channel 42: no persona"),
        ] {
            let error = text.parse::<ChannelDefaults>().unwrap_err().to_string();
            assert!(error.starts_with(problem), "{error}");
        }
        assert!(r#"{"42": {"modle": "opus"}}"#.parse::<ChannelDefaults>().is_err());
        assert!(r#"{"general": {"model": "opus"}}"#.parse::<ChannelDefaults>().is_err());
    }

    #[test]
    fn empty_text_has_no_defaults() {
        assert_eq!(
            "".parse::<ChannelDefaults>().unwrap(),
            ChannelDefaults::default()
        );
    }
}
