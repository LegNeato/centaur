//! The models of the operator's custom Codex providers.
//!
//! The chart renders `codex.customProviders` as `CODEX_CUSTOM_PROVIDERS`.
//! slackbotv2 reads the same catalog (`services/slackbotv2/src/model-catalog.ts`).
//! A model of the catalog runs on Codex with its provider.

use std::{collections::BTreeMap, str::FromStr};

use serde::Deserialize;

/// The catalog. The text form is the JSON of `CODEX_CUSTOM_PROVIDERS`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelCatalog {
    providers: BTreeMap<String, Provider>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Provider {
    /// The display name of the provider.
    name: Option<String>,
    /// The model of a request that names only the provider.
    default_model: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, ModelEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ModelEntry {
    display_name: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    reasoning_efforts: Vec<String>,
    default_reasoning: Option<String>,
    /// Alternative effort names, each mapped to one of `reasoning_efforts`.
    #[serde(default)]
    reasoning_aliases: BTreeMap<String, String>,
}

/// A model of the catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogModel<'a> {
    pub id: &'a str,
    pub provider: &'a str,
    entry: &'a ModelEntry,
}

impl CatalogModel<'_> {
    pub fn display_name(&self) -> &str {
        self.entry.display_name.as_deref().unwrap_or(self.id)
    }

    /// Returns the effort that the model runs for `requested`. An alias maps
    /// to its effort. An unsupported or absent request gets the default
    /// effort of the model.
    pub fn reasoning(&self, requested: Option<&str>) -> Option<String> {
        let requested = requested
            .map(str::to_lowercase)
            .or_else(|| self.entry.default_reasoning.clone())?;
        let effective = self
            .entry
            .reasoning_aliases
            .get(&requested)
            .cloned()
            .unwrap_or(requested);
        if self.entry.reasoning_efforts.contains(&effective) {
            Some(effective)
        } else {
            self.entry
                .default_reasoning
                .clone()
                .filter(|effort| self.entry.reasoning_efforts.contains(effort))
        }
    }

    fn is_named(&self, name: &str) -> bool {
        let qualified = format!("{}/{}", self.provider, self.id);
        [self.id, qualified.as_str()]
            .into_iter()
            .chain(self.entry.aliases.iter().map(String::as_str))
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
    }
}

impl ModelCatalog {
    fn models(&self) -> impl Iterator<Item = CatalogModel<'_>> {
        self.providers.iter().flat_map(|(provider, config)| {
            config.models.iter().map(|(id, entry)| CatalogModel {
                id,
                provider,
                entry,
            })
        })
    }

    /// Returns true when a model of any provider has the name `name`.
    pub fn names(&self, name: &str) -> bool {
        self.models().any(|model| model.is_named(name))
    }

    /// Returns the one model with the name `name`. A `provider` limits the
    /// search to that provider. A name of models of two providers selects no
    /// model.
    pub fn find(&self, name: &str, provider: Option<&str>) -> Option<CatalogModel<'_>> {
        let mut matches = self
            .models()
            .filter(|model| provider.is_none_or(|provider| model.provider == provider))
            .filter(|model| model.is_named(name));
        let found = matches.next()?;
        matches.next().is_none().then_some(found)
    }

    pub fn default_model(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)?
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }

    /// Returns the display name of a provider of the catalog.
    pub fn provider_name(&self, provider: &str) -> Option<&str> {
        let (id, config) = self.providers.get_key_value(provider)?;
        Some(config.name.as_deref().unwrap_or(id))
    }
}

impl FromStr for ModelCatalog {
    type Err = serde_json::Error;

    /// Parses the JSON object of the providers. An empty text is an empty
    /// catalog.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(Self {
            providers: serde_json::from_str(text)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ModelCatalog;

    pub const CATALOG: &str = r#"{
        "private_responses": {
            "name": "Private Responses",
            "baseUrl": "https://inference.example.com/v1",
            "defaultModel": "example-model",
            "models": {
                "example-model": {
                    "displayName": "Example Model",
                    "aliases": ["example"],
                    "reasoningEfforts": ["low", "high"],
                    "defaultReasoning": "high",
                    "reasoningAliases": {"max": "high"}
                }
            }
        },
        "other": {"models": {"shared": {}}},
        "third": {"models": {"shared": {}}}
    }"#;

    fn catalog() -> ModelCatalog {
        CATALOG.parse().unwrap()
    }

    #[test]
    fn a_model_is_found_by_id_alias_or_qualified_name() {
        let catalog = catalog();
        for name in [
            "example-model",
            "EXAMPLE",
            "private_responses/example-model",
        ] {
            let model = catalog.find(name, None).unwrap();
            assert_eq!(model.id, "example-model", "{name}");
            assert_eq!(model.provider, "private_responses");
            assert_eq!(model.display_name(), "Example Model");
        }
    }

    #[test]
    fn a_name_of_two_providers_needs_the_provider() {
        let catalog = catalog();
        assert!(catalog.names("shared"));
        assert_eq!(catalog.find("shared", None), None);
        assert_eq!(
            catalog.find("shared", Some("other")).unwrap().provider,
            "other"
        );
        assert!(!catalog.names("gpt-6"));
    }

    #[test]
    fn reasoning_follows_the_aliases_and_the_model_default() {
        let catalog = catalog();
        let model = catalog.find("example", None).unwrap();
        assert_eq!(model.reasoning(Some("max")).as_deref(), Some("high"));
        assert_eq!(model.reasoning(Some("low")).as_deref(), Some("low"));
        assert_eq!(model.reasoning(Some("medium")).as_deref(), Some("high"));
        assert_eq!(model.reasoning(None).as_deref(), Some("high"));
    }

    #[test]
    fn providers_have_a_default_model_and_a_display_name() {
        let catalog = catalog();
        assert_eq!(
            catalog.default_model("private_responses"),
            Some("example-model")
        );
        assert_eq!(catalog.default_model("other"), None);
        assert_eq!(
            catalog.provider_name("private_responses"),
            Some("Private Responses")
        );
        assert_eq!(catalog.provider_name("other"), Some("other"));
        assert_eq!(catalog.provider_name("missing"), None);
    }

    #[test]
    fn empty_text_is_an_empty_catalog_and_bad_json_is_an_error() {
        assert_eq!("".parse::<ModelCatalog>().unwrap(), ModelCatalog::default());
        assert!("{".parse::<ModelCatalog>().is_err());
    }
}
