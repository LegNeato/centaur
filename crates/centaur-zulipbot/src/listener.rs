//! Reads messages from a Zulip event queue and passes them to the bot.
//!
//! The listener registers a queue and long-polls it. It receives each message
//! in the order of the queue and starts one task for each received message.
//! The listener registers a new queue when the server expires the old one. A
//! new queue holds no earlier messages. The listener therefore starts a scan for
//! unfinished messages after every registration.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{info, warn};

use crate::{bot::Bot, retry::Backoff, sessions::Sessions};
use centaur_zulip::{ErrorCode, EventId, QueueId, Registration, ZulipApi, ZulipError};

/// The backoff between failed registrations or polls. Only a permanent
/// failure ends the listener.
const RECONNECT: Backoff = Backoff {
    attempts: u32::MAX,
    initial_delay: Duration::from_secs(1),
    max_delay: Duration::from_secs(60),
};

/// The retries of a failed recovery scan. The scan resumes the answers that a
/// restart interrupted. It therefore gets more attempts than a request. A
/// later registration scans again.
const RECOVERY_RETRY: Backoff = Backoff {
    attempts: 5,
    initial_delay: Duration::from_secs(5),
    max_delay: Duration::from_secs(60),
};

/// The longest time without a successful poll that still counts as live. A
/// long poll returns at least once a minute because the server sends
/// heartbeats.
pub const LIVENESS_WINDOW: Duration = Duration::from_secs(300);

/// The time of the last successful registration or poll. The health
/// endpoint reads it.
#[derive(Debug, Default)]
pub struct Health {
    /// Unix time in seconds. Zero means that no request has succeeded yet.
    last_success: AtomicU64,
}

impl Health {
    pub fn record_success(&self, now: SystemTime) {
        self.last_success
            .store(unix_seconds(now), Ordering::Relaxed);
    }

    pub fn is_live(&self, now: SystemTime) -> bool {
        let last_success = self.last_success.load(Ordering::Relaxed);
        last_success != 0
            && unix_seconds(now).saturating_sub(last_success) <= LIVENESS_WINDOW.as_secs()
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Listener<Z, S> {
    bot: Arc<Bot<Z, S>>,
    tasks: TaskTracker,
    health: Arc<Health>,
}

/// Why a poll loop ended.
enum PollEnd {
    QueueExpired,
    Shutdown,
}

impl<Z: ZulipApi, S: Sessions> Listener<Z, S> {
    pub fn new(bot: Arc<Bot<Z, S>>, tasks: TaskTracker, health: Arc<Health>) -> Self {
        Self { bot, tasks, health }
    }

    /// Runs until `shutdown` is cancelled. It returns early with a permanent
    /// error, for example rejected credentials or an unsupported server.
    pub async fn run(&self, shutdown: &CancellationToken) -> Result<(), ZulipError> {
        loop {
            let Some(registration) = self.register(shutdown).await? else {
                return Ok(());
            };
            info!(
                event = "zulipbot_queue_registered",
                feature_level = registration.server.feature_level,
            );
            self.bot.set_server_settings(registration.server);
            self.spawn_recovery(shutdown);
            match self
                .poll(&registration.queue_id, registration.last_event_id, shutdown)
                .await?
            {
                PollEnd::QueueExpired => warn!(event = "zulipbot_queue_expired"),
                PollEnd::Shutdown => return Ok(()),
            }
        }
    }

    /// Registers a queue and retries transient failures. It returns `None`
    /// when shutdown starts.
    async fn register(
        &self,
        shutdown: &CancellationToken,
    ) -> Result<Option<Registration>, ZulipError> {
        let mut failures = 0;
        loop {
            let result = tokio::select! {
                result = self.bot.zulip().register_queue() => result,
                () = shutdown.cancelled() => return Ok(None),
            };
            match result {
                Ok(registration) => {
                    self.health.record_success(SystemTime::now());
                    return Ok(Some(registration));
                }
                Err(error) if error.is_retryable() => {
                    warn!(event = "zulipbot_register_failed", failures, %error);
                    if !self.pause(&error, failures, shutdown).await {
                        return Ok(None);
                    }
                    failures += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn poll(
        &self,
        queue_id: &QueueId,
        mut last_event_id: EventId,
        shutdown: &CancellationToken,
    ) -> Result<PollEnd, ZulipError> {
        let mut failures = 0;
        loop {
            let result = tokio::select! {
                result = self.bot.zulip().events(queue_id, last_event_id) => result,
                () = shutdown.cancelled() => return Ok(PollEnd::Shutdown),
            };
            match result {
                Ok(events) => {
                    failures = 0;
                    self.health.record_success(SystemTime::now());
                    for event in events {
                        last_event_id = last_event_id.max(event.id);
                        if let Some(received) = event
                            .into_message()
                            .and_then(|message| self.bot.receive(message))
                        {
                            let bot = self.bot.clone();
                            self.tasks.spawn(async move { bot.process(received).await });
                        }
                    }
                }
                Err(error) if error.code() == Some(ErrorCode::BadEventQueueId) => {
                    return Ok(PollEnd::QueueExpired);
                }
                Err(error) if error.is_retryable() => {
                    warn!(event = "zulipbot_poll_failed", failures, %error);
                    if !self.pause(&error, failures, shutdown).await {
                        return Ok(PollEnd::Shutdown);
                    }
                    failures += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Waits before the next attempt. It returns false when shutdown starts.
    async fn pause(&self, error: &ZulipError, failures: u32, shutdown: &CancellationToken) -> bool {
        let delay = RECONNECT
            .delay(failures)
            .max(error.retry_after().unwrap_or_default());
        tokio::select! {
            () = tokio::time::sleep(delay) => true,
            () = shutdown.cancelled() => false,
        }
    }

    fn spawn_recovery(&self, shutdown: &CancellationToken) {
        let bot = self.bot.clone();
        let tasks = self.tasks.clone();
        let shutdown = shutdown.clone();
        self.tasks.spawn(async move {
            let scan = RECOVERY_RETRY.retry("recovery_scan", || {
                bot.unanswered_messages(SystemTime::now())
            });
            let result = tokio::select! {
                result = scan => result,
                () = shutdown.cancelled() => return,
            };
            match result {
                Ok(messages) => {
                    info!(event = "zulipbot_recovery_scan", messages = messages.len());
                    for received in messages
                        .into_iter()
                        .filter_map(|message| bot.receive_recovered(message))
                    {
                        let bot = bot.clone();
                        tasks.spawn(async move { bot.process(received).await });
                    }
                }
                Err(error) => warn!(event = "zulipbot_recovery_scan_failed", %error),
            }
        });
    }
}

#[cfg(test)]
mod tests;
