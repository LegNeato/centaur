use std::collections::BTreeSet;

use centaur_session_core::HarnessType;

use super::{ChannelDefaults, Choices, DisabledDefaultHarness, ModelCatalog, Rejected};
use crate::{flags::Flags, policy::Allowlist};
use centaur_zulip::ChannelId;

const CATALOG: &str = r#"{
    "private_responses": {
        "defaultModel": "example-model",
        "models": {"example-model": {"aliases": ["example"], "reasoningEfforts": ["low", "high"], "defaultReasoning": "high"}}
    },
    "other": {"models": {"shared": {}}},
    "third": {"models": {"shared": {}}}
}"#;

/// The channel with the Claude default of [`choices`].
const CLAUDE_CHANNEL: ChannelId = ChannelId(42);
/// The channel whose default names an ambiguous catalog model.
const AMBIGUOUS_CHANNEL: ChannelId = ChannelId(43);

const DEFAULTS: &str = r#"{
    "42": {"persona": "eng", "harness": "claude", "model": "opus", "reasoning": "low"},
    "43": {"model": "shared"}
}"#;

fn choices_with(enabled: Allowlist<HarnessType>) -> Result<Choices, DisabledDefaultHarness> {
    Choices::new(
        HarnessType::Codex,
        enabled,
        CATALOG.parse::<ModelCatalog>().unwrap(),
        DEFAULTS.parse::<ChannelDefaults>().unwrap(),
    )
}

fn choices() -> Choices {
    choices_with(Allowlist::Everyone).unwrap()
}

fn only(harnesses: &[HarnessType]) -> Allowlist<HarnessType> {
    Allowlist::Only(harnesses.iter().copied().collect::<BTreeSet<_>>())
}

mod construction {
    use super::*;

    #[test]
    fn a_default_on_a_disabled_harness_is_an_error() {
        assert_eq!(
            choices_with(only(&[HarnessType::Codex])).unwrap_err(),
            DisabledDefaultHarness {
                channel: CLAUDE_CHANNEL,
                harness: HarnessType::ClaudeCode,
            }
        );
    }

    #[test]
    fn a_default_on_an_enabled_or_the_deployment_harness_is_accepted() {
        choices_with(only(&[HarnessType::ClaudeCode])).unwrap();
        Choices::new(
            HarnessType::ClaudeCode,
            only(&[]),
            ModelCatalog::default(),
            r#"{"42": {"harness": "claude"}}"#.parse().unwrap(),
        )
        .unwrap();
    }
}

mod flags {
    use super::*;

    #[test]
    fn no_flag_and_no_default_selects_the_deployment_harness() {
        let selection = choices().select(&Flags::parse("hi"), None).unwrap();

        assert_eq!(selection.harness, HarnessType::Codex);
        assert!(!selection.restart);
        assert!(!selection.explicit);
        assert_eq!(selection.model, None);
    }

    #[test]
    fn a_catalog_model_selects_codex_and_its_provider_and_restarts() {
        let selection = choices()
            .select(&Flags::parse("--model example -rsn low go"), None)
            .unwrap();

        assert_eq!(selection.harness, HarnessType::Codex);
        assert!(selection.restart);
        assert_eq!(selection.model.as_deref(), Some("example-model"));
        assert_eq!(selection.provider.as_deref(), Some("private_responses"));
        assert_eq!(selection.reasoning.as_deref(), Some("low"));
    }

    #[test]
    fn a_provider_without_a_model_gets_its_default_model() {
        let selection = choices()
            .select(&Flags::parse("--provider private_responses go"), None)
            .unwrap();

        assert_eq!(selection.model.as_deref(), Some("example-model"));
        assert_eq!(selection.reasoning.as_deref(), Some("high"));
    }

    #[test]
    fn an_ambiguous_or_mismatched_catalog_model_is_rejected() {
        let choices = choices();
        for text in ["--model shared go", "--claude --model example go"] {
            assert!(choices.select(&Flags::parse(text), None).is_err(), "{text}");
        }
    }

