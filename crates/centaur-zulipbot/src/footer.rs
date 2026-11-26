//! The metadata line under an answer.
//!
//! The line names the model, the custom provider, the harness, and the
//! reasoning effort of the answer, for example `Opus 5.5 · Claude Code ·
//! High`. slackbotv2 shows the same line
//! (`services/slackbotv2/src/response-context.ts`).

use std::{collections::BTreeMap, sync::LazyLock};

use centaur_session_core::HarnessType;
use clap::ValueEnum;
use regex::Regex;
use serde::Deserialize;

use crate::{
    selection::{ModelCatalog, Selection},
    sessions::OpenedSession,
};

/// The harness config files that the sandbox image bakes in.
const CLAUDE_SETTINGS: &str = include_str!("../../../harness/claude/settings.json");
const CODEX_CONFIG: &str = include_str!("../../../harness/codex/config.toml");

/// A Claude model ID: the family, the version, variant words, and a date,
/// for example `claude-haiku-4-5-20251001`.
static CLAUDE_MODEL_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^claude-([a-z]+)((?:-\d{1,2})*)((?:-[a-z]+)*)(?:-\d{8})?$")
        .expect("the Claude model pattern is valid")
});
/// A GPT model ID: the version, variant words, and a date, for example
/// `gpt-5.4-pro`.
static GPT_MODEL_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^gpt-(\d+(?:\.\d+)?)((?:-[a-z]+)*)(?:-\d{4}-\d{2}-\d{2})?$")
        .expect("the GPT model pattern is valid")
});
/// The GPT codenames that replace the `GPT` prefix, as in `Sol 5.6`.
const GPT_CODENAMES: [&str; 4] = ["sol", "luna", "terra", "astra"];

/// When an answer gets the metadata line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum MetadataMode {
    /// The first answer of a conversation and every answer to a request
    /// with a selection flag.
    #[default]
    First,
    Always,
    Never,
}

impl MetadataMode {
    /// Returns true when the answer gets the metadata line. `explicit` marks
    /// a request with a selection flag. `first` marks the first answer of a
    /// conversation.
    pub const fn shows(self, explicit: bool, first: bool) -> bool {
        match self {
            Self::First => explicit || first,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// The model and the reasoning effort that each harness runs without a
/// selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HarnessDefaults {
    models: BTreeMap<HarnessType, String>,
    reasoning: BTreeMap<HarnessType, String>,
}

/// The deployment variables that override the baked defaults. The chart
/// copies them from `sandbox.extraEnv`.
#[derive(Clone, Debug, Default)]
pub struct SandboxDefaults {
    pub claude_model: Option<String>,
    pub codex_model: Option<String>,
    pub pi_model: Option<String>,
    pub omp_model: Option<String>,
    pub codex_reasoning: Option<String>,
}

impl HarnessDefaults {
    /// Returns the baked defaults with the settings of `sandbox` applied. Nanocode shares
    /// the Codex config.
    pub fn new(sandbox: SandboxDefaults) -> Self {
        #[derive(Deserialize)]
        struct ClaudeSettings {
            model: Option<String>,
        }
        #[derive(Deserialize)]
        struct CodexConfig {
            model: Option<String>,
            model_reasoning_effort: Option<String>,
        }
        let claude: ClaudeSettings =
            serde_json::from_str(CLAUDE_SETTINGS).expect("the baked Claude settings are JSON");
        let codex: CodexConfig =
            toml::from_str(CODEX_CONFIG).expect("the baked Codex config is TOML");
        let codex_model = sandbox.codex_model.or(codex.model);
        let codex_reasoning = sandbox.codex_reasoning.or(codex.model_reasoning_effort);
        let models = [
            (
                HarnessType::ClaudeCode,
                sandbox.claude_model.or(claude.model),
            ),
            (HarnessType::Codex, codex_model.clone()),
            (HarnessType::Nanocode, codex_model),
            (HarnessType::Pi, sandbox.pi_model),
            (HarnessType::Omp, sandbox.omp_model),
        ];
        let reasoning = [
            (HarnessType::Codex, codex_reasoning.clone()),
            (HarnessType::Nanocode, codex_reasoning),
        ];
        Self {
            models: present(models),
            reasoning: present(reasoning),
        }
    }
}

fn present<const N: usize>(
    entries: [(HarnessType, Option<String>); N],
) -> BTreeMap<HarnessType, String> {
    entries
        .into_iter()
        .filter_map(|(harness, value)| {
            let value = value?.trim().to_owned();
            (!value.is_empty()).then_some((harness, value))
        })
        .collect()
}

/// Returns the metadata line of an answer of `session` to a request with
/// `selection`.
pub fn line(
    selection: &Selection,
    session: &OpenedSession,
    defaults: &HarnessDefaults,
    catalog: &ModelCatalog,
) -> Option<String> {
    let harness = &session.harness;
    // A persona that pins a model replaces the requested model and provider.
    let (model, provider) = match &session.persona_model {
        Some(pinned) => (Some(pinned.as_str()), None),
        None => (
            selection
                .model
                .as_deref()
                .or_else(|| defaults.models.get(harness).map(String::as_str)),
            selection.provider.as_deref(),
        ),
    };
    let catalog_model = model.and_then(|model| catalog.find(model, provider));
    let reasoning = match harness {
        _ if catalog_model.is_some() => selection.reasoning.as_deref(),
        // The defaults of these harnesses depend on the model.
        HarnessType::ClaudeCode | HarnessType::Pi | HarnessType::Omp => {
            selection.reasoning.as_deref()
        }
        HarnessType::Codex | HarnessType::Nanocode => selection
            .reasoning
            .as_deref()
            .or_else(|| defaults.reasoning.get(harness).map(String::as_str)),
        HarnessType::Amp | HarnessType::Hermes => None,
    };
    let segments = [
        model.map(|model| match catalog_model {
            Some(found) => found.display_name().to_owned(),
            None => model_display_name(model),
        }),
        provider
            .and_then(|provider| catalog.provider_name(provider))
            .map(str::to_owned),
        Some(harness.display_name().to_owned()),
        reasoning.map(reasoning_display_name),
    ];
    let text = segments
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    (!text.is_empty()).then(|| format!("*{text}*"))
}

fn reasoning_display_name(reasoning: &str) -> String {
    match reasoning.to_lowercase().as_str() {
        "xhigh" => "XHigh".to_owned(),
        other => title_case(other),
    }
}

/// Returns the product name of a model ID. `claude-opus-5-5` becomes
/// `Opus 5.5`. `gpt-5.6-sol` becomes `Sol 5.6`. `gpt-5.2` becomes `GPT 5.2`.
/// Another ID is shown in uppercase.
fn model_display_name(model: &str) -> String {
    let id = model.to_lowercase();
    if let Some(gpt) = GPT_MODEL_ID.captures(&id) {
        let version = &gpt[1];
        let variant = &gpt[2];
        let word = variant.trim_start_matches('-');
        if GPT_CODENAMES.contains(&word) {
            return format!("{} {version}", title_case(word));
        }
        return ["GPT".to_owned(), version.to_owned(), title_case(variant)]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let Some(claude) = CLAUDE_MODEL_ID.captures(&id) else {
        return model.to_uppercase();
    };
    [
        title_case(&claude[1]),
        claude[2].trim_start_matches('-').replace('-', "."),
        title_case(&claude[3]),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

/// Capitalizes each word. Spaces, `-`, and `_` separate the words.
fn title_case(text: &str) -> String {
    text.split([' ', '-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
