use centaur_session_core::HarnessType;

use super::Flags;

#[test]
fn a_request_without_flags_is_unchanged() {
    let flags = Flags::parse("@**Centaur** why did --the deploy fail?");

    assert_eq!(
        flags,
        Flags {
            text: "@**Centaur** why did --the deploy fail?".to_owned(),
            ..Flags::default()
        }
    );
}

#[test]
fn any_flag_chooses_something() {
    assert!(!Flags::parse("@**Centaur** why did --the deploy fail?").chooses_anything());
    for text in [
        "--codex hi",
        "--model gpt-5 hi",
        "--provider bedrock hi",
        "-rsn high hi",
        "--persona reviewer hi",
    ] {
        assert!(Flags::parse(text).chooses_anything(), "{text}");
    }
}

mod harness {
    use super::*;

    #[test]
    fn a_harness_flag_selects_the_harness_and_leaves_the_text() {
        let flags = Flags::parse("@**Centaur** --claude-code fix the build");

        assert_eq!(flags.harness, Some(HarnessType::ClaudeCode));
        assert_eq!(flags.text, "@**Centaur** fix the build");
    }

    #[test]
    fn a_flag_matches_without_regard_to_case() {
        let flags = Flags::parse("please --Codex fix it");

        assert_eq!(flags.harness, Some(HarnessType::Codex));
        assert_eq!(flags.text, "please fix it");
    }

    #[test]
    fn the_last_harness_flag_in_table_order_wins() {
        let flags = Flags::parse("--codex --amp go");

        assert_eq!(flags.harness, Some(HarnessType::Codex));
        assert_eq!(flags.text, "go");
    }

    #[test]
    fn a_flag_inside_a_word_or_with_a_suffix_does_not_count() {
        for content in ["use --claudex please", "x--codex", "--pickle"] {
            let flags = Flags::parse(content);
            assert_eq!(flags.harness, None, "{content}");
            assert_eq!(flags.text, content);
        }
    }
}

mod model {
    use super::*;

    #[test]
    fn a_model_value_and_an_alias_are_recognized() {
        assert_eq!(
            Flags::parse("--model gpt-6.2-sol go").model.as_deref(),
            Some("gpt-6.2-sol")
        );
        assert_eq!(
            Flags::parse("--model=Opus go").model.as_deref(),
            Some("claude-opus-5-5")
        );
    }

    #[test]
    fn a_bare_alias_selects_claude_code_unless_a_harness_flag_is_present() {
        let opus = Flags::parse("--opus review this");
        assert_eq!(opus.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(opus.harness, Some(HarnessType::ClaudeCode));
        assert_eq!(opus.text, "review this");

        let explicit = Flags::parse("--model gpt-6 --sonnet");
        assert_eq!(explicit.model.as_deref(), Some("gpt-6"));
        assert_eq!(explicit.harness, Some(HarnessType::ClaudeCode));
    }

    #[test]
    fn a_value_on_the_next_line_is_not_a_value() {
        let flags = Flags::parse("--model\nopus");

        assert_eq!(flags.model, None);
    }
}

mod provider_and_reasoning {
    use super::*;

    #[test]
    fn a_provider_selects_codex() {
        let flags = Flags::parse("--provider OpenRouter go");
        assert_eq!(flags.provider.as_deref(), Some("openrouter"));
        assert_eq!(flags.harness, Some(HarnessType::Codex));

        let bedrock = Flags::parse("--bedrock go");
        assert_eq!(bedrock.provider.as_deref(), Some("amazon-bedrock"));
        assert_eq!(bedrock.harness, Some(HarnessType::Codex));
    }

    #[test]
    fn a_known_effort_is_normalized_and_an_unknown_one_stays_in_the_text() {
        let known = Flags::parse("think -rsn hi about it");
        assert_eq!(known.reasoning, Some("high"));
        assert_eq!(known.text, "think about it");

        let unknown = Flags::parse("think -rsn loud about it");
        assert_eq!(unknown.reasoning, None);
        assert_eq!(unknown.text, "think -rsn loud about it");
    }
}

#[test]
fn a_persona_flag_names_the_persona() {
    let flags = Flags::parse("@**Centaur** --persona=reviewer\ncheck the diff");

    assert_eq!(flags.persona_id.as_deref(), Some("reviewer"));
    assert_eq!(flags.text, "@**Centaur** check the diff");
}
