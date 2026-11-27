use strum::EnumString;
use thiserror::Error;

/// Where iron-proxy resolves the real value of a secret placeholder.
///
/// Each variant holds only the settings of its own source. The registry of
/// `centaur-iron-control` and the translator of `centaur-perms` turn a
/// placeholder into the matching iron-control secret source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SourcePolicy {
    /// The environment variable of the iron-proxy container that the
    /// placeholder names.
    #[default]
    Env,
    /// The vault item that the placeholder names, read with a 1Password
    /// service account.
    OnePassword(OnePasswordVault),
    /// The vault item that the placeholder names, read through 1Password
    /// Connect.
    OnePasswordConnect(OnePasswordVault),
    /// The AWS Secrets Manager secret that the prefix and the placeholder
    /// name.
    AwsSecretsManager(AwsSecretsManager),
}

/// The names of the secret sources in the deployment settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, EnumString)]
pub enum SourceKind {
    #[strum(serialize = "env")]
    Env,
    #[strum(serialize = "onepassword")]
    OnePassword,
    #[strum(serialize = "onepassword-connect")]
    OnePasswordConnect,
    #[strum(serialize = "aws-secrets-manager")]
    AwsSecretsManager,
}

/// A 1Password vault. Each item holds its value in its `credential` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnePasswordVault {
    /// The name or the ID of the vault.
    pub name: String,
    /// How long iron-proxy keeps a value before it reads the item again.
    pub ttl: String,
}

impl OnePasswordVault {
    pub fn new(name: impl Into<String>, ttl: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ttl: ttl.into(),
        }
    }
}

/// The AWS Secrets Manager settings of a deployment. The secret of a
/// placeholder is the prefix followed by the placeholder. The prefix
/// `centaur/` therefore gives `OPENAI_API_KEY` the secret
/// `centaur/OPENAI_API_KEY`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwsSecretsManager {
    region: String,
    prefix: String,
    ttl: String,
}

/// The prefix of the AWS secrets of a deployment that sets no prefix.
pub const DEFAULT_AWS_SECRET_PREFIX: &str = "centaur/";

/// The characters that AWS allows in the name of a secret, other than ASCII
/// letters and digits.
const SECRET_NAME_PUNCTUATION: &[char] = &['/', '_', '+', '=', '.', '@', '-'];

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AwsSecretsManagerError {
    #[error("the AWS region {0:?} is not a region name such as us-east-1")]
    InvalidRegion(String),
    #[error("the AWS secret prefix {0:?} has a character that AWS does not allow in a secret name")]
    InvalidPrefix(String),
}

impl AwsSecretsManager {
    pub fn new(
        region: impl Into<String>,
        prefix: impl Into<String>,
        ttl: impl Into<String>,
    ) -> Result<Self, AwsSecretsManagerError> {
        let region = region.into();
        let prefix = prefix.into();
        let is_region = !region.is_empty()
            && region
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !is_region {
            return Err(AwsSecretsManagerError::InvalidRegion(region));
        }
        if !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || SECRET_NAME_PUNCTUATION.contains(&c))
        {
            return Err(AwsSecretsManagerError::InvalidPrefix(prefix));
        }
        Ok(Self {
            region,
            prefix,
            ttl: ttl.into(),
        })
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    /// How long iron-proxy keeps a value before it reads the secret again.
    pub fn ttl(&self) -> &str {
        &self.ttl
    }

    /// Returns the name of the secret that holds the value of `placeholder`.
    pub fn secret_id(&self, placeholder: &str) -> String {
        format!("{}{placeholder}", self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_of_a_placeholder_starts_with_the_prefix() {
        let aws = AwsSecretsManager::new("us-east-1", "centaur/", "10m").unwrap();
        assert_eq!(aws.secret_id("OPENAI_API_KEY"), "centaur/OPENAI_API_KEY");
        assert_eq!(aws.region(), "us-east-1");
        assert_eq!(aws.ttl(), "10m");
    }

    #[test]
    fn an_empty_prefix_uses_the_placeholder_as_the_secret_name() {
        let aws = AwsSecretsManager::new("eu-west-2", "", "10m").unwrap();
        assert_eq!(aws.secret_id("GITHUB_TOKEN"), "GITHUB_TOKEN");
    }

    #[test]
    fn a_prefix_can_hold_every_character_that_aws_allows() {
        assert!(AwsSecretsManager::new("us-east-1", "a/b_c+d=e.f@g-h9/", "10m").is_ok());
    }

    #[test]
    fn a_prefix_with_a_character_that_aws_rejects_is_invalid() {
        for prefix in ["centaur prod/", "centaur:prod/", "céntaur/"] {
            assert_eq!(
                AwsSecretsManager::new("us-east-1", prefix, "10m"),
                Err(AwsSecretsManagerError::InvalidPrefix(prefix.to_owned())),
                "{prefix:?}"
            );
        }
    }

    #[test]
    fn a_region_must_be_a_region_name() {
        for region in ["", "US-EAST-1", "us east 1", "us-east-1/"] {
            assert_eq!(
                AwsSecretsManager::new(region, "centaur/", "10m"),
                Err(AwsSecretsManagerError::InvalidRegion(region.to_owned())),
                "{region:?}"
            );
        }
    }

    #[test]
    fn the_deployment_setting_names_the_aws_source() {
        assert_eq!(
            "aws-secrets-manager".parse::<SourceKind>(),
            Ok(SourceKind::AwsSecretsManager)
        );
    }
}
