use serde_json::json;

use super::{
    ChatDestination, DiscordDestination, GithubDestination, GithubThreadKind, LinearDestination,
    SlackDestination, ZulipDestination, ZulipDirect, ZulipTopic,
};
use crate::ThreadKey;

fn destination(key: &str) -> Option<ChatDestination> {
    ThreadKey::parse(key).unwrap().chat_destination()
}

fn slack(channel_id: &str, thread_ts: &str) -> ChatDestination {
    ChatDestination::Slack(SlackDestination {
        channel_id: channel_id.to_owned(),
        thread_ts: thread_ts.to_owned(),
    })
}

#[test]
fn keys_without_a_platform_destination_resolve_to_none() {
    // A Discord key needs a channel ID.
    assert_eq!(destination("discord:111"), None);
    // A Linear key needs an issue ID and a known shape.
    assert_eq!(destination("linear::c:CMT"), None);
    assert_eq!(destination("linear:ISSUE:x:Y"), None);
    // A GitHub key needs a numeric thread number and an owner/repo pair.
    // githubbot's synthetic review sessions stay unaddressable, as in its own
    // parser.
    assert_eq!(destination("github:0xSplits/centaur:abc"), None);
    assert_eq!(destination("github:no-repo-part:704"), None);
    assert_eq!(destination("github-review:0xSplits/centaur:704"), None);
    assert_eq!(destination("api:abc123"), None);
}

mod slack {
    use super::{destination, slack};

    #[test]
    fn resolves_channel_team_and_bare_keys() {
        assert_eq!(
            destination("slack:C123:123.456"),
            Some(slack("C123", "123.456"))
        );
        // The team ID shifts the channel and timestamp one segment right.
        assert_eq!(
            destination("slack:T999:C123:123.456"),
            Some(slack("C123", "123.456"))
        );
        // A bare conversation ID with a C, D, or G prefix also resolves.
        assert_eq!(destination("D42:123.456"), Some(slack("D42", "123.456")));
        assert_eq!(destination("X42:123.456"), None);
    }

    #[test]
    fn context_line_picks_the_tool_policy_for_dms_and_channels() {
        let channel = slack("C123", "123.456").context_line();
        assert!(channel.contains("Slack channel"));
        assert!(channel.contains("C123"));
        assert!(channel.contains("use proxied methods"));

        let dm = slack("D123", "123.456").context_line();
        assert!(dm.contains("Slack DM"));
        assert!(dm.contains("D123"));
        assert!(dm.contains("use direct (`*-direct`) methods"));
    }
}

mod discord {
    use super::{ChatDestination, DiscordDestination, destination};

    #[test]
    fn resolves_keys_with_and_without_a_thread() {
        assert_eq!(
            destination("discord:111:222:333"),
            Some(ChatDestination::Discord(DiscordDestination {
                guild_id: "111".to_owned(),
                channel_id: "222".to_owned(),
                thread_id: Some("333".to_owned()),
            }))
        );
        assert_eq!(
            destination("discord:111:222"),
            Some(ChatDestination::Discord(DiscordDestination {
                guild_id: "111".to_owned(),
                channel_id: "222".to_owned(),
                thread_id: None,
            }))
        );
    }

    #[test]
    fn context_line_names_the_channel_and_upload_connector() {
        let line = destination("discord:111:222:333").unwrap().context_line();
        assert!(line.contains("Discord"));
        assert!(line.contains("222"));
        assert!(line.contains("discord.upload_file"));
    }
}

mod linear {
    use super::{ChatDestination, LinearDestination, destination};

    fn linear(comment_id: Option<&str>, agent_session_id: Option<&str>) -> ChatDestination {
        ChatDestination::Linear(LinearDestination {
            issue_id: "ISSUE".to_owned(),
            comment_id: comment_id.map(str::to_owned),
            agent_session_id: agent_session_id.map(str::to_owned),
        })
    }

    #[test]
    fn resolves_issue_comment_and_agent_session_keys() {
        assert_eq!(
            destination("linear:ISSUE:c:CMT:s:SESS"),
            Some(linear(Some("CMT"), Some("SESS")))
        );
        assert_eq!(
            destination("linear:ISSUE:s:SESS"),
            Some(linear(None, Some("SESS")))
        );
        assert_eq!(
            destination("linear:ISSUE:c:CMT"),
            Some(linear(Some("CMT"), None))
        );
        assert_eq!(destination("linear:ISSUE"), Some(linear(None, None)));
    }

    #[test]
    fn context_line_names_the_comment_and_promises_no_upload() {
        let line = linear(Some("CMT"), None).context_line();
        assert!(line.contains("Linear"));
        assert!(line.contains("ISSUE"));
        assert!(line.contains("comment CMT"));
        assert!(line.contains("no file-upload surface"));
    }
}

mod github {
    use super::{ChatDestination, GithubDestination, GithubThreadKind, destination};

    fn github(
        number: u64,
        kind: GithubThreadKind,
        review_comment_id: Option<u64>,
    ) -> ChatDestination {
        ChatDestination::Github(GithubDestination {
            owner: "0xSplits".to_owned(),
            repo: "centaur".to_owned(),
            number,
            kind,
            review_comment_id,
        })
    }

    #[test]
    fn resolves_pull_request_issue_and_review_comment_keys() {
        assert_eq!(
            destination("github:0xSplits/centaur:704"),
            Some(github(704, GithubThreadKind::Pr, None))
        );
        assert_eq!(
            destination("github:0xSplits/centaur:issue:12"),
            Some(github(12, GithubThreadKind::Issue, None))
        );
        assert_eq!(
            destination("github:0xSplits/centaur:704:rc:99"),
            Some(github(704, GithubThreadKind::Pr, Some(99)))
        );
        assert_eq!(destination("github:0xSplits/centaur:704:rc:x"), None);
    }

