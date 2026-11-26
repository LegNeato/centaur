//! The inline flags of a request.
//!
//! The flags follow slackbotv2 (`services/slackbotv2/src/overrides.ts`). The
//! bot removes each recognized flag from the request text.
//!
//! - `--codex`, `--claude`, `--claude-code`, `--amp`, `--hermes`,
//!   `--nanocode`, `--pi`, and `--omp` select the harness. A harness flag
//!   restarts a session that runs another harness.
//! - `--model <name>` selects the model of the turn. `fable`, `opus`,
//!   `sonnet`, and `haiku` expand to the current Claude model IDs. The bare
//!   flags `--fable`, `--opus`, `--sonnet`, and `--haiku` also select Claude
//!   Code.
//! - `--provider <name>` selects the model provider of the turn. `--bedrock`
//!   and `--meta` are shortcuts. A provider selects Codex.
//! - `-rsn <effort>` selects the reasoning effort of the turn.
//! - `--persona <id>` selects the persona of a new session.
//!
//! A value flag also accepts `=`, for example `--model=opus`.

use std::sync::LazyLock;

use centaur_session_core::HarnessType;
use regex::Regex;

/// The harness flags in the order that slackbotv2 checks them. The last flag
/// that the text holds wins.
const HARNESS_FLAGS: [(&str, HarnessType); 9] = [
    ("amp", HarnessType::Amp),
    ("claude", HarnessType::ClaudeCode),
    ("claude-code", HarnessType::ClaudeCode),
    ("claudecode", HarnessType::ClaudeCode),
    ("codex", HarnessType::Codex),
    ("hermes", HarnessType::Hermes),
    ("nanocode", HarnessType::Nanocode),
    ("pi", HarnessType::Pi),
    ("omp", HarnessType::Omp),
];

/// The Claude model aliases of slackbotv2. Each alias is also a bare flag
/// that selects Claude Code.
const CLAUDE_MODEL_ALIASES: [(&str, &str); 4] = [
    ("fable", "claude-fable-5-2"),
    ("haiku", "claude-haiku-5-5"),
    ("opus", "claude-opus-5-5"),
    ("sonnet", "claude-sonnet-5-5"),
];

/// The provider shortcut flags. Each one selects Codex.
const PROVIDER_FLAGS: [(&str, &str); 2] = [("bedrock", "amazon-bedrock"), ("meta", "responses")];

/// The reasoning efforts and their aliases.
const REASONING_EFFORTS: [(&str, &str); 13] = [
    ("none", "none"),
    ("minimal", "minimal"),
    ("min", "minimal"),
    ("low", "low"),
    ("medium", "medium"),
    ("med", "medium"),
    ("high", "high"),
    ("hi", "high"),
    ("xhigh", "xhigh"),
    ("xhi", "xhigh"),
    ("x-high", "xhigh"),
    ("max", "max"),
    ("ultra", "ultra"),
];

static MODEL: LazyLock<Regex> =
    LazyLock::new(|| value_flag("--model", "[A-Za-z0-9][A-Za-z0-9._/-]*"));
static PROVIDER: LazyLock<Regex> =
    LazyLock::new(|| value_flag("--provider", "[A-Za-z][A-Za-z0-9_-]*"));
static PERSONA: LazyLock<Regex> =
    LazyLock::new(|| value_flag("--persona", "[A-Za-z0-9][A-Za-z0-9._-]*"));
static REASONING: LazyLock<Regex> = LazyLock::new(|| value_flag("-rsn", "[A-Za-z-]+"));
static LEADING_LINE_BREAKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\r\n?|\n)+").expect("the line break pattern is valid"));

/// The choices that the flags of a request make.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Flags {
    /// The request text without the flags.
    pub text: String,
    pub harness: Option<HarnessType>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning: Option<&'static str>,
    pub persona_id: Option<String>,
}

impl Flags {
    pub fn parse(content: &str) -> Self {
        let mut text = content.to_owned();
        let mut flags = Self::default();
        if let Some(model) = take_value(&mut text, &MODEL) {
            flags.model = Some(model_named(&model));
        }
        // An unknown effort is not a flag and stays in the text.
        flags.reasoning = take_value_if(&mut text, &REASONING, reasoning_named);
        if let Some(provider) = take_value_if(&mut text, &PROVIDER, provider_named) {
            flags.provider = Some(provider);
            flags.harness = Some(HarnessType::Codex);
        }
        for (flag, harness) in HARNESS_FLAGS {
            if take_flag(&mut text, flag) {
                flags.harness = Some(harness);
            }
        }
        for (flag, model) in CLAUDE_MODEL_ALIASES {
            if take_flag(&mut text, flag) {
                flags.model.get_or_insert_with(|| model.to_owned());
                flags.harness.get_or_insert(HarnessType::ClaudeCode);
            }
        }
        for (flag, provider) in PROVIDER_FLAGS {
            if take_flag(&mut text, flag) {
                flags.provider.get_or_insert_with(|| provider.to_owned());
                flags.harness.get_or_insert(HarnessType::Codex);
            }
        }
        flags.persona_id = take_value(&mut text, &PERSONA);
        flags.text = if text == content {
            text
        } else {
            text.trim().to_owned()
        };
        flags
    }

