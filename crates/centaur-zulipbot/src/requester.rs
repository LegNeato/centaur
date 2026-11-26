//! The person who sent a request.
//!
//! The agent gets the identity of the requester and the GitHub handle from
//! the Zulip profile. It credits the requester in a GitHub pull request.
//! slackbotv2 reads the handle from a Slack profile field in the same way.

use std::{fmt::Write as _, sync::LazyLock};

use regex::Regex;

use centaur_zulip::{ProfileFieldId, User, UserId};

/// A GitHub login has letters, digits, and inner hyphens. It has at most 39
/// characters.
static LOGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$")
        .expect("the login pattern is valid")
});
/// A GitHub profile URL.
static PROFILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)github\.com/([A-Za-z0-9-]{1,39})(?:[/?#]|$)")
        .expect("the profile URL pattern is valid")
});
/// A value such as `GitHub: octocat`.
static PREFIXED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bgithub\s*[:=]\s*@?([A-Za-z0-9-]{1,39})\b")
        .expect("the prefixed login pattern is valid")
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Requester {
    pub user_id: UserId,
    pub full_name: String,
    /// The GitHub login from the Zulip profile, without `@`.
    pub github_login: Option<String>,
}

impl Requester {
    /// Reads the GitHub login from the first field of `github_fields` that
    /// holds a valid login.
    pub fn of(user: &User, github_fields: &[ProfileFieldId]) -> Self {
        Self {
            user_id: user.user_id,
            full_name: user.full_name.clone(),
            github_login: github_fields
                .iter()
                .filter_map(|field| user.profile.value(*field))
                .find_map(github_login),
        }
    }

    /// Returns the requester section of the agent input.
    pub fn context(&self) -> String {
        let mut text = format!(
            "# Requester Context\n\n\
             The Zulip user who sent this request is:\n\
             - Zulip user ID: {id}\n\
             - Zulip name: {name}\n\
             - Zulip mention: @**{name}|{id}**\n",
            id = self.user_id,
            name = self.full_name,
        );
        // Writing to a `String` cannot fail.
        let _ = match &self.github_login {
            Some(login) => write!(
                text,
                "- GitHub handle from the Zulip profile: @{login}\n\
                 - GitHub handle verified: yes\n\n\
                 ## GitHub PR Attribution\n\n\
                 - If you create a GitHub PR for this request, the PR body MUST contain this \
                 standalone line: `Prompted by: @{login}`\n\
                 - The credited prompter is the requester in this section. It is not the \
                 author of the first message of the conversation.\n\
                 - This rule is for the PR body. It is not a rule for Zulip mentions.\n\
                 - Assign the PR to the requester when possible: `{login}`\n"
            ),
            None => write!(
                text,
                "- GitHub handle from the Zulip profile: unavailable\n\
                 - GitHub handle verified: no\n\n\
                 ## GitHub PR Attribution\n\n\
                 - If you create a GitHub PR for this request, the PR body MUST contain this \
                 standalone line: `Prompted by: {name}`\n\
                 - Use the Zulip name of the requester. No verified GitHub handle is \
                 available.\n\
                 - Do not infer a GitHub username from the Zulip name or email.\n\
                 - The credited prompter is the requester in this section. It is not the \
                 author of the first message of the conversation.\n\
                 - This rule is for the PR body. It is not a rule for Zulip mentions.\n",
                name = self.full_name,
            ),
        };
        text
    }
}

/// Returns the GitHub login in a profile value. The value can be a login, a
/// login with `@`, a profile URL, or a text such as `GitHub: octocat`.
fn github_login(value: &str) -> Option<String> {
    let value = value.trim();
    let candidates = [
        PROFILE_URL
            .captures(value)
            .and_then(|captures| captures.get(1)),
        PREFIXED
            .captures(value)
            .and_then(|captures| captures.get(1)),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(|login| login.as_str())
        .chain([value.trim_start_matches('@')])
        .find(|login| LOGIN.is_match(login))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{Requester, github_login};
    use centaur_zulip::{Profile, ProfileFieldId, User, UserId};

    fn ada(profile: Profile) -> User {
        User {
            user_id: UserId(7),
            full_name: "Ada Lovelace".to_owned(),
            is_bot: false,
            is_guest: false,
            profile,
        }
    }

    #[test]
    fn a_login_url_or_prefixed_value_gives_the_login() {
        for value in [
            "octocat",
            "@octocat",
            " https://github.com/octocat/ ",
            "GitHub: octocat",
        ] {
            assert_eq!(github_login(value).as_deref(), Some("octocat"), "{value}");
        }
    }

    #[test]
    fn a_value_without_a_valid_login_gives_none() {
        for value in ["", "-octocat", "octo cat", "https://example.com/octocat"] {
            assert_eq!(github_login(value), None, "{value}");
        }
    }

    #[test]
    fn the_first_github_field_with_a_login_wins() {
        let user = ada(Profile::from([
            (ProfileFieldId(1), "not a login"),
            (ProfileFieldId(2), "https://github.com/ada-l"),
            (ProfileFieldId(3), "other"),
        ]));

        let requester = Requester::of(
            &user,
            &[ProfileFieldId(1), ProfileFieldId(2), ProfileFieldId(3)],
        );

        assert_eq!(requester.github_login.as_deref(), Some("ada-l"));
        let context = requester.context();
        assert!(context.contains("`Prompted by: @ada-l`"), "{context}");
        assert!(context.contains("Zulip mention: @**Ada Lovelace|7**"));
    }

    #[test]
    fn without_a_login_the_pr_credits_the_zulip_name() {
        let user = ada(Profile::from([(ProfileFieldId(9), "octocat")]));

        let requester = Requester::of(&user, &[ProfileFieldId(1)]);

        assert_eq!(requester.github_login, None);
        let context = requester.context();
        assert!(context.contains("`Prompted by: Ada Lovelace`"), "{context}");
        assert!(context.contains("GitHub handle verified: no"));
    }
}
