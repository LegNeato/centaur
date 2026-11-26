//! Bounded retries of transient failures.

use std::{future::Future, time::Duration};

use centaur_session_client::ClientError;
use tracing::warn;

use centaur_zulip::ZulipError;

/// An error that reports whether the same request can succeed later.
pub trait Retryable {
    fn is_retryable(&self) -> bool;

    /// The wait that the server requested. It is `None` when the server did
    /// not request one.
    fn retry_after(&self) -> Option<Duration> {
        None
    }
}

impl Retryable for ZulipError {
    fn is_retryable(&self) -> bool {
        ZulipError::is_retryable(self)
    }

    fn retry_after(&self) -> Option<Duration> {
        ZulipError::retry_after(self)
    }
}

impl Retryable for ClientError {
    fn is_retryable(&self) -> bool {
        ClientError::is_retryable(self)
    }
}

/// Exponential backoff between attempts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backoff {
    /// The maximum number of attempts, including the first attempt.
    pub attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Backoff {
    /// The policy for a request that a user waits on. It makes three attempts
    /// within a few seconds.
    pub const REQUEST: Self = Self {
        attempts: 3,
        initial_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(4),
    };

    /// Returns the delay before retry number `retry`. The first retry has the
    /// number zero.
    pub fn delay(self, retry: u32) -> Duration {
        self.initial_delay
            .saturating_mul(2_u32.saturating_pow(retry))
            .min(self.max_delay)
    }

    /// Runs `call` until it succeeds, returns a permanent error, or uses all
    /// attempts. A wait that the server requests replaces a shorter backoff
    /// delay.
    pub async fn retry<T, E, F, Fut>(self, operation: &'static str, call: F) -> Result<T, E>
    where
        E: Retryable + std::fmt::Display,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.retry_when(operation, E::is_retryable, call).await
    }

    /// Runs `call` like [`Backoff::retry`], but repeats only an error that
    /// `repeatable` accepts.
    pub async fn retry_when<T, E, F, Fut>(
        self,
        operation: &'static str,
        repeatable: impl Fn(&E) -> bool,
        mut call: F,
    ) -> Result<T, E>
    where
        E: Retryable + std::fmt::Display,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut retry = 0;
        loop {
            match call().await {
                Ok(value) => return Ok(value),
                Err(error) if repeatable(&error) && retry + 1 < self.attempts => {
                    let delay = self
                        .delay(retry)
                        .max(error.retry_after().unwrap_or_default());
                    warn!(
                        event = "zulipbot_request_retry",
                        operation,
                        retry,
                        delay_ms = delay.as_millis() as u64,
                        %error,
                    );
                    tokio::time::sleep(delay).await;
                    retry += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fmt,
        time::{Duration, Instant},
    };

    use super::{Backoff, Retryable};

    #[derive(Debug)]
    struct Failure {
        retryable: bool,
        retry_after: Option<Duration>,
    }

    impl fmt::Display for Failure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "failure")
        }
    }

    impl Retryable for Failure {
        fn is_retryable(&self) -> bool {
            self.retryable
        }

        fn retry_after(&self) -> Option<Duration> {
            self.retry_after
        }
    }

    const QUICK: Backoff = Backoff {
        attempts: 3,
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
    };

    const TRANSIENT: Failure = Failure {
        retryable: true,
        retry_after: None,
    };

    #[tokio::test]
    async fn retries_a_transient_failure_until_it_succeeds() {
        let calls = Cell::new(0);
        let result = QUICK
            .retry("test", || {
                calls.set(calls.get() + 1);
                async {
                    if calls.get() < 3 {
                        Err(TRANSIENT)
                    } else {
                        Ok("done")
                    }
                }
            })
            .await;
        assert_eq!(result.unwrap(), "done");
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn stops_after_the_last_attempt() {
        let calls = Cell::new(0);
        let result: Result<(), Failure> = QUICK
            .retry("test", || {
                calls.set(calls.get() + 1);
                async { Err(TRANSIENT) }
            })
            .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn does_not_retry_a_permanent_failure() {
        let calls = Cell::new(0);
        let result: Result<(), Failure> = QUICK
            .retry("test", || {
                calls.set(calls.get() + 1);
                async {
                    Err(Failure {
                        retryable: false,
                        retry_after: None,
                    })
                }
            })
            .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn waits_as_long_as_the_server_asks() {
        let calls = Cell::new(0);
        let started = Instant::now();
        let paused_start = tokio::time::Instant::now();
        let _ = QUICK
            .retry("test", || {
                calls.set(calls.get() + 1);
                async {
                    if calls.get() == 1 {
                        Err(Failure {
                            retryable: true,
                            retry_after: Some(Duration::from_secs(30)),
                        })
                    } else {
                        Ok(())
                    }
                }
            })
            .await;
        assert!(paused_start.elapsed() >= Duration::from_secs(30));
        // The test runs on a paused clock. The 30 seconds pass without delay.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn retry_when_repeats_only_an_accepted_error() {
        let calls = Cell::new(0);
        let result: Result<(), Failure> = QUICK
            .retry_when(
                "test",
                |failure: &Failure| failure.retry_after.is_some(),
                || {
                    calls.set(calls.get() + 1);
                    async { Err(TRANSIENT) }
                },
            )
            .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn delay_doubles_up_to_the_maximum() {
        let backoff = Backoff {
            attempts: 10,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(300),
        };
        assert_eq!(
            (0..4).map(|retry| backoff.delay(retry)).collect::<Vec<_>>(),
            [100, 200, 300, 300].map(Duration::from_millis)
        );
    }
}