    /// Returns true when a flag chooses a harness, a model, a provider, a
    /// reasoning effort, or a persona.
    pub fn chooses_anything(&self) -> bool {
        self.harness.is_some()
            || self.model.is_some()
            || self.provider.is_some()
            || self.reasoning.is_some()
            || self.persona_id.is_some()
    }
}

/// Returns the pattern of `flag` followed by a value on the same line.
fn value_flag(flag: &str, value: &str) -> Regex {
    Regex::new(&format!(
        r"(?i)(?:^|\s){}(?:[^\S\r\n]*=[^\S\r\n]*|[^\S\r\n]+)({value})",
        regex::escape(flag)
    ))
    .expect("the flag pattern is valid")
}

/// Returns the harness that a harness flag names without its dashes, for
/// example `claude` for `--claude`.
pub fn harness_named(name: &str) -> Option<HarnessType> {
    HARNESS_FLAGS
        .iter()
        .find(|(flag, _)| flag.eq_ignore_ascii_case(name))
        .map(|(_, harness)| *harness)
}

/// Returns the model ID of a Claude alias, or `name` itself.
pub fn model_named(name: &str) -> String {
    lookup(&CLAUDE_MODEL_ALIASES, name).map_or_else(|| name.to_owned(), str::to_owned)
}

/// Returns the provider ID of a provider name. A shortcut such as `bedrock`
/// maps to its provider. Any other valid name stays, in lowercase.
pub fn provider_named(name: &str) -> Option<String> {
    if let Some(provider) = lookup(&PROVIDER_FLAGS, name) {
        return Some(provider.to_owned());
    }
    let provider = name.to_lowercase();
    let mut characters = provider.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || "_-".contains(character)
        });
    valid.then_some(provider)
}

/// Returns the reasoning effort that `name` or one of its aliases names.
pub fn reasoning_named(name: &str) -> Option<&'static str> {
    lookup(&REASONING_EFFORTS, name)
}

fn lookup<const N: usize>(table: &[(&str, &'static str); N], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| *value)
}

/// Returns true when the flag ends at `end`. A flag ends at whitespace or
/// at the end of the text.
fn ends_at_boundary(text: &str, end: usize) -> bool {
    text[end..].chars().next().is_none_or(char::is_whitespace)
}

fn take_value(text: &mut String, pattern: &Regex) -> Option<String> {
    take_value_if(text, pattern, |value| Some(value.to_owned()))
}

/// Removes the first match of the value flag `pattern` whose value `accept`
/// maps. It returns the mapped value.
fn take_value_if<T>(
    text: &mut String,
    pattern: &Regex,
    accept: impl Fn(&str) -> Option<T>,
) -> Option<T> {
    let (start, end, value) = pattern.captures_iter(text).find_map(|captures| {
        let whole = captures.get(0)?;
        let value = captures.get(1)?;
        if !ends_at_boundary(text, whole.end()) {
            return None;
        }
        Some((whole.start(), whole.end(), accept(value.as_str())?))
    })?;
    strip(text, start, end);
    Some(value)
}

/// Removes the first bare `--<flag>` with the whitespace before it. It
/// returns true when the text held one.
fn take_flag(text: &mut String, flag: &str) -> bool {
    let needle = format!("--{flag}");
    let found = text.char_indices().find_map(|(index, _)| {
        let candidate = text.get(index..index + needle.len())?;
        if !candidate.eq_ignore_ascii_case(&needle) || !ends_at_boundary(text, index + needle.len())
        {
            return None;
        }
        let end = index + needle.len();
        match text[..index].chars().next_back() {
            None => Some((index, end)),
            Some(before) if before.is_whitespace() => Some((index - before.len_utf8(), end)),
            Some(_) => None,
        }
    });
    let Some((start, end)) = found else {
        return false;
    };
    strip(text, start, end);
    true
}

/// Removes `start..end` and the line breaks right after it. One space joins
/// the two sides when neither side has whitespace at the join.
fn strip(text: &mut String, start: usize, end: usize) {
    let before = &text[..start];
    let after = LEADING_LINE_BREAKS.replace(&text[end..], "");
    let joined = !before.is_empty()
        && !after.is_empty()
        && !before.ends_with(char::is_whitespace)
        && !after.starts_with(char::is_whitespace);
    *text = format!("{before}{}{after}", if joined { " " } else { "" });
}

#[cfg(test)]
mod tests;
