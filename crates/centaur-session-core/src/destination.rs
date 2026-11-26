//! The chat destinations that thread keys encode.
//!
//! Each chat ingress encodes the reply destination in the thread keys that it
//! mints. The parser in this module reads the destination from a key. The
//! session context route, the per-turn context line, and the structured
//! `session_context` of each turn all use this parser.

use std::{collections::BTreeSet, fmt::Write as _};

use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::{ThreadKey, ThreadKeyError};

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
    Zulip(ZulipDestination),
}

impl ChatDestination {
    /// Returns the platform name that the agent sees, for example `slack`.
    pub fn platform(&self) -> &'static str {
        match self {
            Self::Slack(_) => SlackDestination::PLATFORM,
            Self::Discord(_) => DiscordDestination::PLATFORM,
            Self::Linear(_) => LinearDestination::PLATFORM,
            Self::Github(_) => GithubDestination::PLATFORM,
            Self::Zulip(_) => ZulipDestination::PLATFORM,
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
            Self::Zulip(destination) => destination.context_line(),
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
            Self::Zulip(destination) => map.serialize_entry(self.platform(), destination)?,
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
        let (namespace, rest) = key.split_once(':')?;
        match namespace {
            GithubDestination::PLATFORM => {
                GithubDestination::parse(rest).map(ChatDestination::Github)
            }
            DiscordDestination::PLATFORM => {
                DiscordDestination::parse(rest).map(ChatDestination::Discord)
            }
            LinearDestination::PLATFORM => {
                LinearDestination::parse(rest).map(ChatDestination::Linear)
            }
            ZulipDestination::PLATFORM => ZulipDestination::parse(rest)
                // A conversation has one key. Another spelling of the key,
                // for example `zulip:dm:9,7`, names no conversation.
                .filter(|destination| destination.thread_key().as_ref() == Ok(self))
                .map(ChatDestination::Zulip),
            _ => SlackDestination::parse(key).map(ChatDestination::Slack),
        }
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

/// A Zulip conversation. The reply is a message in the same topic or
/// direct-message conversation. The Zulip ingress has no file-upload
/// surface. Messages are Markdown.
///
/// The thread key of a topic is `zulip:channel:<channel_id>:<topic>`. The
/// thread key of a direct-message conversation is
/// `zulip:dm:<user_id>,<user_id>,...`. The Zulip ingress mints keys with
/// [`ZulipDestination::thread_key`]. The format therefore has one definition.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ZulipDestination {
    Topic(ZulipTopic),
    Direct(ZulipDirect),
}

/// A topic in a Zulip channel.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct ZulipTopic {
    channel_id: u64,
    topic: String,
}

impl ZulipTopic {
    /// Zulip compares topic names without regard to case. The topic is
    /// stored in lowercase. Every spelling of one topic therefore gets one
    /// session.
    pub fn new(channel_id: u64, topic: &str) -> Self {
        Self {
            channel_id,
            topic: topic.to_lowercase(),
        }
    }

    pub fn channel_id(&self) -> u64 {
        self.channel_id
    }

    /// The lowercase topic name. The empty topic name is the general chat of
    /// the channel.
    pub fn topic(&self) -> &str {
        &self.topic
    }
}

/// A Zulip direct-message conversation. Its participants other than the bot
/// identify it.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct ZulipDirect {
    user_ids: BTreeSet<u64>,
}

impl ZulipDirect {
    /// Returns `None` when `user_ids` is empty. Only the bot's conversation
    /// with itself has no other participant.
    pub fn new(user_ids: impl IntoIterator<Item = u64>) -> Option<Self> {
        let user_ids = user_ids.into_iter().collect::<BTreeSet<_>>();
        (!user_ids.is_empty()).then_some(Self { user_ids })
    }

    /// The participants other than the bot in ascending order.
    pub fn user_ids(&self) -> &BTreeSet<u64> {
        &self.user_ids
    }
}

impl ZulipDestination {
    const PLATFORM: &'static str = "zulip";

    /// Returns the thread key of the conversation. It fails only for a topic
    /// that does not fit in a thread key. The default Zulip limit of 60
    /// characters always fits.
    pub fn thread_key(&self) -> Result<ThreadKey, ThreadKeyError> {
        let mut key = format!("{}:", Self::PLATFORM);
        match self {
            Self::Topic(ZulipTopic { channel_id, topic }) => {
                // Writing to a `String` cannot fail.
                let _ = write!(key, "channel:{channel_id}:{topic}");
            }
            Self::Direct(ZulipDirect { user_ids }) => {
                key.push_str("dm:");
                for (index, user_id) in user_ids.iter().enumerate() {
                    if index > 0 {
                        key.push(',');
                    }
                    let _ = write!(key, "{user_id}");
                }
            }
        }
        ThreadKey::parse(key)
    }

    /// Parses the text after `zulip:` in a thread key. The topic is the rest
    /// of the key. A topic can therefore contain colons.
    fn parse(rest: &str) -> Option<Self> {
        if let Some(rest) = rest.strip_prefix("channel:") {
            let (channel_id, topic) = rest.split_once(':')?;
            return Some(Self::Topic(ZulipTopic::new(
                channel_id.parse().ok()?,
                topic,
            )));
        }
        let user_ids = rest.strip_prefix("dm:")?;
        let user_ids = user_ids
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<u64>, _>>()
            .ok()?;
        ZulipDirect::new(user_ids).map(Self::Direct)
    }

    fn context_line(&self) -> String {
        let conversation = match self {
            Self::Topic(ZulipTopic { channel_id, topic }) => {
                format!("channel {channel_id} · topic \"{topic}\"")
            }
            Self::Direct(ZulipDirect { user_ids }) => {
                let user_ids = user_ids
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("direct message with users {user_ids}")
            }
        };
        format!(
            "[chat surface: Zulip · {conversation}. \
             Centaur posts your reply in this conversation automatically. \
             Zulip replies are Markdown messages with no file-upload surface; share artifacts inline or as a link.]"
        )
    }
}

#[cfg(test)]
mod tests;
