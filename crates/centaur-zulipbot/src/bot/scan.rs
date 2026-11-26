//! The recovery scan for unfinished messages.

use std::time::{SystemTime, UNIX_EPOCH};

use centaur_zulip::{History, Message, MessageId, ZulipApi, ZulipError};

use super::Bot;
use crate::{retry::Backoff, sessions::Sessions, status::Status};

/// The page size of the recovery scan. Zulip returns at most 5000 messages for
/// each request. The scan reads only mentions and direct messages.
const RECOVERY_SCAN_PAGE: u32 = 1000;

impl<Z: ZulipApi, S: Sessions> Bot<Z, S> {
    /// Returns the unfinished mentions and direct messages of the recovery
    /// window before `now`. The caller passes each message to
    /// [`Bot::receive_recovered`]. That call applies the policy.
    pub async fn unanswered_messages(&self, now: SystemTime) -> Result<Vec<Message>, ZulipError> {
        let oldest = now
            .checked_sub(self.settings.recovery_window)
            .unwrap_or(UNIX_EPOCH);
        let mut messages = Vec::new();
        for selector in [HistorySelector::Mentions, HistorySelector::DirectMessages] {
            messages.extend(self.messages_since(selector, oldest).await?);
        }
        messages.sort_by_key(|message| message.id);
        messages.dedup_by_key(|message| message.id);
        messages.retain(|message| {
            message.sender_id != self.identity.user_id
                && !Status::of(message, self.identity.user_id).is_some_and(Status::is_final)
        });
        Ok(messages)
    }

    /// Reads the messages of `selector` that were sent at `oldest` or later.
    /// The read goes back one page at a time. It stops at the page that
    /// reaches `oldest` or at the end of the history.
    async fn messages_since(
        &self,
        selector: HistorySelector,
        oldest: SystemTime,
    ) -> Result<Vec<Message>, ZulipError> {
        let mut messages = Vec::new();
        let mut before = None;
        loop {
            let history = selector.before(before);
            let page = Backoff::REQUEST
                .retry("scan_messages", || {
                    self.zulip.messages(history, RECOVERY_SCAN_PAGE)
                })
                .await?;
            let Some(first) = page.first() else {
                break;
            };
            let reached_oldest = first.sent_at() < oldest;
            let full_page = page.len() == RECOVERY_SCAN_PAGE as usize;
            before = Some(first.id);
            messages.extend(
                page.into_iter()
                    .filter(|message| message.sent_at() >= oldest),
            );
            if reached_oldest || !full_page {
                break;
            }
        }
        Ok(messages)
    }
}

/// A history source that the recovery scan pages through.
#[derive(Clone, Copy, Debug)]
enum HistorySelector {
    Mentions,
    DirectMessages,
}

impl HistorySelector {
    fn before(self, before: Option<MessageId>) -> History<'static> {
        match self {
            Self::Mentions => History::Mentions { before },
            Self::DirectMessages => History::DirectMessages { before },
        }
    }
}
