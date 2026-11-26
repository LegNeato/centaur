//! In-process coordination between message handlers.

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    hash::Hash,
    pin::pin,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use tokio::{sync::Notify, time::Instant};

/// One first-in, first-out queue for each key. A caller joins the queue of a
/// key at once and waits for its turn later. Work on one key therefore runs
/// in the order of the [`Lanes::join`] calls, whatever the order of the
/// waits. Work on different keys runs in parallel.
pub struct Lanes<K> {
    lanes: Arc<Mutex<HashMap<K, Lane>>>,
}

struct Lane {
    /// The ticket of the next place that joins.
    next: u64,
    /// The ticket of the place whose turn it is.
    serving: u64,
    /// The places that left before their turn.
    left: BTreeSet<u64>,
    turn: Arc<Notify>,
}

/// A place in the queue of one key. The turn passes to the next place when
/// this place is dropped.
pub struct Place<K: Eq + Hash> {
    lanes: Arc<Mutex<HashMap<K, Lane>>>,
    key: K,
    ticket: u64,
    turn: Arc<Notify>,
}

impl<K> Default for Lanes<K> {
    fn default() -> Self {
        Self {
            lanes: Arc::default(),
        }
    }
}

impl<K: Clone + Eq + Hash> Lanes<K> {
    /// Takes the last place in the queue of `key`.
    pub fn join(&self, key: K) -> Place<K> {
        let mut lanes = lock(&self.lanes);
        let lane = lanes.entry(key.clone()).or_insert_with(|| Lane {
            next: 0,
            serving: 0,
            left: BTreeSet::new(),
            turn: Arc::default(),
        });
        let ticket = lane.next;
        lane.next += 1;
        Place {
            lanes: self.lanes.clone(),
            key,
            ticket,
            turn: lane.turn.clone(),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.lanes).len()
    }
}

impl<K: Eq + Hash> Place<K> {
    /// Waits until every earlier place of the key is dropped.
    pub async fn turn(&self) {
        loop {
            let mut notified = pin!(self.turn.notified());
            // The waiter registers before the check. A turn that passes
            // between the check and the wait therefore still wakes it.
            notified.as_mut().enable();
            if self.is_serving() {
                return;
            }
            notified.await;
        }
    }

    fn is_serving(&self) -> bool {
        lock(&self.lanes)
            .get(&self.key)
            .is_some_and(|lane| lane.serving == self.ticket)
    }
}

impl<K: Eq + Hash> Drop for Place<K> {
    fn drop(&mut self) {
        let mut lanes = lock(&self.lanes);
        let Some(lane) = lanes.get_mut(&self.key) else {
            return;
        };
        if lane.serving != self.ticket {
            lane.left.insert(self.ticket);
            return;
        }
        lane.serving += 1;
        while lane.left.remove(&lane.serving) {
            lane.serving += 1;
        }
        if lane.serving == lane.next {
            lanes.remove(&self.key);
        } else {
            lane.turn.notify_waiters();
        }
    }
}

/// The values that handlers hold now and the values they held recently. The
/// bot claims each message ID. A message that the event queue and the
/// recovery scan both deliver is then handled once. A message that is still
/// in progress is never handled twice.
pub struct Claims<T> {
    state: Arc<Mutex<ClaimState<T>>>,
}

struct ClaimState<T> {
    held: HashSet<T>,
    recent: RecentSet<T>,
}

/// A claimed value. The value stays recent after the claim is dropped.
pub struct Claim<T: Eq + Hash> {
    state: Arc<Mutex<ClaimState<T>>>,
    value: T,
}

impl<T: Clone + Eq + Hash> Claims<T> {
    /// Returns claims that remember the `recent` most recently claimed
    /// values.
    pub fn new(recent: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ClaimState {
                held: HashSet::new(),
                recent: RecentSet::new(recent),
            })),
        }
    }

    /// Claims `value`. It returns `None` while another claim holds `value`
    /// and for a recently claimed value.
    pub fn claim(&self, value: T) -> Option<Claim<T>> {
        let mut state = lock(&self.state);
        if state.held.contains(&value) || !state.recent.insert(value.clone()) {
            return None;
        }
        state.held.insert(value.clone());
        Some(Claim {
            state: self.state.clone(),
            value,
        })
    }
}

impl<T: Eq + Hash> Drop for Claim<T> {
    fn drop(&mut self) {
        lock(&self.state).held.remove(&self.value);
    }
}

/// The `capacity` most recently inserted values.
struct RecentSet<T> {
    capacity: usize,
    values: HashSet<T>,
    order: VecDeque<T>,
}

impl<T: Clone + Eq + Hash> RecentSet<T> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
        }
    }

    /// Inserts `value`. It returns false when the set already holds `value`.
    fn insert(&mut self, value: T) -> bool {
        if !self.values.insert(value.clone()) {
            return false;
        }
        self.order.push_back(value);
        if self.order.len() > self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.values.remove(&oldest);
        }
        true
    }
}

/// A map whose entries expire when `ttl` has passed since their insertion.
pub struct ExpiringMap<K, V> {
    ttl: Duration,
    entries: Mutex<HashMap<K, (V, Instant)>>,
}

impl<K: Eq + Hash, V: Clone> ExpiringMap<K, V> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::default(),
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let mut entries = lock(&self.entries);
        let now = Instant::now();
        entries.retain(|_, (_, inserted)| now.duration_since(*inserted) < self.ttl);
        entries.get(key).map(|(value, _)| value.clone())
    }

    pub fn insert(&self, key: K, value: V) {
        lock(&self.entries).insert(key, (value, Instant::now()));
    }
}

/// Locks `mutex`. A panic while a guard is held is a bug of this module.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("no handler panics while it holds the lock")
}

#[cfg(test)]
mod tests;
