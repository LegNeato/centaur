use strum::EnumString;

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
