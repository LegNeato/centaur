//! The harness, model, and persona of a request.
//!
//! The flags of the request come first. The default of its channel applies
//! to a request that selects no harness, model, or provider. The deployment
//! default comes last. A model of the custom provider catalog runs on Codex
//! with its provider.

mod catalog;
mod channel;

use centaur_session_core::HarnessType;
use thiserror::Error;

pub use catalog::ModelCatalog;
pub use channel::{ChannelDefault, ChannelDefaults};

use crate::{flags::Flags, policy::Allowlist};
use centaur_zulip::ChannelId;

/// The notice for a model that the catalog cannot resolve.
pub const SELECTION_ERROR_NOTICE: &str = "The model selection is ambiguous, or it does not fit \
     the requested provider or harness. Name its configured provider and use the Codex harness.";

/// The deployment settings that a selection uses.
#[derive(Clone, Debug)]
pub struct Choices {
    /// The harness of a new session without a selection.
    default_harness: HarnessType,
    /// The harnesses that a flag can select.
    enabled_harnesses: Allowlist<HarnessType>,
    catalog: ModelCatalog,
    channel_defaults: ChannelDefaults,
}

/// A channel default that selects a harness that the deployment does not
/// enable.
#[derive(Debug, Error, Eq, PartialEq)]
#[error(
    "the default of channel {channel} selects the harness {harness}. \
     ZULIPBOT_ENABLED_HARNESSES does not enable it."
)]
pub struct DisabledDefaultHarness {
    pub channel: ChannelId,
    pub harness: HarnessType,
}

/// The effective choices of one request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    /// The harness of a new session.
    pub harness: HarnessType,
    /// True when the request selects `harness` itself. A session on another
    /// harness then restarts on it.
    pub restart: bool,
    /// The persona of a new session.
    pub persona_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning: Option<String>,
    /// True when a flag selects the harness, the model, or the provider.
    pub explicit: bool,
    /// A harness flag that the deployment does not enable. The request then
    /// runs without its harness, model, and provider flags.
    pub refused_harness: Option<HarnessType>,
    /// The harness that a channel default chose the model and the provider
    /// for.
    default_harness: Option<HarnessType>,
}

impl Selection {
    /// Fits the selection to the harness of the opened session. A channel
    /// default chose its model and its provider for its own harness. A
    /// session that keeps another harness runs without them.
    pub fn fit(&mut self, session_harness: HarnessType) {
        if self
            .default_harness
            .is_some_and(|harness| harness != session_harness)
        {
            self.model = None;
            self.provider = None;
        }
    }
}

/// A model flag that the catalog cannot resolve. The bot rejects the
/// request. It still opens the session with `fallback`, the selection of
/// the request without its harness, model, and provider flags.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("the model selection does not resolve")]
pub struct Rejected {
    pub fallback: Selection,
}

/// A harness, a model, and a provider that belong together.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Choice {
    harness: Option<HarnessType>,
    model: Option<String>,
    provider: Option<String>,
}

impl Choices {
    /// Checks that each channel default selects an enabled harness. The
    /// default harness of the deployment needs no entry in the allowlist.
    pub fn new(
        default_harness: HarnessType,
        enabled_harnesses: Allowlist<HarnessType>,
        catalog: ModelCatalog,
        channel_defaults: ChannelDefaults,
    ) -> Result<Self, DisabledDefaultHarness> {
        let choices = Self {
            default_harness,
            enabled_harnesses,
            catalog,
            channel_defaults,
        };
        for (channel, default) in choices.channel_defaults.iter() {
            if let Some(harness) = choices.resolve(Choice::of_default(default)).harness
                && harness != default_harness
                && !choices.enabled_harnesses.allows(&harness)
            {
                return Err(DisabledDefaultHarness { channel, harness });
            }
        }
        Ok(choices)
    }

    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    /// Resolves the selection of a request with `flags` in `channel`. A
    /// direct message has no channel.
    pub fn select(&self, flags: &Flags, channel: Option<ChannelId>) -> Result<Selection, Rejected> {
        let channel_default = channel.and_then(|channel| self.channel_defaults.get(channel));
        let explicit = flags.harness.is_some() || flags.model.is_some() || flags.provider.is_some();
        let (mut choice, default_harness) = if explicit {
            let requested = Choice {
                harness: flags.harness,
                model: flags.model.clone(),
                provider: flags.provider.clone(),
            };
            let Some(choice) = self.resolve_strictly(requested) else {
                let kept = Flags {
                    persona_id: flags.persona_id.clone(),
                    reasoning: flags.reasoning,
                    ..Flags::default()
                };
                let fallback = self
                    .select(&kept, channel)
                    .expect("a request without a harness, a model, or a provider resolves");
                return Err(Rejected { fallback });
            };
            (choice, None)
        } else {
            let choice = channel_default
                .map(|default| self.resolve(Choice::of_default(default)))
                .unwrap_or_default();
            let harness = choice.harness;
            (choice, harness)
        };
        let mut refused_harness = None;
        if explicit
            && let Some(requested) = choice.harness
            && !self.enabled_harnesses.allows(&requested)
        {
            refused_harness = Some(requested);
            choice = Choice::default();
        }
        let requested_reasoning = flags
            .reasoning
            .or(channel_default.and_then(|default| default.reasoning));
        let reasoning = match choice
            .model
            .as_deref()
            .and_then(|model| self.catalog.find(model, choice.provider.as_deref()))
        {
            Some(model) => model.reasoning(requested_reasoning),
            None => requested_reasoning.map(str::to_owned),
        };
        Ok(Selection {
            restart: explicit && choice.harness.is_some(),
            harness: choice.harness.unwrap_or(self.default_harness),
            persona_id: flags
                .persona_id
                .clone()
                .or_else(|| channel_default.and_then(|default| default.persona_id.clone())),
            model: choice.model,
            provider: choice.provider,
            reasoning,
            explicit,
            refused_harness,
            default_harness,
        })
    }

    /// Resolves a channel default. A default that the catalog cannot resolve
    /// selects nothing.
    fn resolve(&self, choice: Choice) -> Choice {
        self.resolve_strictly(choice).unwrap_or_default()
    }

    /// Resolves a model of the catalog to its provider and Codex. A provider
    /// without a model gets the default model of the provider. A model that
    /// the catalog does not name stays as it is. It returns `None` for a
    /// catalog model that is ambiguous or does not fit the harness.
    fn resolve_strictly(&self, choice: Choice) -> Option<Choice> {
        let Choice {
            harness,
            model,
            provider,
        } = choice;
        let model = model.or_else(|| {
            provider
                .as_deref()
                .and_then(|provider| self.catalog.default_model(provider))
                .map(str::to_owned)
        });
        let Some(name) = model else {
            return Some(Choice {
                harness,
                model: None,
                provider,
            });
        };
        if !self.catalog.names(&name) {
            return Some(Choice {
                harness,
                model: Some(name),
                provider,
            });
        }
        let found = self.catalog.find(&name, provider.as_deref())?;
        harness
            .is_none_or(|harness| harness == HarnessType::Codex)
            .then(|| Choice {
                harness: Some(HarnessType::Codex),
                model: Some(found.id.to_owned()),
                provider: Some(found.provider.to_owned()),
            })
    }
}

impl Choice {
    fn of_default(default: &ChannelDefault) -> Self {
        Self {
            harness: default.harness,
            model: default.model.clone(),
            provider: default.provider.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
