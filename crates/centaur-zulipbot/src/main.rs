//! Zulip chat ingress for Centaur.
//!
//! The bot reads messages from a Zulip event queue. Each message addressed to
//! the bot starts a Centaur session execution or steers the running one. The
//! bot posts each final answer to the conversation. The service
//! documentation is in `services/zulipbot/README.md`.

mod bot;
mod command;
mod config;
mod files;
mod flags;
mod footer;
mod listener;
mod policy;
mod requester;
mod retry;
mod selection;
mod sessions;
mod split;
mod status;
mod steered;
mod sync;
#[cfg(test)]
mod testing;
mod turn;

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
    sync::Arc,
    time::{Duration, SystemTime},
};

use axum::{Router, extract::State, http::StatusCode, routing::get};
use centaur_session_client::{ClientError, SessionClient};
use clap::Parser;
use thiserror::Error;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    bot::Bot,
    config::Config,
    listener::{Health, Listener},
    retry::Backoff,
    selection::DisabledDefaultHarness,
};
use centaur_zulip::{ZulipApi, ZulipClient, ZulipError};

/// The time that shutdown waits for running handlers. The next process
/// resumes every answer that is still pending after this time.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
enum StartupError {
    #[error("configuration: {0}")]
    Config(#[from] DisabledDefaultHarness),
    #[error("Zulip: {0}")]
    Zulip(#[from] ZulipError),
    #[error("session API client: {0}")]
    SessionClient(#[from] ClientError),
    #[error("health server address {address}: {source}")]
    Bind {
        address: std::net::SocketAddr,
        source: io::Error,
    },
    #[error("termination signal handler: {0}")]
    Signal(#[source] io::Error),
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // Color codes would clutter log files and container logs.
        .with_ansi(io::stdout().is_terminal())
        .init();
    match run(Config::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(event = "zulipbot_exit", %error);
            ExitCode::FAILURE
        }
    }
}

async fn run(config: Config) -> Result<(), StartupError> {
    info!(event = "zulipbot_starting", ?config);
    let settings = config.settings()?;
    let zulip = ZulipClient::new(&config.zulip_credentials())?;
    let sessions =
        SessionClient::with_bearer_token(config.centaur_api_url.clone(), &config.centaur_api_key)?;
    let identity = Backoff::REQUEST
        .retry("get_own_user", || zulip.own_user())
        .await?;
    let policy = config.policy();
    if policy.is_inert() {
        warn!(
            event = "zulipbot_policy_inert",
            hint = "Set ZULIPBOT_CHANNEL_ALLOWLIST or ZULIPBOT_DIRECT_MESSAGE_ALLOWLIST. \
                    The bot ignores every message until one is set.",
        );
    }
    info!(event = "zulipbot_identity", user_id = %identity.user_id, name = %identity.full_name);
    let bot = Arc::new(Bot::new(zulip, sessions, identity, policy, settings));

    let shutdown = CancellationToken::new();
    let tasks = TaskTracker::new();
    let health = Arc::new(Health::default());
    let server = tokio::spawn({
        let app = Router::new()
            .route("/healthz", get(healthz))
            .with_state(health.clone());
        let listener = tokio::net::TcpListener::bind(config.bind_addr)
            .await
            .map_err(|source| StartupError::Bind {
                address: config.bind_addr,
                source,
            })?;
        let shutdown = shutdown.clone();
        async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        }
    });
    let listener = Listener::new(bot, tasks.clone(), health);

    let mut terminate = signal(SignalKind::terminate()).map_err(StartupError::Signal)?;
    let result = tokio::select! {
        result = listener.run(&shutdown) => result.map_err(StartupError::from),
        _ = terminate.recv() => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    };
    info!(event = "zulipbot_stopping");
    shutdown.cancel();
    tasks.close();
    if tokio::time::timeout(SHUTDOWN_GRACE, tasks.wait())
        .await
        .is_err()
    {
        warn!(event = "zulipbot_shutdown_incomplete", tasks = tasks.len());
    }
    match server.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!(event = "zulipbot_health_server_failed", %error),
        Err(error) => warn!(event = "zulipbot_health_server_failed", %error),
    }
    result
}

/// Returns 200 while the event queue answers. It returns 503 when the
/// listener has not reached Zulip for [`listener::LIVENESS_WINDOW`].
async fn healthz(State(health): State<Arc<Health>>) -> StatusCode {
    if health.is_live(SystemTime::now()) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
