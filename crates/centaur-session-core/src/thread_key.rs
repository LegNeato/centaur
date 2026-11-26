//! The key that names a session.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The maximum length of a thread key in bytes.
pub const MAX_THREAD_KEY_BYTES: usize = 512;

/// The key of a session, for example `slack:C123:1700000000.000100`. The part
/// before the first `:` names the source. The rest identifies the
/// conversation in that source. A thread key is at most
/// [`MAX_THREAD_KEY_BYTES`] long and holds no ASCII control characters.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThreadKey(String);

impl ThreadKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, ThreadKeyError> {
        let value = value.into();
        validate_thread_key(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ThreadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThreadKey {
    type Err = ThreadKeyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for ThreadKey {
    type Error = ThreadKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ThreadKey> for String {
    fn from(key: ThreadKey) -> Self {
        key.0
    }
}

impl AsRef<str> for ThreadKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum ThreadKeyError {
    #[error("thread_key is required")]
    Empty,
    #[error("thread_key must be at most {MAX_THREAD_KEY_BYTES} bytes")]
    TooLong,
    #[error("thread_key must be namespaced as '<source>:<id>'")]
    MissingNamespace,
    #[error("thread_key must not contain ASCII control characters")]
    ControlCharacter,
    #[error("thread_key must not be raw JSON")]
    RawJson,
}

fn validate_thread_key(value: &str) -> Result<(), ThreadKeyError> {
    if value.is_empty() {
        return Err(ThreadKeyError::Empty);
    }
    if value.len() > MAX_THREAD_KEY_BYTES {
        return Err(ThreadKeyError::TooLong);
    }
    if value.starts_with('{') || value.starts_with('[') {
        return Err(ThreadKeyError::RawJson);
    }
    if value.chars().any(|ch| ch.is_ascii_control()) {
        return Err(ThreadKeyError::ControlCharacter);
    }
    let Some((namespace, rest)) = value.split_once(':') else {
        return Err(ThreadKeyError::MissingNamespace);
    };
    if namespace.is_empty() || rest.is_empty() {
        return Err(ThreadKeyError::MissingNamespace);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_THREAD_KEY_BYTES, ThreadKey, ThreadKeyError};

    #[test]
    fn accepts_namespaced_values() {
        let key = ThreadKey::parse("chat:C123:1780000000.000000").unwrap();
        assert_eq!(key.as_str(), "chat:C123:1780000000.000000");
    }

    #[test]
    fn accepts_the_longest_key() {
        let key = format!("a:{}", "x".repeat(MAX_THREAD_KEY_BYTES - 2));
        assert!(ThreadKey::parse(key).is_ok());
    }

    #[test]
    fn rejects_each_invalid_shape() {
        for (value, error) in [
            (String::new(), ThreadKeyError::Empty),
            (
                format!("a:{}", "x".repeat(MAX_THREAD_KEY_BYTES - 1)),
                ThreadKeyError::TooLong,
            ),
            (
                "not-namespaced".to_owned(),
                ThreadKeyError::MissingNamespace,
            ),
            (":x".to_owned(), ThreadKeyError::MissingNamespace),
            ("x:".to_owned(), ThreadKeyError::MissingNamespace),
            ("a:b\nc".to_owned(), ThreadKeyError::ControlCharacter),
            ("{\"thread\":\"x\"}".to_owned(), ThreadKeyError::RawJson),
            ("[1]".to_owned(), ThreadKeyError::RawJson),
        ] {
            assert_eq!(ThreadKey::parse(value.clone()), Err(error), "{value:?}");
        }
    }

    #[test]
    fn serde_uses_the_plain_string_and_validates_it() {
        let key = ThreadKey::parse("cli:abc").unwrap();
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"cli:abc\"");
        assert_eq!(
            serde_json::from_str::<ThreadKey>("\"cli:abc\"").unwrap(),
            key
        );
        assert!(serde_json::from_str::<ThreadKey>("\"no-namespace\"").is_err());
    }
}