    #[test]
    fn a_rejected_request_keeps_its_persona_and_effort_but_not_its_harness() {
        let Rejected { fallback } = choices()
            .select(
                &Flags::parse("--claude --model example --persona ops -rsn high go"),
                Some(CLAUDE_CHANNEL),
            )
            .unwrap_err();

        assert_eq!(fallback.persona_id.as_deref(), Some("ops"));
        assert_eq!(fallback.reasoning.as_deref(), Some("high"));
        assert!(!fallback.restart);
        assert!(!fallback.explicit);
        // The fallback is the selection of the channel without the flags.
        assert_eq!(fallback.harness, HarnessType::ClaudeCode);
    }

    #[test]
    fn a_model_outside_the_catalog_keeps_the_session_harness() {
        let selection = choices()
            .select(&Flags::parse("--model gpt-6 go"), None)
            .unwrap();

        assert!(!selection.restart);
        assert_eq!(selection.model.as_deref(), Some("gpt-6"));
        assert_eq!(selection.provider, None);
    }

    #[test]
    fn a_disabled_harness_flag_is_refused_with_its_model() {
        let choices = Choices::new(
            HarnessType::Codex,
            only(&[HarnessType::Codex]),
            ModelCatalog::default(),
            ChannelDefaults::default(),
        )
        .unwrap();

        let selection = choices.select(&Flags::parse("--opus go"), None).unwrap();

        assert_eq!(selection.refused_harness, Some(HarnessType::ClaudeCode));
        assert_eq!(selection.harness, HarnessType::Codex);
        assert!(!selection.restart);
        assert_eq!(selection.model, None);
    }
}

mod channel_defaults {
    use super::*;

    #[test]
    fn a_default_applies_without_a_restart() {
        let selection = choices()
            .select(&Flags::parse("hi"), Some(CLAUDE_CHANNEL))
            .unwrap();

        assert_eq!(selection.harness, HarnessType::ClaudeCode);
        assert!(!selection.restart);
        assert_eq!(selection.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(selection.reasoning.as_deref(), Some("low"));
        assert_eq!(selection.persona_id.as_deref(), Some("eng"));
    }

    #[test]
    fn a_flag_replaces_the_whole_default_selection_but_not_the_persona() {
        let selection = choices()
            .select(&Flags::parse("--codex -rsn high go"), Some(CLAUDE_CHANNEL))
            .unwrap();

        assert_eq!(selection.harness, HarnessType::Codex);
        assert!(selection.restart);
        assert_eq!(selection.model, None);
        assert_eq!(selection.reasoning.as_deref(), Some("high"));
        assert_eq!(selection.persona_id.as_deref(), Some("eng"));
    }

    #[test]
    fn an_unresolvable_default_selects_nothing() {
        let selection = choices()
            .select(&Flags::parse("hi"), Some(AMBIGUOUS_CHANNEL))
            .unwrap();

        assert_eq!(selection.harness, HarnessType::Codex);
        assert_eq!(selection.model, None);
    }

    #[test]
    fn a_default_model_leaves_a_session_on_another_harness() {
        let mut selection = choices()
            .select(&Flags::parse("hi"), Some(CLAUDE_CHANNEL))
            .unwrap();

        selection.fit(HarnessType::Codex);

        assert_eq!(selection.model, None);
        assert_eq!(selection.provider, None);
        assert_eq!(selection.reasoning.as_deref(), Some("low"));
    }

    #[test]
    fn a_default_model_stays_on_its_own_harness_and_a_flag_model_stays_anywhere() {
        let mut defaulted = choices()
            .select(&Flags::parse("hi"), Some(CLAUDE_CHANNEL))
            .unwrap();
        defaulted.fit(HarnessType::ClaudeCode);
        assert_eq!(defaulted.model.as_deref(), Some("claude-opus-5-5"));

        let mut flagged = choices()
            .select(&Flags::parse("--model gpt-6 go"), Some(CLAUDE_CHANNEL))
            .unwrap();
        flagged.fit(HarnessType::Codex);
        assert_eq!(flagged.model.as_deref(), Some("gpt-6"));
    }
}
