use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use centaur_session_client::ClientError;
use centaur_session_core::{
    ExecutionStatus, HarnessType, ThreadKey,
    api::{
        ApiErrorBody, ApiErrorCode, AppendMessagesRequest, AppendMessagesResponse,
        CreateSessionRequest, ExecuteSessionRequest, ExecuteSessionResponse, OnHarnessConflict,
        SteeringOutcome, SteeringStatus,
    },
    event::{ExecutionCompleted, ExecutionEnd, ProgressUpdate},
};
use reqwest::StatusCode;
use tokio::sync::Semaphore;

use crate::sessions::{OpenedSession, Sessions};

/// An in-memory session API. Executions are idempotent by key as in api-rs.
/// Each execution ends with its scripted outcome.
#[derive(Default)]
pub struct FakeSessions {
    state: Mutex<State>,
    /// A gate holds each execution until the test adds a permit.
    gate: Option<Arc<Semaphore>>,
}

#[derive(Default)]
struct State {
    created: Vec<(ThreadKey, CreateSessionRequest)>,
    appended: Vec<(ThreadKey, AppendMessagesRequest)>,
    executed: Vec<(ThreadKey, ExecuteSessionRequest)>,
    steering: VecDeque<Option<SteeringOutcome>>,
    create_errors: VecDeque<ClientError>,
    /// The harness of the existing session. A create request for another
    /// harness conflicts unless it allows a restart.
    harness: Option<HarnessType>,
    /// The persona that the deployment offers. Every other requested persona
    /// is unavailable.
    offered_persona: Option<String>,
    persona_model: Option<String>,
    execution_ids: HashMap<String, String>,
    ends: HashMap<String, ExecutionEnd>,
    /// The progress updates of each execution, in order.
    progress: HashMap<String, Vec<ProgressUpdate>>,
    waits: usize,
    /// The errors of the next waits, in order. A failed wait reports the
    /// progress updates first.
    wait_errors: VecDeque<ClientError>,
    /// The thread keys and reasons of the interrupts in order.
    interrupts: Vec<(ThreadKey, String)>,
    /// True when an interrupt finds a running execution.
    running: bool,
}

/// The answer of an execution whose end the test does not script.
pub fn default_answer(execution_id: &str) -> String {
    format!("answer from {execution_id}")
}

impl FakeSessions {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Reports `outcome` for the next append. Appends without a scripted
    /// outcome report no active execution.
    pub fn with_steering(self, outcome: Option<SteeringOutcome>) -> Self {
        self.state().steering.push_back(outcome);
        self
    }

    /// Makes the session exist on `harness`.
    pub fn with_existing_session(self, harness: HarnessType) -> Self {
        self.state().harness = Some(harness);
        self
    }

    /// Makes `persona_id` the only persona of the deployment.
    pub fn offering_persona(self, persona_id: &str) -> Self {
        self.state().offered_persona = Some(persona_id.to_owned());
        self
    }

    /// Makes the persona of the session pin `model`.
    pub fn with_persona_model(self, model: &str) -> Self {
        self.state().persona_model = Some(model.to_owned());
        self
    }

    pub fn with_create_error(self, error: ClientError) -> Self {
        self.state().create_errors.push_back(error);
        self
    }

    pub fn with_end(self, execution_id: &str, end: ExecutionEnd) -> Self {
        self.state().ends.insert(execution_id.to_owned(), end);
        self
    }

    /// Reports `text` as a progress update of `execution_id` that the server
    /// wrote at `created_at_ms`.
    pub fn with_progress(self, execution_id: &str, text: &str, created_at_ms: i64) -> Self {
        self.state()
            .progress
            .entry(execution_id.to_owned())
            .or_default()
            .push(ProgressUpdate {
                execution_id: execution_id.to_owned(),
                text: text.to_owned(),
                created_at_ms,
                elapsed_secs: 0,
                model: "progress-model".to_owned(),
            });
        self
    }

    pub fn with_wait_error(self, error: ClientError) -> Self {
        self.state().wait_errors.push_back(error);
        self
    }

    /// Makes interrupts find a running execution.
    pub fn with_running_execution(self) -> Self {
        self.state().running = true;
        self
    }

    pub fn interrupts(&self) -> Vec<(ThreadKey, String)> {
        self.state().interrupts.clone()
    }

    /// Holds every execution open until [`FakeSessions::release`].
    pub fn holding_executions(mut self) -> Self {
        self.gate = Some(Arc::new(Semaphore::new(0)));
        self
    }

    pub fn release(&self, executions: usize) {
        self.gate
            .as_ref()
            .expect("executions are held")
            .add_permits(executions);
    }

    pub fn created(&self) -> Vec<(ThreadKey, CreateSessionRequest)> {
        self.state().created.clone()
    }

