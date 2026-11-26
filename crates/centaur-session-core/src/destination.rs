use crate::ThreadKey;

/// The chat surface a thread is delivered to, parsed from its thread key.
///
/// Slack, Discord, Linear, and GitHub all encode the destination — where a reply
/// (and, where the surface supports it, an uploaded file) lands — directly in the
/// key. Resolving it in one place lets the API session context, the per-turn
/// context line the agent reads, and any caller that needs a posting destination
/// share a single parser instead of each re-deriving the platform from the key
/// shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatDestination {
    Slack {
        channel_id: String,
        thread_ts: String,
    },
    Discord {
        guild_id: String,
        channel_id: String,
        thread_id: Option<String>,
    },
    /// A Linear issue thread. The reply lands as a comment on the issue (nested
    /// under `comment_id` when the turn came in on a comment thread). Unlike
    /// Slack/Discord, Linear has no file-upload surface — comments are markdown.
    Linear {
        issue_id: String,
        comment_id: Option<String>,
        agent_session_id: Option<String>,
    },
    /// A GitHub issue or pull-request thread. The reply lands as a comment on
    /// the issue/PR (pinned to `review_comment_id` when the turn came in on a
    /// PR review-comment thread). Like Linear, GitHub has no file-upload
    /// surface — comments are markdown.
    Github {
        owner: String,
        repo: String,
        number: u64,
        kind: GithubThreadKind,
        review_comment_id: Option<u64>,
    },
}

/// Whether a GitHub thread maps to an issue or a pull request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GithubThreadKind {
    Issue,
    Pr,
}

impl GithubThreadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "pr",
        }
    }
}

