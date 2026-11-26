use std::collections::HashMap;

use serde::Deserialize;

use super::{ProfileFieldId, UserId};

/// A Zulip user account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct User {
    pub user_id: UserId,
    pub full_name: String,
    pub is_bot: bool,
    /// A guest sees only the channels that include the guest.
    #[serde(default)]
    pub is_guest: bool,
    /// The custom profile field values. A user lookup asks for them.
    #[serde(default, rename = "profile_data")]
    pub profile: Profile,
}

/// The custom profile field values of a user, by field ID.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Profile(HashMap<ProfileFieldId, ProfileValue>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ProfileValue {
    value: String,
}

impl Profile {
    pub fn value(&self, field: ProfileFieldId) -> Option<&str> {
        self.0.get(&field).map(|entry| entry.value.as_str())
    }
}

impl<const N: usize> From<[(ProfileFieldId, &str); N]> for Profile {
    fn from(values: [(ProfileFieldId, &str); N]) -> Self {
        Self(
            values
                .into_iter()
                .map(|(field, value)| {
                    let value = value.to_owned();
                    (field, ProfileValue { value })
                })
                .collect(),
        )
    }
}
