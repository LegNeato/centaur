use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use centaur_session_client::ClientError;
use centaur_session_core::{
    HarnessType, ThreadKey,
    api::{OnHarnessConflict, SteeringStatus},
    event::{ExecutionCancelled, ExecutionCompleted, ExecutionEnd, ExecutionFailed},
};
use reqwest::StatusCode;

use super::{
    Bot,
    delivery::{EMPTY_ANSWER_NOTICE, LOST_EXECUTION_NOTICE},
    hand_off::{START_FAILED_NOTICE, STEERED_FLAGS_NOTICE},
    unreadable_channel_notice,
};
use crate::{
    policy::{Allowlist, Policy},
    selection::{ChannelDefaults, Choices, ModelCatalog},
    testing::{
        ADA, BOT, BOT_NAME, ENGINEERING, FakeSessions, FakeZulip, GRACE, SENT_AT, direct_message,
        fake_sessions::{default_answer, steering},
        fake_zulip::SERVER,
        reacted, settings, topic_message,
    },
};
use centaur_zulip::{Conversation, ErrorCode, Message, ServerSettings, User, ZulipError};

type TestBot = Bot<FakeZulip, FakeSessions>;

fn policy() -> Policy {
    Policy {
        channels: Allowlist::Only(BTreeSet::from([ENGINEERING])),
        direct_message_senders: Allowlist::Only(BTreeSet::from([ADA])),
        trigger_bots: Allowlist::default(),
        trigger_guests: Allowlist::default(),
    }
}

fn bot(zulip: FakeZulip, sessions: FakeSessions) -> TestBot {
    let bot = Bot::new(
        zulip,
        sessions,
        User {
            user_id: BOT,
            full_name: BOT_NAME.to_owned(),
            is_bot: true,
            is_guest: false,
            profile: Default::default(),
        },
        policy(),
        settings(),
    );
    bot.set_server_settings(SERVER);
    bot
}

fn mention(id: u64) -> Message {
    topic_message(id, ADA, "@**Centaur** why did the deploy fail?", true)
}

fn completed(execution_id: &str, result_text: Option<&str>) -> ExecutionEnd {
    ExecutionEnd::Completed(ExecutionCompleted {
        execution_id: execution_id.to_owned(),
        thread_key: ThreadKey::parse("zulip:channel:42:deploys").unwrap(),
        completion_reason: "turn_completed".to_owned(),
        result_text: result_text.map(str::to_owned),
    })
}

mod admission;
mod answering;
mod channels;
mod defaults_and_footer;
mod failures;
mod files;
mod ignoring;
mod ordering;
mod progress;
mod recovery;
mod requester;
mod selection;
mod senders;
mod steering;
mod stopping;
