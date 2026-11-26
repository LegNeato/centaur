//! The delivery: the wait for the end of an execution and the answer.

use std::{
    future::Future,
    pin::pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use centaur_session_core::{
    ThreadKey,
    event::{
        ExecutionCancelled, ExecutionCompleted, ExecutionEnd, ExecutionFailed, ProgressUpdate,
    },
};
use tokio::{sync::mpsc, time::MissedTickBehavior};
use tracing::{debug, error, info, warn};

use super::Bot;
use crate::{sessions::Sessions, status::Status};
use centaur_zulip::{Conversation, Message, ZulipApi};

/// The age limit of a progress update that the bot posts. A resumed delivery
/// replays every update of its execution. Only recent updates still describe
/// the work. slackbotv2 uses the same limit.
const PROGRESS_UPDATE_MAX_AGE: Duration = Duration::from_secs(120);
/// The pause before the bot waits again for an execution whose event stream
/// failed after every reconnect of the session client. api-rs can be down
/// while the execution still runs.
const WAIT_RETRY_DELAY: Duration = Duration::from_secs(30);
/// The shortest renewal period of the typing notification. A server can
/// report a period of zero. A timer cannot run with that period.
const MIN_TYPING_REFRESH: Duration = Duration::from_secs(1);

pub(super) const LOST_EXECUTION_NOTICE: &str =
    "I could not get the result of this request. Please ask again.";
pub(super) const EMPTY_ANSWER_NOTICE: &str = "I finished without a written answer.";

impl<Z: ZulipApi, S: Sessions> Bot<Z, S> {
    pub(super) async fn deliver(&self, delivery: Delivery<'_>) {
        let Delivery {
            message,
            conversation,
            thread_key,
            execution_id,
            footer,
        } = delivery;
        let (progress_sender, progress) = mpsc::unbounded_channel();
        let wait = async {
            loop {
                let progress_sender = progress_sender.clone();
                let end = self
                    .sessions
                    .wait_for_execution_end(thread_key, execution_id, move |update| {
                        // The receiver lives until the wait ends.
                        let _ = progress_sender.send(update);
                    })
                    .await;
                match end {
                    Err(error) if error.is_retryable() => {
                        warn!(
                            event = "zulipbot_execution_wait_interrupted",
                            message_id = %message.id,
                            execution_id,
                            %error,
                        );
                        tokio::time::sleep(WAIT_RETRY_DELAY).await;
                    }
                    end => return end,
                }
            }
        };
        let end = self.while_running(conversation, wait, progress).await;
        let (answer, mut status) = match end {
            Ok(ExecutionEnd::Completed(ExecutionCompleted { result_text, .. })) => {
                let text = result_text
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| EMPTY_ANSWER_NOTICE.to_owned());
                let answer = match footer {
                    Some(footer) => format!("{text}\n\n{footer}"),
                    None => text,
                };
                (Some(answer), Status::Answered)
            }
            Ok(ExecutionEnd::Failed(ExecutionFailed { error, .. })) => {
                (Some(format!("The run failed: {error}")), Status::Failed)
            }
            // Only an interrupt cancels an execution. The person who stopped
            // the run expects no answer.
            Ok(ExecutionEnd::Cancelled(ExecutionCancelled { .. })) => (None, Status::Stopped),
            Err(error) => {
                error!(
                    event = "zulipbot_execution_lost",
                    message_id = %message.id,
                    execution_id,
                    %error,
                );
                (Some(LOST_EXECUTION_NOTICE.to_owned()), Status::Failed)
            }
        };
        if let Some(answer) = answer
            && !self.post(conversation, &answer).await
        {
            status = Status::Failed;
        }
        info!(
            event = "zulipbot_execution_delivered",
            message_id = %message.id,
            execution_id,
            status = status.emoji_name(),
        );
        self.finish(message.id, status).await;
        for steered_message in self.steered.end(execution_id, status) {
            self.replace_reaction(steered_message, Status::Steered, status)
                .await;
        }
    }

    /// Runs `work` until it ends. In the meantime the bot renews the typing
    /// notification and posts each progress update from `progress`. Zulip
    /// clients hide a typing notification that is not renewed. A new wait
    /// replays the updates of its execution. The bot posts only the updates
    /// newer than the newest posted one.
    async fn while_running<T>(
        &self,
        conversation: &Conversation,
        work: impl Future<Output = T>,
        mut progress: mpsc::UnboundedReceiver<ProgressUpdate>,
    ) -> T {
        let period = self
            .server_settings()
            .typing_refresh_interval
            .max(MIN_TYPING_REFRESH);
        let mut renewal = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        // A renewal that a slow post delays does not repeat to catch up.
        renewal.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut work = pin!(work);
        let mut newest_posted = None;
        self.set_typing(conversation, true).await;
        let output = loop {
            tokio::select! {
                output = &mut work => break output,
                _ = renewal.tick() => self.set_typing(conversation, true).await,
                Some(update) = progress.recv() => {
                    self.post_progress(conversation, &update, &mut newest_posted).await;
                }
            }
        };
        // An update can arrive in the same poll as the end.
        while let Ok(update) = progress.try_recv() {
            self.post_progress(conversation, &update, &mut newest_posted)
                .await;
        }
        self.set_typing(conversation, false).await;
        output
    }

    /// Posts a progress update that is younger than
    /// [`PROGRESS_UPDATE_MAX_AGE`] and newer than `newest_posted`. A Zulip
    /// client ends the typing notification of a sender who posts. The bot
    /// therefore renews it.
    async fn post_progress(
        &self,
        conversation: &Conversation,
        update: &ProgressUpdate,
        newest_posted: &mut Option<i64>,
    ) {
        if newest_posted.is_some_and(|newest| update.created_at_ms <= newest) {
            return;
        }
        *newest_posted = Some(update.created_at_ms);
        let written = UNIX_EPOCH
            + Duration::from_millis(u64::try_from(update.created_at_ms).unwrap_or_default());
        let age = SystemTime::now()
            .duration_since(written)
            .unwrap_or_default();
        if age > PROGRESS_UPDATE_MAX_AGE {
            debug!(event = "zulipbot_progress_update_stale", execution_id = %update.execution_id);
            return;
        }
        self.post(conversation, &update.text).await;
        self.set_typing(conversation, true).await;
    }
}

/// An execution whose answer the bot posts.
#[derive(Clone, Copy, Debug)]
pub(super) struct Delivery<'a> {
    pub(super) message: &'a Message,
    pub(super) conversation: &'a Conversation,
    pub(super) thread_key: &'a ThreadKey,
    pub(super) execution_id: &'a str,
    /// The metadata line under the answer.
    pub(super) footer: Option<&'a str>,
}