    #[test]
    fn context_line_names_the_subject_and_promises_no_upload() {
        let review = github(704, GithubThreadKind::Pr, Some(99)).context_line();
        assert!(review.contains("pull request 0xSplits/centaur#704"));
        assert!(review.contains("review comment 99"));
        assert!(review.contains("no file-upload surface"));

        let issue = github(12, GithubThreadKind::Issue, None).context_line();
        assert!(issue.contains("issue 0xSplits/centaur#12"));
    }
}

mod zulip {
    use super::{ChatDestination, ZulipDestination, ZulipDirect, ZulipTopic, destination, json};

    fn topic(channel_id: u64, topic: &str) -> ZulipDestination {
        ZulipDestination::Topic(ZulipTopic::new(channel_id, topic))
    }

    fn direct(user_ids: &[u64]) -> ZulipDestination {
        ZulipDestination::Direct(ZulipDirect::new(user_ids.iter().copied()).unwrap())
    }

    #[test]
    fn topic_key_round_trips_and_keeps_colons_in_the_topic() {
        let destination_in = topic(42, "deploy: api");
        let key = destination_in.thread_key().unwrap();
        assert_eq!(key.as_str(), "zulip:channel:42:deploy: api");
        assert_eq!(
            key.chat_destination(),
            Some(ChatDestination::Zulip(destination_in))
        );
    }

    #[test]
    fn topic_spellings_that_differ_only_in_case_share_a_key() {
        assert_eq!(
            topic(42, "Deploys").thread_key().unwrap(),
            topic(42, "deploys").thread_key().unwrap()
        );
        assert_eq!(topic(42, "Deploys"), topic(42, "DEPLOYS"));
    }

    #[test]
    fn empty_topic_is_the_general_chat() {
        let key = topic(42, "").thread_key().unwrap();
        assert_eq!(key.as_str(), "zulip:channel:42:");
        assert_eq!(
            destination("zulip:channel:42:"),
            Some(ChatDestination::Zulip(topic(42, "")))
        );
    }

    #[test]
    fn direct_key_lists_participants_once_in_ascending_order() {
        let key = direct(&[9, 7, 9]).thread_key().unwrap();
        assert_eq!(key.as_str(), "zulip:dm:7,9");
        assert_eq!(
            key.chat_destination(),
            Some(ChatDestination::Zulip(direct(&[7, 9])))
        );
    }

    #[test]
    fn another_spelling_of_a_key_names_no_conversation() {
        for key in [
            "zulip:dm:9,7",
            "zulip:dm:7,7",
            "zulip:dm:07",
            "zulip:dm:+7",
            "zulip:channel:+42:deploys",
            "zulip:channel:042:deploys",
            "zulip:channel:42:Deploys",
        ] {
            assert_eq!(destination(key), None, "{key}");
        }
    }

    #[test]
    fn direct_conversation_needs_a_participant() {
        assert_eq!(ZulipDirect::new([]), None);
        assert_eq!(destination("zulip:dm:"), None);
    }

    #[test]
    fn malformed_keys_have_no_destination() {
        assert_eq!(destination("zulip:channel:abc:topic"), None);
        assert_eq!(destination("zulip:channel:42"), None);
        assert_eq!(destination("zulip:dm:7,x"), None);
        assert_eq!(destination("zulip:stream:42:topic"), None);
    }

    #[test]
    fn context_line_names_the_conversation() {
        let line = ChatDestination::Zulip(topic(42, "Deploys")).context_line();
        assert!(line.contains("Zulip · channel 42 · topic \"deploys\""));
        assert!(line.contains("`zulip.upload_file`"));

        let line = ChatDestination::Zulip(direct(&[9, 7])).context_line();
        assert!(line.contains("direct message with users 7, 9"));
    }

    #[test]
    fn serializes_the_conversation_kind() {
        assert_eq!(
            serde_json::to_value(ChatDestination::Zulip(topic(42, "Deploys"))).unwrap(),
            json!({
                "platform": "zulip",
                "zulip": {"kind": "topic", "channel_id": 42, "topic": "deploys"},
            })
        );
        assert_eq!(
            serde_json::to_value(ChatDestination::Zulip(direct(&[9, 7]))).unwrap(),
            json!({"platform": "zulip", "zulip": {"kind": "direct", "user_ids": [7, 9]}})
        );
    }
}

mod serialization {
    use super::{destination, json, slack};

    #[test]
    fn nests_fields_under_the_platform_name() {
        assert_eq!(
            serde_json::to_value(slack("C123", "123.456")).unwrap(),
            json!({
                "platform": "slack",
                "slack": {"channel_id": "C123", "thread_ts": "123.456"},
            })
        );
        assert_eq!(
            serde_json::to_value(destination("github:0xSplits/centaur:704:rc:99")).unwrap(),
            json!({
                "platform": "github",
                "github": {
                    "owner": "0xSplits",
                    "repo": "centaur",
                    "number": 704,
                    "kind": "pr",
                    "review_comment_id": 99,
                },
            })
        );
    }

    #[test]
    fn omits_absent_optional_fields() {
        assert_eq!(
            serde_json::to_value(destination("discord:111:222")).unwrap(),
            json!({
                "platform": "discord",
                "discord": {"guild_id": "111", "channel_id": "222"},
            })
        );
        assert_eq!(
            serde_json::to_value(destination("linear:ISSUE")).unwrap(),
            json!({"platform": "linear", "linear": {"issue_id": "ISSUE"}})
        );
    }
}
