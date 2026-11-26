use centaur_session_core::HarnessType;

use super::{HarnessDefaults, SandboxDefaults, line, model_display_name};
use crate::{
    flags::Flags,
    selection::{ModelCatalog, Selection},
    sessions::OpenedSession,
    testing::choices,
};

fn selection(model: Option<&str>, provider: Option<&str>, reasoning: Option<&str>) -> Selection {
    let mut selection = choices().select(&Flags::default(), None).unwrap();
    selection.model = model.map(str::to_owned);
    selection.provider = provider.map(str::to_owned);
    selection.reasoning = reasoning.map(str::to_owned);
    selection
}

fn session(harness: HarnessType) -> OpenedSession {
    OpenedSession {
        harness,
        harness_switched: false,
        unavailable_persona_id: None,
        persona_model: None,
    }
}

#[test]
fn model_ids_become_product_names() {
    for (id, name) in [
        ("claude-opus-5-5", "Opus 5.5"),
        ("claude-haiku-4-5-20251001", "Haiku 4.5"),
        ("claude-opus-5-fast", "Opus 5 Fast"),
        ("gpt-5.6-sol", "Sol 5.6"),
        ("gpt-5.4-pro", "GPT 5.4 Pro"),
        ("gpt-5.2", "GPT 5.2"),
        ("o3-mini", "O3-MINI"),
    ] {
        assert_eq!(model_display_name(id), name, "{id}");
    }
}

#[test]
fn the_baked_defaults_come_from_the_harness_config_files() {
    let defaults = HarnessDefaults::new(SandboxDefaults::default());

    assert_eq!(
        line(
            &selection(None, None, None),
            &session(HarnessType::Codex),
            &defaults,
            &ModelCatalog::default()
        ),
        Some("*Sol 5.6 · Codex · Medium*".to_owned())
    );
    assert_eq!(
        line(
            &selection(None, None, None),
            &session(HarnessType::ClaudeCode),
            &defaults,
            &ModelCatalog::default()
        ),
        Some("*Opus 5.5 · Claude Code*".to_owned())
    );
}

#[test]
fn deployment_overrides_replace_the_baked_defaults() {
    let defaults = HarnessDefaults::new(SandboxDefaults {
        codex_model: Some("gpt-5.4-pro".to_owned()),
        codex_reasoning: Some("xhigh".to_owned()),
        ..SandboxDefaults::default()
    });

    assert_eq!(
        line(
            &selection(None, None, None),
            &session(HarnessType::Nanocode),
            &defaults,
            &ModelCatalog::default()
        ),
        Some("*GPT 5.4 Pro · Nanocode · XHigh*".to_owned())
    );
}

#[test]
fn a_catalog_model_shows_its_names_and_a_pinned_model_wins() {
    let catalog: ModelCatalog = r#"{"private_responses": {"name": "Private Responses",
        "models": {"example-model": {"displayName": "Example Model"}}}}"#
        .parse()
        .unwrap();
    let defaults = HarnessDefaults::new(SandboxDefaults::default());
    let chosen = selection(
        Some("example-model"),
        Some("private_responses"),
        Some("high"),
    );

    assert_eq!(
        line(&chosen, &session(HarnessType::Codex), &defaults, &catalog),
        Some("*Example Model · Private Responses · Codex · High*".to_owned())
    );
    let pinned = OpenedSession {
        persona_model: Some("claude-sonnet-5-5".to_owned()),
        ..session(HarnessType::ClaudeCode)
    };
    assert_eq!(
        line(&chosen, &pinned, &defaults, &catalog),
        Some("*Sonnet 5.5 · Claude Code · High*".to_owned())
    );
}

#[test]
fn amp_shows_only_its_harness() {
    let defaults = HarnessDefaults::new(SandboxDefaults::default());

    assert_eq!(
        line(
            &selection(None, None, Some("high")),
            &session(HarnessType::Amp),
            &defaults,
            &ModelCatalog::default()
        ),
        Some("*Amp*".to_owned())
    );
}
