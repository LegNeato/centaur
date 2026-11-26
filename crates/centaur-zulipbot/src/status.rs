//! The reactions that the bot adds to the messages it handles.
//!
//! The reactions show people the state of each message. They are also the
//! bot's only record of its work. The recovery scan reads them to find the
//! messages that still need an answer.

use centaur_zulip::{Message, UserId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// The bot is handling the message and has not posted the answer.
    Working,
    /// The message steered an execution that was already running. The answer
    /// of that execution includes the message.
    Steered,
    Answered,
    Failed,
    /// A stop command interrupted the execution of the message.
    Stopped,
}

impl Status {
    /// The statuses in the order that [`Status::of`] checks them. Final
    /// statuses come first because the bot adds a final reaction before it
    /// removes [`Status::Working`].
    const BY_PRECEDENCE: [Self; 5] = [
        Self::Answered,
        Self::Failed,
        Self::Stopped,
        Self::Steered,
        Self::Working,
    ];

    /// The Zulip name of the reaction emoji.
    pub const fn emoji_name(self) -> &'static str {
        match self {
            Self::Working => "eyes",
            Self::Steered => "time_ticking",
            Self::Answered => "check",
            Self::Failed => "cross_mark",
            Self::Stopped => "stop_sign",
        }
    }

    /// Returns true when the bot has no further work for the message.
    pub const fn is_final(self) -> bool {
        !matches!(self, Self::Working)
    }

    /// Returns true when the status marks a message that the session holds.
    /// [`Status::Failed`] also marks a rejected request and a failed
    /// hand-off. The bot stored neither. It therefore counts a failed message
    /// as not stored.
    pub const fn marks_stored(self) -> bool {
        !matches!(self, Self::Failed)
    }

    /// Returns the status that the bot's reactions on `message` record.
    pub fn of(message: &Message, bot: UserId) -> Option<Self> {
        Self::BY_PRECEDENCE
            .into_iter()
            .find(|status| message.has_reaction(bot, status.emoji_name()))
    }
}

#[cfg(test)]
mod tests {
    use super::Status;
    use crate::testing::{ADA, BOT, reacted, topic_message};

    #[test]
    fn no_bot_reaction_means_no_status() {
        let mut message = topic_message(1, ADA, "hi", true);
        // Only the bot's own reactions record a status.
        message.reactions.push(centaur_zulip::Reaction {
            emoji_name: "eyes".to_owned(),
            user_id: ADA,
        });
        assert_eq!(Status::of(&message, BOT), None);
    }

    #[test]
    fn a_final_reaction_wins_over_working() {
        let message = reacted(topic_message(1, ADA, "hi", true), &["eyes", "check"]);
        assert_eq!(Status::of(&message, BOT), Some(Status::Answered));
    }

    #[test]
    fn working_alone_is_not_final() {
        let message = reacted(topic_message(1, ADA, "hi", true), &["eyes"]);
        let status = Status::of(&message, BOT).unwrap();
        assert_eq!(status, Status::Working);
        assert!(!status.is_final());
    }

    #[test]
    fn only_a_failed_message_may_be_missing_from_the_session() {
        assert!(!Status::Failed.marks_stored());
        for status in [
            Status::Working,
            Status::Steered,
            Status::Answered,
            Status::Stopped,
        ] {
            assert!(status.marks_stored(), "{status:?}");
        }
    }

    #[test]
    fn every_status_has_its_own_emoji() {
        let mut names = Status::BY_PRECEDENCE.map(Status::emoji_name).to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Status::BY_PRECEDENCE.len());
    }
}
