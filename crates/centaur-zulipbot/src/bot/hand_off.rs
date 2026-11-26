//! The hand-off: the session, the stored messages, and the execution.

use centaur_session_client::ClientError;
use centaur_session_core::{
    ThreadKey,
    api::{ApiErrorCode, CreateSessionRequest, SteeringStatus},
};
use tracing::{error, info, warn};

use super::{Bot, HandOff};
use crate::{
    files::Files,
    flags::Flags,
    footer,
    requester::Requester,
    retry::Backoff,
    selection::{Rejected, SELECTION_ERROR_NOTICE, Selection},
    sessions::{OpenedSession, Sessions},
    status::Status,
    turn::{self, Request, Turn},
};
use centaur_zulip::{Conversation, History, Message, User, ZulipApi};

pub(super) const START_FAILED_NOTICE: &str =
    "I could not start working on this message. Please send it again in a moment.";
pub(super) const STEERED_FLAGS_NOTICE: &str =
    "This message joined the request that is running. Its flags apply only to a new request.";

impl<Z: ZulipApi, S: Sessions> Bot<Z, S> {
    pub(super) async fn hand_off(
        &self,
        message: &Message,
        sender: &User,
        conversation: &Conversation,
        thread_key: &ThreadKey,
    ) -> HandOff {
        self.add_reaction(message.id, Status::Working).await;
        match self.start(message, sender, conversation, thread_key).await {
            Ok(handoff) => handoff,
            Err(error)
                if error.api_code() == Some(ApiErrorCode::SessionPrincipalNotPreapproved) =>
            {
                // The bot posts nothing in a conversation that the deployment
                // does not admit.
                info!(
                    event = "zulipbot_conversation_not_admitted",
                    message_id = %message.id,
                    thread_key = %thread_key,
                );
                self.remove_reaction(message.id, Status::Working).await;
                HandOff::NotAdmitted
            }
            Err(error) => {
                error!(
                    event = "zulipbot_handoff_failed",
                    message_id = %message.id,
                    thread_key = %thread_key,
                    %error,
                );
                self.post(conversation, START_FAILED_NOTICE).await;
                self.finish(message.id, Status::Failed).await;
                HandOff::Failed
            }
        }
    }

    /// Returns the messages of the conversation before `message`. A failed
    /// read returns no messages. The turn then runs with less context.
    async fn history_before(&self, message: &Message, conversation: &Conversation) -> Vec<Message> {
        let history = History::Conversation {
            conversation,
            before: message.id,
        };
        Backoff::REQUEST
            .retry("fetch_history", || {
                self.zulip.messages(history, self.settings.context_messages)
            })
            .await
            .unwrap_or_else(|error| {
                warn!(event = "zulipbot_history_unavailable", message_id = %message.id, %error);
                Vec::new()
            })
    }

