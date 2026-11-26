use std::collections::BTreeSet;

use super::{Allowlist, Ignored, Policy};
use crate::testing::{ADA, BOT, ENGINEERING, GRACE, conversation, direct_message, topic_message};
use centaur_zulip::{ChannelId, UserId};

fn open_policy() -> Policy {
    Policy {
        channels: Allowlist::Only(BTreeSet::from([ENGINEERING])),
        direct_message_senders: Allowlist::Only(BTreeSet::from([ADA])),
        trigger_bots: Allowlist::default(),
        trigger_guests: Allowlist::default(),
    }
}

fn check(policy: &Policy, message: &centaur_zulip::Message) -> Result<(), Ignored> {
    policy.check_message(message, &conversation(message), BOT)
}

mod allowlist {
    use super::{Allowlist, BTreeSet, ChannelId};

    #[test]
    fn parses_ids_separated_by_commas_or_whitespace() {
        let list: Allowlist<ChannelId> = "42, 7\n9".parse().unwrap();
        assert_eq!(
            list,
            Allowlist::Only(BTreeSet::from([ChannelId(7), ChannelId(9), ChannelId(42)]))
        );
        assert!(list.allows(&ChannelId(42)));
        assert!(!list.allows(&ChannelId(43)));
    }

    #[test]
    fn star_allows_everyone() {
        let list: Allowlist<ChannelId> = " * ".parse().unwrap();
        assert!(list.allows(&ChannelId(12345)));
        assert!(!list.is_empty());
    }

    #[test]
    fn empty_text_allows_no_one() {
        let list: Allowlist<ChannelId> = "".parse().unwrap();
        assert!(list.is_empty());
        assert!(!list.allows(&ChannelId(42)));
    }

    #[test]
    fn rejects_an_id_that_is_not_a_number() {
        let error = "42,general".parse::<Allowlist<ChannelId>>().unwrap_err();
        assert_eq!(error.id, "general");
        assert!(
            error.to_string().starts_with("invalid ID \"general\""),
            "{error}"
        );
    }
}

mod channel_messages {
    use super::{ADA, BOT, Ignored, Policy, check, open_policy, topic_message};

    #[test]
    fn a_mention_in_an_allowed_channel_starts_work() {
        let message = topic_message(1, ADA, "@**Centaur** hi", true);
        assert_eq!(check(&open_policy(), &message), Ok(()));
    }

    #[test]
    fn a_message_without_a_mention_is_not_addressed() {
        let message = topic_message(1, ADA, "hi all", false);
        assert_eq!(check(&open_policy(), &message), Err(Ignored::NotAddressed));
    }

    #[test]
    fn a_channel_outside_the_allowlist_is_ignored() {
        let message = topic_message(1, ADA, "@**Centaur** hi", true);
        assert_eq!(
            check(&Policy::default(), &message),
            Err(Ignored::ChannelNotAllowed)
        );
    }

    #[test]
    fn the_bots_own_message_is_ignored() {
        let message = topic_message(1, BOT, "@**Centaur** echo", true);
        assert_eq!(check(&open_policy(), &message), Err(Ignored::OwnMessage));
    }
}

mod direct_messages {
    use super::{ADA, GRACE, Ignored, check, direct_message, open_policy};

    #[test]
    fn a_one_to_one_message_needs_no_mention() {
        let message = direct_message(1, ADA, &[], "hi");
        assert_eq!(check(&open_policy(), &message), Ok(()));
    }

    #[test]
    fn a_sender_outside_the_allowlist_is_ignored() {
        let message = direct_message(1, GRACE, &[], "hi");
        assert_eq!(
            check(&open_policy(), &message),
            Err(Ignored::SenderNotAllowed)
        );
    }

    #[test]
    fn a_group_message_needs_a_mention() {
        let mut message = direct_message(1, ADA, &[GRACE], "hi both");
        assert_eq!(check(&open_policy(), &message), Err(Ignored::NotAddressed));
        message.mentions_bot = true;
        assert_eq!(check(&open_policy(), &message), Ok(()));
    }
}

mod senders {
    use super::{Allowlist, BTreeSet, Ignored, Policy, UserId, open_policy};
    use centaur_zulip::User;

    fn sender(id: u64, is_bot: bool, is_guest: bool) -> User {
        User {
            user_id: UserId(id),
            full_name: "Sender".to_owned(),
            is_bot,
            is_guest,
            profile: Default::default(),
        }
    }

    #[test]
    fn members_pass_the_sender_check() {
        assert_eq!(open_policy().check_sender(&sender(7, false, false)), Ok(()));
    }

    #[test]
    fn bots_need_the_trigger_bot_allowlist() {
        let policy = Policy {
            trigger_bots: Allowlist::Only(BTreeSet::from([UserId(50)])),
            ..open_policy()
        };
        assert_eq!(policy.check_sender(&sender(50, true, false)), Ok(()));
        assert_eq!(
            policy.check_sender(&sender(51, true, false)),
            Err(Ignored::BotNotAllowed)
        );
    }

    #[test]
    fn guests_need_the_guest_allowlist() {
        let policy = Policy {
            trigger_guests: Allowlist::Only(BTreeSet::from([UserId(60)])),
            ..open_policy()
        };
        assert_eq!(policy.check_sender(&sender(60, false, true)), Ok(()));
        assert_eq!(
            policy.check_sender(&sender(61, false, true)),
            Err(Ignored::GuestNotAllowed)
        );
    }

    #[test]
    fn everyone_open_for_direct_messages_still_leaves_out_guests() {
        let policy = Policy {
            direct_message_senders: Allowlist::Everyone,
            ..open_policy()
        };
        assert_eq!(
            policy.check_sender(&sender(61, false, true)),
            Err(Ignored::GuestNotAllowed)
        );
    }
}

#[test]
fn a_policy_without_channels_or_senders_is_inert() {
    assert!(Policy::default().is_inert());
    assert!(!open_policy().is_inert());
}
