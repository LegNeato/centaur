//! The text commands of a message that addresses the bot.

use std::sync::LazyLock;

use regex::Regex;

/// The stop grammar of slackbotv2 (`services/slackbotv2/src/stop-command.ts`).
/// The whole text must be the command, for example `please stop the run`.
static STOP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)^",
        r"(?:(?:please|pls)\s+)?",
        r"(?:(?:can|could|would|will)\s+you\s+)?",
        r"(?:stop+|kill(?:ed|ing|s)?|end(?:ed|ing|s)?|cancell?(?:ed|ing|s)?)",
        r"(?:\s+(?:it|this|that|now|please|pls|the\s+(?:run|execution|request|job|thread|turn)))*",
        r"[.!?]*$",
    ))
    .expect("the stop pattern is valid")
});

/// A user mention (`@**Ada**`, `@**Ada|7**`) or a user group mention
/// (`@*support*`). A leading `_` makes the mention silent.
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@_?(?:\*\*[^*]+\*\*|\*[^*]+\*)").expect("the mention pattern is valid")
});

/// Returns true when the Markdown source `content` is a stop command. The
/// mentions in the text do not count.
pub fn is_stop(content: &str) -> bool {
    let without_mentions = MENTION.replace_all(content, " ");
    let text = without_mentions
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    STOP.is_match(&text)
}

#[cfg(test)]
mod tests {
    use super::is_stop;

    #[test]
    fn stop_phrases_with_mentions_are_commands() {
        for content in [
            "@**Centaur** stop",
            "@**Centaur|99** STOP!",
            "@_**Centaur** please cancel the run.",
            "can you kill it now @**Centaur**",
            "@*bots* end this",
        ] {
            assert!(is_stop(content), "{content}");
        }
    }

    #[test]
    fn a_stop_phrase_without_a_mention_is_a_command() {
        for content in ["stop", "stoppp", "cancelled."] {
            assert!(is_stop(content), "{content}");
        }
    }

    #[test]
    fn a_request_that_mentions_stopping_is_not_a_command() {
        for content in [
            "@**Centaur** stop the deploy job on staging",
            "@**Centaur** why did the run end?",
            "@**Centaur**",
            "don't stop",
            "",
        ] {
            assert!(!is_stop(content), "{content}");
        }
    }
}
