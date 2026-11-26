//! Durable session control-plane types.
//!
//! A session is the public control-plane object for one ongoing agent
//! conversation. `thread_key` is the canonical identifier. The [`api`] module
//! holds the HTTP request and response bodies of the session API. The
//! [`event`] module holds the types and payloads of session stream events.
//! Servers and clients share these definitions.

pub mod api;
pub mod destination;
pub mod event;
mod thread_key;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{AsRefStr, Display, EnumString};
use time::OffsetDateTime;

pub use thread_key::{MAX_THREAD_KEY_BYTES, ThreadKey, ThreadKeyError};

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    AsRefStr,
    Display,
    EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum HarnessType {
    Codex,
    Amp,
    ClaudeCode,
    /// Runs in the long-lived `centaur-brain` service instead of a sandbox.
    Nanocode,
    Hermes,
    Pi,
    Omp,
}

impl HarnessType {
    /// Returns the product name that people see, for example `Claude Code`.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Amp => "Amp",
            Self::ClaudeCode => "Claude Code",
            Self::Nanocode => "Nanocode",
            Self::Hermes => "Hermes",
            Self::Pi => "Pi",
            Self::Omp => "OMP",
        }
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Idle,
    Executing,
    Failed,
    Archived,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxRepoCacheAccess {
    None,
    Public,
    #[default]
    All,
}

impl SandboxRepoCacheAccess {
    pub const fn enabled(&self) -> bool {
        !matches!(self, Self::None)
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Public => "public",
            Self::All => "all",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "public" => Some(Self::Public),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    pub const fn from_legacy_enabled(enabled: bool) -> Self {
        if enabled { Self::All } else { Self::None }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    #[serde(default)]
    pub repo_cache: SandboxRepoCacheAccess,
    pub observability_enabled: bool,
}

impl SandboxCapabilities {
    pub const fn default_enabled() -> Self {
        Self {
            repo_cache: SandboxRepoCacheAccess::All,
            observability_enabled: true,
        }
    }

    pub const fn is_default_enabled(&self) -> bool {
        matches!(self.repo_cache, SandboxRepoCacheAccess::All) && self.observability_enabled
    }

    pub const fn repo_cache_enabled(&self) -> bool {
        self.repo_cache.enabled()
    }
}

impl Default for SandboxCapabilities {
    fn default() -> Self {
        Self::default_enabled()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub thread_key: ThreadKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub sandbox_id: Option<String>,
    /// Capabilities applied to the currently assigned sandbox. `None` means the
    /// sandbox predates capability tracking; callers may treat it as compatible
    /// only with the default-enabled profile.
    #[serde(default)]
    pub sandbox_capabilities: Option<SandboxCapabilities>,
    pub harness_type: HarnessType,
    pub harness_thread_id: Option<String>,
    pub persona_id: Option<String>,
    pub status: SessionStatus,
    /// iron-control principal OID this session's egress proxy binds to,
    /// captured at registration so a resumed session can recreate its sandbox.
    pub iron_control_principal: Option<String>,
    /// Per-proxy labels captured at session creation and applied whenever this
    /// session's egress proxy is created, repaired, or rebound.
    #[serde(default)]
    pub proxy_labels: BTreeMap<String, String>,
    /// Last meaningful activity for the currently assigned sandbox. This is
    /// the eviction signal for capacity pressure and intentionally separate
    /// from `updated_at`, which also changes for metadata/status writes.
    pub sandbox_last_active_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, AsRefStr, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMessageInput {
    #[serde(default)]
    pub client_message_id: Option<String>,
    pub role: MessageRole,
    pub parts: Vec<Value>,
    #[serde(default = "empty_object")]
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMessage {
    pub message_id: String,
    pub client_message_id: Option<String>,
    pub thread_key: ThreadKey,
    pub role: MessageRole,
    pub parts: Vec<Value>,
    pub metadata: Value,
    pub created_at: OffsetDateTime,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ExecutionStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionExecution {
    pub execution_id: String,
    pub idempotency_key: Option<String>,
    pub thread_key: ThreadKey,
    pub status: ExecutionStatus,
    pub metadata: Value,
    pub error: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub event_id: i64,
    pub thread_key: ThreadKey,
    pub execution_id: Option<String>,
    pub event_type: String,
    pub payload: Value,
    pub created_at: OffsetDateTime,
}

pub fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod harness_type_tests {
    use std::str::FromStr;

    use super::HarnessType;

    #[test]
    fn accepts_supported_values() {
        assert_eq!(HarnessType::from_str("codex").unwrap(), HarnessType::Codex);
        assert_eq!(HarnessType::from_str("amp").unwrap(), HarnessType::Amp);
        assert_eq!(
            HarnessType::from_str("nanocode").unwrap(),
            HarnessType::Nanocode
        );
        assert_eq!(
            HarnessType::from_str("claudecode").unwrap(),
            HarnessType::ClaudeCode
        );
        assert_eq!(HarnessType::from_str("pi").unwrap(), HarnessType::Pi);
    }

    #[test]
    fn serializes_as_wire_value() {
        assert_eq!(
            serde_json::to_value(HarnessType::ClaudeCode).unwrap(),
            serde_json::json!("claudecode")
        );
        assert_eq!(
            serde_json::from_value::<HarnessType>(serde_json::json!("codex")).unwrap(),
            HarnessType::Codex
        );
        assert_eq!(
            serde_json::to_value(HarnessType::Nanocode).unwrap(),
            serde_json::json!("nanocode")
        );
        assert_eq!(HarnessType::Nanocode.to_string(), "nanocode");
        assert_eq!(HarnessType::Nanocode.as_ref(), "nanocode");
        assert_eq!(
            serde_json::from_value::<HarnessType>(serde_json::json!("nanocode")).unwrap(),
            HarnessType::Nanocode
        );
    }

    #[test]
    fn rejects_unsupported_values() {
        assert!(HarnessType::from_str("claude-code").is_err());
        assert!(HarnessType::from_str("nanocodex").is_err());
    }
}