impl ChatDestination {
    /// The platform identifier surfaced to the agent (`slack` / `discord` /
    /// `linear` / `github`).
    pub fn platform(&self) -> &'static str {
        match self {
            Self::Slack { .. } => "slack",
            Self::Discord { .. } => "discord",
            Self::Linear { .. } => "linear",
            Self::Github { .. } => "github",
        }
    }

    /// A terse, model-visible note describing the current chat surface. It is
    /// prepended to each user turn so the agent never has to infer which platform
    /// it is on — the static system prompt is platform-neutral, so this line is
    /// the agent's authoritative signal for where its reply and uploads go.
    pub fn context_line(&self) -> String {
        match self {
            Self::Slack {
                channel_id,
                thread_ts,
            } => {
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
            Self::Discord {
                guild_id,
                channel_id,
                thread_id,
            } => {
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
            Self::Linear {
                issue_id,
                comment_id,
                ..
            } => {
                let comment = comment_id
                    .as_deref()
                    .map(|id| format!(" · comment {id}"))
                    .unwrap_or_default();
                format!(
                    "[chat surface: Linear · issue {issue_id}{comment}. \
                     Centaur posts your reply as a comment on this Linear thread automatically — do not repost it with the linear tool. \
                     Linear replies are markdown comments with no file-upload surface; share artifacts inline or as a link.]"
                )
            }
            Self::Github {
                owner,
                repo,
                number,
                kind,
                review_comment_id,
            } => {
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
    }
}

impl ThreadKey {
    /// Resolve the chat surface this thread is delivered to, when the key encodes
    /// a recognized platform destination.
    ///
    /// Returns `None` for keys that are not platform-addressable (e.g. `api:`
    /// threads, or githubbot's synthetic `github-review:` sessions). The Slack
    /// arms are kept byte-for-byte compatible with the historical
    /// session-context parser so existing Slack behavior is preserved; Discord
    /// keys are `discord:<guild>:<channel>[:<thread>]`, Linear keys are
    /// `linear:<issue>[:c:<comment>][:s:<agent_session>]` (mirroring the
    /// linearbot chat-SDK `encodeThreadId` shape), and GitHub keys are
    /// `github:<owner>/<repo>:<pr>[:rc:<review_comment>]` or
    /// `github:<owner>/<repo>:issue:<issue>` (mirroring githubbot's
    /// `parseGithubThreadKey`).
    pub fn chat_destination(&self) -> Option<ChatDestination> {
        let key = self.as_str();
        if let Some(rest) = key.strip_prefix("github:") {
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
            let number = number.parse::<u64>().ok()?;
            let review_comment_id = match review_comment_id {
                Some(comment) => Some(comment.parse::<u64>().ok()?),
                None => None,
            };
            return Some(ChatDestination::Github {
                owner: owner.to_owned(),
                repo: repo.to_owned(),
                number,
                kind,
                review_comment_id,
            });
        }
        if let Some(rest) = key.strip_prefix("discord:") {
            let mut segments = rest.split(':').map(str::trim);
            let guild_id = segments.next().filter(|s| !s.is_empty())?;
            let channel_id = segments.next().filter(|s| !s.is_empty())?;
            let thread_id = segments
                .next()
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            return Some(ChatDestination::Discord {
                guild_id: guild_id.to_owned(),
                channel_id: channel_id.to_owned(),
                thread_id,
            });
        }
        if let Some(rest) = key.strip_prefix("linear:") {
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
            return Some(ChatDestination::Linear {
                issue_id: issue_id.to_owned(),
                comment_id: comment_id.filter(|s| !s.is_empty()).map(ToOwned::to_owned),
                agent_session_id: agent_session_id
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned),
            });
        }
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
        Some(ChatDestination::Slack {
            channel_id: channel_id.to_owned(),
            thread_ts: thread_ts.to_owned(),
        })
    }
}

/// Slack conversation ids start with `C` (channel), `D` (DM), or `G` (group).
fn is_slack_conversation_id(value: &str) -> bool {
    matches!(value.as_bytes().first(), Some(b'C' | b'D' | b'G'))
}

#[cfg(test)]
mod tests {
    use super::{ChatDestination, GithubThreadKind};

    use crate::ThreadKey;

    #[test]
    fn chat_destination_resolves_slack_keys() {
        let dest = ThreadKey::parse("slack:C123:123.456")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            dest,
            ChatDestination::Slack {
                channel_id: "C123".to_owned(),
                thread_ts: "123.456".to_owned(),
            }
        );
        assert_eq!(dest.platform(), "slack");

        // The team-id variant shifts the channel/ts one segment to the right.
        let team = ThreadKey::parse("slack:T999:C123:123.456")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            team,
            ChatDestination::Slack {
                channel_id: "C123".to_owned(),
                thread_ts: "123.456".to_owned(),
            }
        );

        // A bare conversation id (C/D/G prefix) plus a timestamp also resolves.
        let bare = ThreadKey::parse("D42:123.456")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            bare,
            ChatDestination::Slack {
                channel_id: "D42".to_owned(),
                thread_ts: "123.456".to_owned(),
            }
        );
    }

    #[test]
    fn chat_destination_resolves_discord_keys() {
        let with_thread = ThreadKey::parse("discord:111:222:333")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            with_thread,
            ChatDestination::Discord {
                guild_id: "111".to_owned(),
                channel_id: "222".to_owned(),
                thread_id: Some("333".to_owned()),
            }
        );
        assert_eq!(with_thread.platform(), "discord");

        // The thread segment is optional (a channel-root message).
        let no_thread = ThreadKey::parse("discord:111:222")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            no_thread,
            ChatDestination::Discord {
                guild_id: "111".to_owned(),
                channel_id: "222".to_owned(),
                thread_id: None,
            }
        );
    }

    #[test]
    fn chat_destination_resolves_linear_keys() {
        // An agent session anchored to a comment carries both ids.
        let comment_session = ThreadKey::parse("linear:ISSUE:c:CMT:s:SESS")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            comment_session,
            ChatDestination::Linear {
                issue_id: "ISSUE".to_owned(),
                comment_id: Some("CMT".to_owned()),
                agent_session_id: Some("SESS".to_owned()),
            }
        );
        assert_eq!(comment_session.platform(), "linear");

        // An issue-level agent session has a session id but no comment.
        let issue_session = ThreadKey::parse("linear:ISSUE:s:SESS")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            issue_session,
            ChatDestination::Linear {
                issue_id: "ISSUE".to_owned(),
                comment_id: None,
                agent_session_id: Some("SESS".to_owned()),
            }
        );

        // A plain comment thread, and a bare issue, both resolve.
        let comment = ThreadKey::parse("linear:ISSUE:c:CMT")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            comment,
            ChatDestination::Linear {
                issue_id: "ISSUE".to_owned(),
                comment_id: Some("CMT".to_owned()),
                agent_session_id: None,
            }
        );
        let issue = ThreadKey::parse("linear:ISSUE")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            issue,
            ChatDestination::Linear {
                issue_id: "ISSUE".to_owned(),
                comment_id: None,
                agent_session_id: None,
            }
        );
    }

    #[test]
    fn chat_destination_resolves_github_keys() {
        // A bare number is a PR conversation thread.
        let pr = ThreadKey::parse("github:0xSplits/centaur:704")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            pr,
            ChatDestination::Github {
                owner: "0xSplits".to_owned(),
                repo: "centaur".to_owned(),
                number: 704,
                kind: GithubThreadKind::Pr,
                review_comment_id: None,
            }
        );
        assert_eq!(pr.platform(), "github");

        let issue = ThreadKey::parse("github:0xSplits/centaur:issue:12")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            issue,
            ChatDestination::Github {
                owner: "0xSplits".to_owned(),
                repo: "centaur".to_owned(),
                number: 12,
                kind: GithubThreadKind::Issue,
                review_comment_id: None,
            }
        );

        let review_comment = ThreadKey::parse("github:0xSplits/centaur:704:rc:99")
            .unwrap()
            .chat_destination()
            .unwrap();
        assert_eq!(
            review_comment,
            ChatDestination::Github {
                owner: "0xSplits".to_owned(),
                repo: "centaur".to_owned(),
                number: 704,
                kind: GithubThreadKind::Pr,
                review_comment_id: Some(99),
            }
        );
    }

    #[test]
    fn chat_destination_is_none_for_unaddressable_keys() {
        // No channel id → not a postable Discord destination.
        assert!(
            ThreadKey::parse("discord:111")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        // A Linear key with an empty issue id, or an unrecognized shape, is not
        // addressable.
        assert!(
            ThreadKey::parse("linear::c:CMT")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        assert!(
            ThreadKey::parse("linear:ISSUE:x:Y")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        // A GitHub key without a numeric thread number, or with an unrecognized
        // shape, is not addressable — and githubbot's synthetic review sessions
        // deliberately stay unaddressable, matching its own parser.
        assert!(
            ThreadKey::parse("github:0xSplits/centaur:abc")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        assert!(
            ThreadKey::parse("github:no-repo-part:704")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        assert!(
            ThreadKey::parse("github-review:0xSplits/centaur:704")
                .unwrap()
                .chat_destination()
                .is_none()
        );
        // Non-platform namespaces resolve to nothing.
        assert!(
            ThreadKey::parse("api:abc123")
                .unwrap()
                .chat_destination()
                .is_none()
        );
    }

    #[test]
    fn chat_destination_renders_a_platform_context_line() {
        let slack_channel = ThreadKey::parse("slack:C123:123.456")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(slack_channel.contains("Slack channel"));
        assert!(slack_channel.contains("C123"));
        assert!(slack_channel.contains("use proxied methods"));

        let slack_dm = ThreadKey::parse("slack:D123:123.456")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(slack_dm.contains("Slack DM"));
        assert!(slack_dm.contains("D123"));
        assert!(slack_dm.contains("use direct (`*-direct`) methods"));

        let discord = ThreadKey::parse("discord:111:222:333")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(discord.contains("Discord"));
        assert!(discord.contains("222"));
        assert!(discord.contains("discord.upload_file"));

        let linear = ThreadKey::parse("linear:ISSUE:c:CMT")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(linear.contains("Linear"));
        assert!(linear.contains("ISSUE"));
        assert!(linear.contains("comment CMT"));
        // Linear has no upload command, so the line must not promise a
        // `linear upload` analog of the Slack/Discord upload tools.
        assert!(!linear.contains("linear upload"));

        let github = ThreadKey::parse("github:0xSplits/centaur:704:rc:99")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(github.contains("GitHub"));
        assert!(github.contains("pull request 0xSplits/centaur#704"));
        assert!(github.contains("review comment 99"));
        // GitHub has no upload command either.
        assert!(!github.contains("github upload"));

        let github_issue = ThreadKey::parse("github:0xSplits/centaur:issue:12")
            .unwrap()
            .chat_destination()
            .unwrap()
            .context_line();
        assert!(github_issue.contains("issue 0xSplits/centaur#12"));
    }
}