    /// Opens the session, stores the new messages, and starts or steers the
    /// execution. The session opens first. A conversation that the
    /// deployment does not admit therefore gets no history reads and no
    /// downloads.
    async fn start(
        &self,
        message: &Message,
        sender: &User,
        conversation: &Conversation,
        thread_key: &ThreadKey,
    ) -> Result<HandOff, ClientError> {
        let flags = Flags::parse(&message.content);
        let (mut selection, rejected) = match self
            .settings
            .choices
            .select(&flags, conversation.channel_id())
        {
            Ok(selection) => (selection, false),
            Err(Rejected { fallback }) => (fallback, true),
        };
        // The session opens before any notice. A conversation that the
        // deployment does not admit therefore gets no notice.
        let session = self
            .open_session(conversation, thread_key, &selection)
            .await?;
        if rejected {
            self.post(conversation, SELECTION_ERROR_NOTICE).await;
            self.finish(message.id, Status::Failed).await;
            return Ok(HandOff::Rejected);
        }
        selection.fit(session.harness);
        // A message that the recovery scan resumes got its notices before
        // the restart.
        let resumed = Status::of(message, self.identity.user_id) == Some(Status::Working);
        if !resumed {
            self.post_selection_notices(conversation, &selection, &session)
                .await;
        }
        let requester = Requester::of(sender, &self.server_settings().github_fields);
        let history = self.history_before(message, conversation).await;
        let request = Request {
            message,
            text: &flags.text,
            selection: &selection,
            requester: &requester,
        };
        let turn = Turn::new(
            thread_key,
            conversation,
            request,
            history,
            self.identity.user_id,
            session.harness_switched,
        );
        let files = Files::fetch(
            &self.zulip,
            turn.by_file_priority(),
            self.settings.file_bytes,
        )
        .await;
        let append = turn.append_request(&files);
        let appended = Backoff::REQUEST
            .retry("append_messages", || {
                self.sessions.append_messages(thread_key, &append)
            })
            .await?;
        if let Some(steering) = appended.steering {
            match steering.status {
                // The harness can have the message already. A second execution
                // would answer the message twice.
                SteeringStatus::Delivered | SteeringStatus::DeliveryUnknown => {
                    info!(
                        event = "zulipbot_message_steered",
                        message_id = %turn.request.message.id,
                        thread_key = %thread_key,
                        execution_id = steering.execution_id.as_deref(),
                    );
                    let message_id = turn.request.message.id;
                    self.finish(message_id, Status::Steered).await;
                    if flags.chooses_anything() && !resumed {
                        self.post(conversation, STEERED_FLAGS_NOTICE).await;
                    }
                    if let Some(execution_id) = steering.execution_id
                        && let Some(status) = self.steered.record(&execution_id, message_id)
                    {
                        self.replace_reaction(message_id, Status::Steered, status)
                            .await;
                    }
                    return Ok(HandOff::Steered);
                }
                SteeringStatus::NoActiveExecution | SteeringStatus::NotDelivered => {}
            }
        }
        let execute = turn.execute_request(self.settings.limits, &files);
        let execution = Backoff::REQUEST
            .retry("execute_session", || {
                self.sessions.execute_session(thread_key, &execute)
            })
            .await?;
        info!(
            event = "zulipbot_execution_started",
            message_id = %turn.request.message.id,
            thread_key = %thread_key,
            execution_id = %execution.execution_id,
        );
        Ok(HandOff::Execution {
            execution_id: execution.execution_id,
            footer: self.footer(&selection, &session, &turn),
        })
    }

    /// Returns the metadata line of the answer that the metadata mode asks
    /// for.
    fn footer(
        &self,
        selection: &Selection,
        session: &OpenedSession,
        turn: &Turn<'_>,
    ) -> Option<String> {
        self.settings
            .metadata
            .shows(selection.explicit, turn.is_first())
            .then(|| {
                footer::line(
                    selection,
                    session,
                    &self.settings.harness_defaults,
                    self.settings.choices.catalog(),
                )
            })
            .flatten()
    }

    /// Creates or reuses the session. A selection flag restarts a session on
    /// another harness. Otherwise the session keeps its harness.
    async fn open_session(
        &self,
        conversation: &Conversation,
        thread_key: &ThreadKey,
        selection: &Selection,
    ) -> Result<OpenedSession, ClientError> {
        let request = turn::create_request(
            conversation,
            selection.harness,
            selection.persona_id.clone(),
            selection.restart,
        );
        match self.create_session(thread_key, &request).await {
            Err(error) if !selection.restart => {
                let Some(existing) = error.existing_harness() else {
                    return Err(error);
                };
                let request = CreateSessionRequest {
                    harness_type: existing,
                    ..request
                };
                self.create_session(thread_key, &request).await
            }
            result => result,
        }
    }

    async fn create_session(
        &self,
        thread_key: &ThreadKey,
        request: &CreateSessionRequest,
    ) -> Result<OpenedSession, ClientError> {
        Backoff::REQUEST
            .retry("create_session", || {
                self.sessions.create_session(thread_key, request)
            })
            .await
    }

    /// Tells the sender about a choice that the session cannot honor.
    async fn post_selection_notices(
        &self,
        conversation: &Conversation,
        selection: &Selection,
        session: &OpenedSession,
    ) {
        if let Some(harness) = &selection.refused_harness {
            self.post(
                conversation,
                &format!(
                    "The harness `{harness}` is not enabled. The request runs on the harness of \
                     the conversation."
                ),
            )
            .await;
        }
        if let Some(persona_id) = &session.unavailable_persona_id {
            self.post(
                conversation,
                &format!(
                    "The persona `{persona_id}` is not available. This conversation uses the \
                     default persona."
                ),
            )
            .await;
        }
        if let (Some(pinned), Some(requested)) = (&session.persona_model, &selection.model)
            && pinned != requested
        {
            self.post(
                conversation,
                &format!(
                    "The persona of this conversation runs the model `{pinned}`. The \
                     `--model` flag has no effect."
                ),
            )
            .await;
        }
    }
}
