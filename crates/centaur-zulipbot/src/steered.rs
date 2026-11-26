//! The messages that steered each execution.
//!
//! A steering message gets the final status of the answer of its execution.
//! The steer and the end of the execution can come in either order. The
//! record therefore keeps the final status of each recently ended execution.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use centaur_zulip::MessageId;

use crate::status::Status;

pub struct SteeredMessages {
    state: Mutex<State>,
}

struct State {
    /// The messages that steered each execution whose answer is pending.
    waiting: HashMap<String, Vec<MessageId>>,
    /// The final status of each recently ended execution.
    ended: HashMap<String, Status>,
    /// The ended executions, oldest first.
    ended_order: VecDeque<String>,
    /// The number of ended executions that the record keeps.
    capacity: usize,
}

impl SteeredMessages {
    /// Returns a record that keeps the final status of the `capacity` most
    /// recently ended executions.
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                waiting: HashMap::new(),
                ended: HashMap::new(),
                ended_order: VecDeque::new(),
                capacity,
            }),
        }
    }

    /// Records that `message` steered `execution_id`. It returns the final
    /// status of an execution that already ended. The message then gets that
    /// status at once.
    pub fn record(&self, execution_id: &str, message: MessageId) -> Option<Status> {
        let mut state = self.lock();
        if let Some(status) = state.ended.get(execution_id) {
            return Some(*status);
        }
        state
            .waiting
            .entry(execution_id.to_owned())
            .or_default()
            .push(message);
        None
    }

    /// Ends `execution_id` with `status`. It returns the messages that
    /// steered the execution.
    pub fn end(&self, execution_id: &str, status: Status) -> Vec<MessageId> {
        let mut state = self.lock();
        if state
            .ended
            .insert(execution_id.to_owned(), status)
            .is_none()
        {
            state.ended_order.push_back(execution_id.to_owned());
            if state.ended_order.len() > state.capacity
                && let Some(oldest) = state.ended_order.pop_front()
            {
                state.ended.remove(&oldest);
            }
        }
        state.waiting.remove(execution_id).unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("no handler panics while it holds the steered record")
    }
}

#[cfg(test)]
mod tests {
    use super::SteeredMessages;
    use crate::status::Status;
    use centaur_zulip::MessageId;

    #[test]
    fn the_end_returns_the_messages_that_steered_the_execution() {
        let steered = SteeredMessages::new(4);
        assert_eq!(steered.record("exe-1", MessageId(11)), None);
        assert_eq!(steered.record("exe-1", MessageId(12)), None);
        assert_eq!(steered.record("exe-2", MessageId(13)), None);

        assert_eq!(
            steered.end("exe-1", Status::Answered),
            [MessageId(11), MessageId(12)]
        );
        assert_eq!(steered.end("exe-2", Status::Failed), [MessageId(13)]);
    }

    #[test]
    fn a_steer_after_the_end_gets_the_final_status() {
        let steered = SteeredMessages::new(4);
        assert!(steered.end("exe-1", Status::Stopped).is_empty());

        assert_eq!(
            steered.record("exe-1", MessageId(11)),
            Some(Status::Stopped)
        );
        assert_eq!(steered.record("exe-2", MessageId(12)), None);
    }

    #[test]
    fn the_record_forgets_the_oldest_ended_execution_past_its_capacity() {
        let steered = SteeredMessages::new(1);
        steered.end("exe-1", Status::Answered);
        steered.end("exe-2", Status::Answered);

        assert_eq!(steered.record("exe-1", MessageId(11)), None);
        assert_eq!(
            steered.record("exe-2", MessageId(12)),
            Some(Status::Answered)
        );
    }
}