    pub fn appended(&self) -> Vec<(ThreadKey, AppendMessagesRequest)> {
        self.state().appended.clone()
    }

    pub fn executed(&self) -> Vec<(ThreadKey, ExecuteSessionRequest)> {
        self.state().executed.clone()
    }

    /// How many waits for an execution's end have started.
    pub fn waits(&self) -> usize {
        self.state().waits
    }
}

pub fn steering(status: SteeringStatus, execution_id: Option<&str>) -> Option<SteeringOutcome> {
    Some(SteeringOutcome {
        status,
        execution_id: execution_id.map(str::to_owned),
    })
}

impl Sessions for FakeSessions {
    async fn create_session(
        &self,
        thread_key: &ThreadKey,
        request: &CreateSessionRequest,
    ) -> Result<OpenedSession, ClientError> {
        let mut state = self.state();
        if let Some(error) = state.create_errors.pop_front() {
            return Err(error);
        }
        state.created.push((thread_key.clone(), request.clone()));
        let harness_switched = match &state.harness {
            Some(existing) if *existing != request.harness_type => {
                if request.on_harness_conflict != OnHarnessConflict::Restart {
                    let body = ApiErrorBody {
                        code: Some(ApiErrorCode::HarnessConflict),
                        existing_harness: Some(*existing),
                        requested_harness: Some(request.harness_type),
                        ..ApiErrorBody::new("harness conflict")
                    };
                    return Err(ClientError::api(
                        StatusCode::CONFLICT,
                        serde_json::to_string(&body).expect("the body serializes"),
                    ));
                }
                true
            }
            Some(_) => false,
            None => {
                // A new session gets its persona.
                let unavailable = request
                    .persona_id
                    .clone()
                    .filter(|persona| state.offered_persona.as_ref() != Some(persona));
                state.harness = Some(request.harness_type);
                return Ok(OpenedSession {
                    harness: request.harness_type,
                    harness_switched: false,
                    unavailable_persona_id: unavailable,
                    persona_model: state.persona_model.clone(),
                });
            }
        };
        state.harness = Some(request.harness_type);
        Ok(OpenedSession {
            harness: request.harness_type,
            harness_switched,
            unavailable_persona_id: None,
            persona_model: state.persona_model.clone(),
        })
    }

    async fn append_messages(
        &self,
        thread_key: &ThreadKey,
        request: &AppendMessagesRequest,
    ) -> Result<AppendMessagesResponse, ClientError> {
        let mut state = self.state();
        state.appended.push((thread_key.clone(), request.clone()));
        let steering = state
            .steering
            .pop_front()
            .unwrap_or_else(|| steering(SteeringStatus::NoActiveExecution, None));
        Ok(AppendMessagesResponse {
            ok: true,
            message_ids: Vec::new(),
            steering,
        })
    }

    async fn execute_session(
        &self,
        thread_key: &ThreadKey,
        request: &ExecuteSessionRequest,
    ) -> Result<ExecuteSessionResponse, ClientError> {
        let mut state = self.state();
        state.executed.push((thread_key.clone(), request.clone()));
        let key = request.idempotency_key.clone().unwrap_or_default();
        let next_id = format!("exe-{}", state.execution_ids.len() + 1);
        let execution_id = state.execution_ids.entry(key).or_insert(next_id).clone();
        Ok(ExecuteSessionResponse {
            ok: true,
            execution_id,
            thread_key: thread_key.clone(),
            status: ExecutionStatus::Running,
        })
    }

    async fn interrupt(&self, thread_key: &ThreadKey, reason: &str) -> Result<bool, ClientError> {
        let mut state = self.state();
        state
            .interrupts
            .push((thread_key.clone(), reason.to_owned()));
        Ok(std::mem::take(&mut state.running))
    }

    async fn wait_for_execution_end(
        &self,
        thread_key: &ThreadKey,
        execution_id: &str,
        mut on_progress: impl FnMut(ProgressUpdate) + Send,
    ) -> Result<ExecutionEnd, ClientError> {
        // Each wait replays every update, as a wait from the start of the
        // event stream does.
        let (progress, error) = {
            let mut state = self.state();
            state.waits += 1;
            let progress = state
                .progress
                .get(execution_id)
                .cloned()
                .unwrap_or_default();
            (progress, state.wait_errors.pop_front())
        };
        for update in progress {
            on_progress(update);
            // The bot posts the update while the execution runs.
            tokio::task::yield_now().await;
        }
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("gate stays open").forget();
        }
        let end = self.state().ends.get(execution_id).cloned();
        Ok(end.unwrap_or_else(|| {
            ExecutionEnd::Completed(ExecutionCompleted {
                execution_id: execution_id.to_owned(),
                thread_key: thread_key.clone(),
                completion_reason: "turn_completed".to_owned(),
                result_text: Some(default_answer(execution_id)),
            })
        }))
    }
}
