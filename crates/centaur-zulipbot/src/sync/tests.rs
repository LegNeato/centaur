use std::{sync::Arc, time::Duration};

use super::{Claims, ExpiringMap, Lanes, RecentSet};

mod lanes {
    use super::*;

    /// Returns true when `place` gets its turn without a wait.
    async fn has_turn<K: Eq + std::hash::Hash>(place: &super::super::Place<K>) -> bool {
        tokio::time::timeout(Duration::from_millis(20), place.turn())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn places_of_one_key_take_turns_in_join_order() {
        let lanes = Lanes::default();
        let first = lanes.join("topic");
        let second = lanes.join("topic");

        // The second place waits first. It still waits for the first place.
        assert!(!has_turn(&second).await);
        assert!(has_turn(&first).await);
        drop(first);
        assert!(has_turn(&second).await);
    }

    #[tokio::test]
    async fn a_waiting_place_wakes_when_the_turn_passes() {
        let lanes = Arc::new(Lanes::default());
        let first = lanes.join("topic");
        let second = lanes.join("topic");
        let waiter = tokio::spawn(async move { second.turn().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        drop(first);

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("the turn passes to the waiter")
            .unwrap();
    }

    #[tokio::test]
    async fn a_place_that_leaves_early_does_not_skip_an_earlier_place() {
        let lanes = Lanes::default();
        let first = lanes.join("topic");
        let second = lanes.join("topic");
        let third = lanes.join("topic");

        drop(second);
        assert!(!has_turn(&third).await);
        drop(first);
        assert!(has_turn(&third).await);
    }

    #[tokio::test]
    async fn different_keys_do_not_wait_for_each_other() {
        let lanes = Lanes::default();
        let _topic = lanes.join("topic");
        let other = lanes.join("other");

        assert!(has_turn(&other).await);
    }

    #[tokio::test]
    async fn a_drained_key_leaves_no_entry() {
        let lanes = Lanes::default();
        let first = lanes.join("topic");
        let second = lanes.join("topic");
        drop(first);
        drop(second);
        assert_eq!(lanes.len(), 0);

        assert!(has_turn(&lanes.join("topic")).await);
    }
}

mod claims {
    use super::*;

    #[test]
    fn a_value_is_claimed_once() {
        let claims = Claims::new(8);
        let claim = claims.claim(1);
        assert!(claim.is_some());
        assert!(claims.claim(1).is_none());
        drop(claim);
        // The released value stays recent.
        assert!(claims.claim(1).is_none());
        assert!(claims.claim(2).is_some());
    }

    #[test]
    fn a_held_value_stays_claimed_after_it_leaves_the_recent_values() {
        let claims = Claims::new(1);
        let held = claims.claim(1).unwrap();
        // The claim of 2 makes 1 leave the recent values.
        drop(claims.claim(2).unwrap());
        assert!(claims.claim(1).is_none());

        drop(held);
        drop(claims.claim(3).unwrap());
        assert!(claims.claim(1).is_some());
    }
}

#[tokio::test(start_paused = true)]
async fn expiring_map_forgets_an_entry_after_its_lifetime() {
    let map = ExpiringMap::new(Duration::from_secs(60));
    map.insert("ada", 1);
    tokio::time::advance(Duration::from_secs(59)).await;
    assert_eq!(map.get(&"ada"), Some(1));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(map.get(&"ada"), None);
}

#[test]
fn recent_set_forgets_the_oldest_value_past_its_capacity() {
    let mut recent = RecentSet::new(2);
    assert!(recent.insert(1));
    assert!(!recent.insert(1));
    assert!(recent.insert(2));
    assert!(recent.insert(3));
    // The insertion of 3 evicted 1, the oldest value. The set holds 2 and 3.
    assert!(recent.insert(1));
    assert!(!recent.insert(3));
}
