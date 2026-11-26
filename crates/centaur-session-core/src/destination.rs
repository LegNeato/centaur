//! The chat destinations that thread keys encode.
//!
//! Each chat ingress encodes the reply destination in the thread keys that it
//! mints. The parser in this module reads the destination from a key. The
//! session context route, the per-turn context line, and the structured
//! `session_context` of each turn all use this parser.

use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::ThreadKey;

/// The chat surface of a thread, parsed from its thread key.
///
/// The serialized form is `{"platform": "<platform>", "<platform>": {...}}`.
/// The session context route and the `session_context` of each turn use this
/// form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatDestination {
    Slack(SlackDestination),
    Discord(DiscordDestination),
    Linear(LinearDestination),
    Github(GithubDestination),
}

impl ChatDestination {
    /// Returns the platform name that the agent sees, for example `slack`.
    pub fn platform(&self) -> &'static str {
        match self {
            Self::Slack(_) => SlackDestination::PLATFORM,
            Self::Discord(_) => DiscordDestination::PLATFORM,
            Self::Linear(_) => LinearDestination::PLATFORM,
            Self::Github(_) => GithubDestination::PLATFORM,
        }
    }

    /// Returns a short note about the chat surface. The runtime puts the note
    /// before each user turn. The system prompt is the same for every
    /// platform. The note therefore tells the agent where its reply and its
    /// uploads go.
    pub fn context_line(&self) -> String {
        match self {
            Self::Slack(destination) => destination.context_line(),
            Self::Discord(destination) => destination.context_line(),
            Self::Linear(destination) => destination.context_line(),
            Self::Github(destination) => destination.context_line(),
        }
    }
}

impl Serialize for ChatDestination {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("platform", self.platform())?;
        match self {
            Self::Slack(destination) => map.serialize_entry(self.platform(), destination)?,
            Self::Discord(destination) => map.serialize_entry(self.platform(), destination)?,
            Self::Linear(destination) => map.serialize_entry(self.platform(), destination)?,
            Self::Github(destination) => map.serialize_entry(self.platform(), destination)?,
        }
        map.end()
    }
}

impl ThreadKey {
    /// Returns the chat surface of the thread. It returns `None` for a key
    /// that no chat platform can address. Examples are `api:` threads and the
    /// synthetic `github-review:` sessions of githubbot.
    pub fn chat_destination(&self) -> Option<ChatDestination> {
        let key = self.as_str();
        if let Some(rest) = key.strip_prefix("github:") {
            return GithubDestination::parse(rest).map(ChatDestination::Github);
        }
        if let Some(rest) = key.strip_prefix("discord:") {
            return DiscordDestination::parse(rest).map(ChatDestination::Discord);
        }
        if let Some(rest) = key.strip_prefix("linear:") {
            return LinearDestination::parse(rest).map(ChatDestination::Linear);
        }
        SlackDestination::parse(key).map(ChatDestination::Slack)
    }
}

/// A Slack thread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlackDestination {
    pub channel_id: String,
    pub thread_ts: String,
}

impl SlackDestination {
    const PLATFORM: &'static str = "slack";

    /// Parses `slack:<channel>:<ts>`, `slack:<team>:<channel>:<ts>`, and the
    /// legacy form `<channel>:<ts>`. The legacy form must start with a Slack
    /// conversation ID prefix.
    fn parse(key: &str) -> Option<Self> {
        let parts = key.split(':').collect::<Vec<_>>();
        let (channel_id, thread_ts) = match parts.as_slice() {
            ["slack", channel_id, thread_ts] => (*channel_id, *thread_ts),
            ["slack", _team_id, channel_id, thread_ts] => (*channel_id, *thread_ts),
            [channel_id, thread_ts] if is_slack_conversation_id(channel_id) => {
                (*channel_id, *thread_ts)
            }
            _ => return None,
        };
        if channel_id.is_empty() || thread_ts.is_empty() {
            return None;
        }
        Some(Self {
            channel_id: channel_id.to_owned(),
            thread_ts: thread_ts.to_owned(),
        })
    }

    fn context_line(&self) -> String {
        let Self {
            channel_id,
            thread_ts,
        } = self;
        if channel_id.starts_with('D') {
            format!(
                "[chat surface: Slack DM · conversation {channel_id} · thread {thread_ts}. \
                 Slack tool policy: use direct (`*-direct`) methods. \
                 Centaur delivers your reply automatically.]"
            )
        } else {
            format!(
                "[chat surface: Slack channel · channel {channel_id} · thread {thread_ts}. \
                 Slack tool policy: use proxied methods (without `-direct`). \
                 Centaur delivers your reply automatically.]"
            )
        }
    }
}

/// Slack conversation IDs start with `C` (channel), `D` (DM), or `G` (group).
fn is_slack_conversation_id(value: &str) -> bool {
    matches!(value.as_bytes().first(), Some(b'C' | b'D' | b'G'))
}

