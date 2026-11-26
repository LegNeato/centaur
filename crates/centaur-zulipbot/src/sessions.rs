//! The session API calls that the bot makes.
//!
//! [`SessionClient`] implements [`Sessions`] over HTTP. The tests implement it
//! with an in-memory fake.

use std::future::Future;

use centaur_session_client::{ClientError, Reconnect, SessionClient};
use centaur_session_core::{
    HarnessType, ThreadKey,
    api::{
        AppendMessagesRequest, AppendMessagesResponse, CreateSessionRequest, ExecuteSessionRequest,
        ExecuteSessionResponse, InterruptSessionExecutionRequest,
    },
    event::{ExecutionEnd, ProgressUpdate},
};

/// The parts of a create response that the bot reads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenedSession {
    /// The harness that the session runs.
    pub harness: HarnessType,
    /// True when the request restarted the session on another harness. The
    /// new harness has no history.
    pub harness_switched: bool,
    /// The requested persona that the deployment does not offer. The session
    /// uses the default persona instead.
    pub unavailable_persona_id: Option<String>,
    /// The model that the persona of the session pins. The server ignores a
    /// requested model and provider then.
    pub persona_model: Option<String>,
}

pub trait Sessions: Send + Sync + 'static {
    /// Creates the session or reuses the existing one.
    fn create_session(
        &self,
        thread_key: &ThreadKey,
        request: &CreateSessionRequest,
    ) -> impl Future<Output = Result<OpenedSession, ClientError>> + Send;

    fn append_messages(
        &self,
        thread_key: &ThreadKey,
        request: &AppendMessagesRequest,
    ) -> impl Future<Output = Result<AppendMessagesResponse, ClientError>> + Send;

    fn execute_session(
        &self,
        thread_key: &ThreadKey,
        request: &ExecuteSessionRequest,
    ) -> impl Future<Output = Result<ExecuteSessionResponse, ClientError>> + Send;

    /// Waits for the end of `execution_id`. Each progress update of the
    /// execution goes to `on_progress` first.
    fn wait_for_execution_end(
        &self,
        thread_key: &ThreadKey,
        execution_id: &str,
        on_progress: impl FnMut(ProgressUpdate) + Send,
    ) -> impl Future<Output = Result<ExecutionEnd, ClientError>> + Send;

    /// Interrupts the active execution of the session. It returns false when
    /// no execution was running.
    fn interrupt(
        &self,
        thread_key: &ThreadKey,
        reason: &str,
    ) -> impl Future<Output = Result<bool, ClientError>> + Send;
}

impl Sessions for SessionClient {
    async fn create_session(
        &self,
        thread_key: &ThreadKey,
        request: &CreateSessionRequest,
    ) -> Result<OpenedSession, ClientError> {
        let response = SessionClient::create_session(self, thread_key, request).await?;
        Ok(OpenedSession {
            harness: response.session.harness_type,
            harness_switched: response.harness_switched,
            unavailable_persona_id: response.unavailable_requested_persona_id,
            persona_model: response.persona_model,
        })
    }

    async fn append_messages(
        &self,
        thread_key: &ThreadKey,
        request: &AppendMessagesRequest,
    ) -> Result<AppendMessagesResponse, ClientError> {
        SessionClient::append_messages(self, thread_key, request).await
    }

    async fn execute_session(
        &self,
        thread_key: &ThreadKey,
        request: &ExecuteSessionRequest,
    ) -> Result<ExecuteSessionResponse, ClientError> {
        SessionClient::execute_session(self, thread_key, request).await
    }

    async fn wait_for_execution_end(
        &self,
        thread_key: &ThreadKey,
        execution_id: &str,
        on_progress: impl FnMut(ProgressUpdate) + Send,
    ) -> Result<ExecutionEnd, ClientError> {
        SessionClient::wait_for_execution_end(
            self,
            thread_key,
            execution_id,
            Reconnect::default(),
            on_progress,
        )
        .await
    }

    async fn interrupt(&self, thread_key: &ThreadKey, reason: &str) -> Result<bool, ClientError> {
        let request = InterruptSessionExecutionRequest {
            reason: Some(reason.to_owned()),
        };
        SessionClient::interrupt_execution(self, thread_key, &request)
            .await
            .map(|response| response.interrupted)
    }
}
