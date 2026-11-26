use std::{collections::HashMap, time::Duration};

use axum::http::StatusCode;
use centaur_session_core::{
    ThreadKey,
    destination::{ZulipDestination, ZulipDirect, ZulipTopic},
};
use centaur_zulip::{ChannelId, ErrorCode, Recipient, UserId, ZulipError};

use super::{
    ConfigError, client_from, file_message, markdown_link_text, recipient, zulip_destination,
    zulip_failure,
};
use crate::ApiError;

fn keys(keys: &[&str]) -> Vec<ThreadKey> {
    keys.iter()
        .map(|key| ThreadKey::parse(*key).unwrap())
        .collect()
}

fn api_error(status: StatusCode, code: ErrorCode) -> ZulipError {
    ZulipError::Api {
        status,
        code,
        message: "Message is too long".to_owned(),
        retry_after: None,
    }
}

mod destination {
    use super::*;

    #[test]
    fn the_one_zulip_conversation_of_the_sandbox_is_found() {
        let destination =
            zulip_destination(&keys(&["slack:C123:1.2", "zulip:channel:42:deploys"])).unwrap();
        assert_eq!(
            destination,
            ZulipDestination::Topic(ZulipTopic::new(42, "deploys"))
        );
    }

    #[test]
    fn a_sandbox_without_a_zulip_conversation_is_refused() {
        for thread_keys in [keys(&[]), keys(&["slack:C123:1.2", "zulip:dm:9,7"])] {
            assert!(
                matches!(zulip_destination(&thread_keys), Err(ApiError::Forbidden(_))),
                "{thread_keys:?}"
            );
        }
    }

    #[test]
    fn a_sandbox_with_two_zulip_conversations_is_refused() {
        let thread_keys = keys(&["zulip:channel:42:deploys", "zulip:dm:7"]);
        assert!(matches!(
            zulip_destination(&thread_keys),
            Err(ApiError::Forbidden(_))
        ));
    }

    #[test]
    fn a_topic_and_a_direct_conversation_become_recipients() {
        assert_eq!(
            recipient(&ZulipDestination::Topic(ZulipTopic::new(42, "Deploys"))),
            Recipient::Topic {
                channel_id: ChannelId(42),
                topic: "deploys".to_owned(),
            }
        );
        assert_eq!(
            recipient(&ZulipDestination::Direct(ZulipDirect::new([9, 7]).unwrap())),
            Recipient::Direct {
                user_ids: vec![UserId(7), UserId(9)],
            }
        );
    }
}

mod file_messages {
    use super::*;

    #[test]
    fn link_text_escapes_brackets_and_backslashes() {
        assert_eq!(
            markdown_link_text("chart [v2]\\a.png"),
            "chart \\[v2\\]\\\\a.png"
        );
        assert_eq!(markdown_link_text("report.pdf"), "report.pdf");
    }

    #[test]
    fn a_comment_goes_above_the_link() {
        assert_eq!(
            file_message("a.png", "/user_uploads/2/a/b/a.png", Some(" The chart. ")),
            "The chart.\n\n[a.png](/user_uploads/2/a/b/a.png)"
        );
    }

    #[test]
    fn a_blank_or_missing_comment_leaves_only_the_link() {
        for comment in [None, Some(""), Some("  ")] {
            assert_eq!(
                file_message("a.png", "/user_uploads/2/a/b/a.png", comment),
                "[a.png](/user_uploads/2/a/b/a.png)"
            );
        }
    }
}

mod config {
    use super::*;

    const COMPLETE: [(&str, &str); 3] = [
        ("ZULIP_SITE", "https://chat.example.test"),
        ("ZULIP_EMAIL", "bot@example.test"),
        ("ZULIP_API_KEY", "key"),
    ];

    fn env(values: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let values: HashMap<_, _> = values.iter().copied().collect();
        move |name| values.get(name).map(|value| (*value).to_owned())
    }

    #[test]
    fn a_complete_environment_builds_a_client() {
        client_from(env(&COMPLETE)).unwrap();
    }

    #[test]
    fn each_missing_setting_is_named() {
        for missing in ["ZULIP_SITE", "ZULIP_EMAIL", "ZULIP_API_KEY"] {
            let values: Vec<_> = COMPLETE
                .into_iter()
                .filter(|(name, _)| *name != missing)
                .collect();
            let error = client_from(env(&values)).unwrap_err();
            assert!(
                matches!(error, ConfigError::Missing(name) if name == missing),
                "{error}"
            );
        }
    }

    #[test]
    fn a_site_that_is_not_a_url_is_rejected() {
        let mut values = COMPLETE;
        values[0].1 = "chat.example.test";
        let error = client_from(env(&values)).unwrap_err();
        assert!(matches!(error, ConfigError::InvalidSite(_)), "{error}");
    }
}

mod failures {
    use super::*;

    #[test]
    fn a_rejected_request_carries_the_zulip_reason() {
        let error = zulip_failure(api_error(StatusCode::BAD_REQUEST, ErrorCode::BadRequest));
        let ApiError::Unprocessable(message) = &error else {
            panic!("{error}");
        };
        assert!(message.contains("Message is too long"), "{message}");
        let error = zulip_failure(ZulipError::UnexpectedStatus {
            status: StatusCode::NOT_FOUND,
        });
        assert!(matches!(error, ApiError::Unprocessable(_)), "{error}");
    }

    #[test]
    fn a_file_over_the_server_limit_is_too_large() {
        let error = zulip_failure(ZulipError::UnexpectedStatus {
            status: StatusCode::PAYLOAD_TOO_LARGE,
        });
        assert!(matches!(error, ApiError::PayloadTooLarge(_)), "{error}");
    }

    #[test]
    fn a_rejected_bot_credential_is_not_a_fault_of_the_request() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let error = zulip_failure(api_error(status, ErrorCode::Other));
            assert!(matches!(error, ApiError::ServiceUnavailable(_)), "{error}");
        }
    }

    #[test]
    fn a_rate_limit_names_the_wait() {
        let error = zulip_failure(ZulipError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: ErrorCode::RateLimitHit,
            message: "API usage exceeded rate limit".to_owned(),
            retry_after: Some(Duration::from_millis(2500)),
        });
        let ApiError::ServiceUnavailable(message) = &error else {
            panic!("{error}");
        };
        assert!(message.contains("Retry after 2 seconds"), "{message}");
    }

    #[test]
    fn a_server_error_is_unavailable() {
        let error = zulip_failure(ZulipError::UnexpectedStatus {
            status: StatusCode::BAD_GATEWAY,
        });
        assert!(matches!(error, ApiError::ServiceUnavailable(_)), "{error}");
    }
}