/// A Discord channel or a thread in a Discord channel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscordDestination {
    pub guild_id: String,
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl DiscordDestination {
    const PLATFORM: &'static str = "discord";

    /// Parses the text after the prefix of
    /// `discord:<guild>:<channel>[:<thread>]`.
    fn parse(rest: &str) -> Option<Self> {
        let mut segments = rest.split(':').map(str::trim);
        let guild_id = segments.next().filter(|s| !s.is_empty())?;
        let channel_id = segments.next().filter(|s| !s.is_empty())?;
        let thread_id = segments
            .next()
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        Some(Self {
            guild_id: guild_id.to_owned(),
            channel_id: channel_id.to_owned(),
            thread_id,
        })
    }

    fn context_line(&self) -> String {
        let Self {
            guild_id,
            channel_id,
            thread_id,
        } = self;
        let thread = thread_id
            .as_deref()
            .map(|id| format!(" · thread {id}"))
            .unwrap_or_default();
        format!(
            "[chat surface: Discord · channel {channel_id}{thread} (guild {guild_id}). \
             Centaur delivers your reply to this thread automatically — do not repost it with the discord tool. \
             Send files here with the `discord.upload_file` connector.]"
        )
    }
}

/// A Linear issue thread. The reply is a comment on the issue. A turn from a
/// comment thread gets its reply under `comment_id`. Linear has no
/// file-upload surface. Comments are Markdown.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LinearDestination {
    pub issue_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

impl LinearDestination {
    const PLATFORM: &'static str = "linear";

    /// Parses the text after the prefix of
    /// `linear:<issue>[:c:<comment>][:s:<agent_session>]`. The format matches
    /// `encodeThreadId` in linearbot.
    fn parse(rest: &str) -> Option<Self> {
        let segments = rest.split(':').collect::<Vec<_>>();
        let (issue_id, comment_id, agent_session_id) = match segments.as_slice() {
            [issue, "c", comment, "s", session] => (*issue, Some(*comment), Some(*session)),
            [issue, "s", session] => (*issue, None, Some(*session)),
            [issue, "c", comment] => (*issue, Some(*comment), None),
            [issue] => (*issue, None, None),
            _ => return None,
        };
        if issue_id.is_empty() {
            return None;
        }
        let non_empty = |id: Option<&str>| id.filter(|s| !s.is_empty()).map(ToOwned::to_owned);
        Some(Self {
            issue_id: issue_id.to_owned(),
            comment_id: non_empty(comment_id),
            agent_session_id: non_empty(agent_session_id),
        })
    }

    fn context_line(&self) -> String {
        let comment = self
            .comment_id
            .as_deref()
            .map(|id| format!(" · comment {id}"))
            .unwrap_or_default();
        format!(
            "[chat surface: Linear · issue {}{comment}. \
             Centaur posts your reply as a comment on this Linear thread automatically — do not repost it with the linear tool. \
             Linear replies are markdown comments with no file-upload surface; share artifacts inline or as a link.]",
            self.issue_id
        )
    }
}

/// A GitHub issue or pull-request thread. The reply is a comment on the issue
/// or pull request. A turn from a review-comment thread gets its reply under
/// `review_comment_id`. GitHub has no file-upload surface. Comments are
/// Markdown.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GithubDestination {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub kind: GithubThreadKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_comment_id: Option<u64>,
}

/// The kind of a GitHub thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GithubThreadKind {
    Issue,
    Pr,
}

impl GithubDestination {
    const PLATFORM: &'static str = "github";

    /// Parses the text after the prefix of
    /// `github:<owner>/<repo>:<pr>[:rc:<review_comment>]` and
    /// `github:<owner>/<repo>:issue:<issue>`. The format matches
    /// `parseGithubThreadKey` in githubbot.
    fn parse(rest: &str) -> Option<Self> {
        let (repo_path, thread) = rest.split_once(':')?;
        let (owner, repo) = repo_path.split_once('/')?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return None;
        }
        let segments = thread.split(':').collect::<Vec<_>>();
        let (kind, number, review_comment_id) = match segments.as_slice() {
            ["issue", number] => (GithubThreadKind::Issue, *number, None),
            [number] => (GithubThreadKind::Pr, *number, None),
            [number, "rc", comment] => (GithubThreadKind::Pr, *number, Some(*comment)),
            _ => return None,
        };
        Some(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            number: number.parse().ok()?,
            kind,
            review_comment_id: review_comment_id.map(str::parse).transpose().ok()?,
        })
    }

    fn context_line(&self) -> String {
        let Self {
            owner,
            repo,
            number,
            kind,
            review_comment_id,
        } = self;
        let subject = match kind {
            GithubThreadKind::Issue => "issue",
            GithubThreadKind::Pr => "pull request",
        };
        let review_comment = review_comment_id
            .map(|id| format!(" · review comment {id}"))
            .unwrap_or_default();
        format!(
            "[chat surface: GitHub · {subject} {owner}/{repo}#{number}{review_comment}. \
             Centaur posts your reply as a comment on this GitHub thread automatically — do not repost it with `gh`. \
             GitHub replies are markdown comments with no file-upload surface; share artifacts inline or as a link.]"
        )
    }
}

#[cfg(test)]
mod tests;
