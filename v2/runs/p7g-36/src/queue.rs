//! A single queue: its attributes, its messages and the indexes that keep
//! retention, receipt handles and deduplication cheap.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Maximum number of messages one queue may hold, in any state.
pub const MAX_MESSAGES: usize = 100_000;

/// How long (seconds) a FIFO deduplication id suppresses later sends.
const DEDUP_WINDOW: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    /// The `n` of `m-<n>`.
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    /// FIFO queues only.
    pub group_id: Option<String>,
    /// FIFO queues only; assigned by [`Queue::add`].
    pub sequence_number: Option<u64>,
    /// The message is delayed while `now` is below this.
    available_at: u64,
    /// The message is in flight while `now` is below this.
    invisible_until: u64,
    /// The `n` of the latest `r-<n>` handed out for this message.
    handle: Option<u64>,
}

impl Message {
    pub fn new(id: u64, body: String, sent_at: u64, delay: u64, group_id: Option<String>) -> Self {
        Message {
            id,
            body,
            sent_at,
            receive_count: 0,
            first_received_at: None,
            group_id,
            sequence_number: None,
            available_at: sent_at + delay,
            invisible_until: 0,
            handle: None,
        }
    }

    /// The message as it arrives in a dead-letter queue: same identity, fresh
    /// delivery history, no delay.
    fn reset_for_dead_letter(mut self) -> Self {
        self.receive_count = 0;
        self.first_received_at = None;
        self.sequence_number = None;
        self.available_at = 0;
        self.invisible_until = 0;
        self.handle = None;
        self
    }

    /// The first moment at which the message is visible.
    fn visible_from(&self) -> u64 {
        self.available_at.max(self.invisible_until)
    }

    fn state(&self, now: u64) -> State {
        if now < self.available_at {
            State::Delayed
        } else if now < self.invisible_until {
            State::InFlight
        } else {
            State::Visible
        }
    }

    /// The receipt handle number, if the message was delivered at all.
    pub fn handle(&self) -> Option<u64> {
        self.handle
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// Result of one [`Queue::receive`].
#[derive(Debug, Default)]
pub struct Received {
    /// Delivered messages (post-delivery state), in delivery order.
    pub delivered: Vec<Message>,
    /// Messages to hand to the dead-letter queue via [`Queue::add`].
    pub dead: Vec<Message>,
}

/// The send that opened the current deduplication window for an id.
#[derive(Debug, Clone, Copy)]
struct DedupEntry {
    accepted_at: u64,
    message_id: u64,
    sequence_number: Option<u64>,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    /// Messages keyed by arrival number, so iteration is arrival order.
    messages: BTreeMap<u64, Message>,
    /// Arrival numbers of the messages that are visible as of the last
    /// [`advance`](Self::advance).
    ready: BTreeSet<u64>,
    /// `(visible_from, arrival)` of the delayed and in-flight messages.
    waiting: BTreeSet<(u64, u64)>,
    /// FIFO only: arrival numbers of each group's messages, in any state.
    groups: HashMap<String, BTreeSet<u64>>,
    /// `(sent_at, arrival)` of every message, oldest first, for retention.
    by_age: BTreeSet<(u64, u64)>,
    /// Latest receipt handle number -> arrival number.
    handles: HashMap<u64, u64>,
    dedup: HashMap<String, DedupEntry>,
    next_arrival: u64,
    next_sequence: u64,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Queue {
            attributes,
            messages: BTreeMap::new(),
            ready: BTreeSet::new(),
            waiting: BTreeSet::new(),
            groups: HashMap::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            dedup: HashMap::new(),
            next_arrival: 0,
            next_sequence: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Brings the queue up to `now`: applies retention and moves messages
    /// whose delay or visibility timeout has ended into the ready set. Time
    /// only matters when a queue is looked at, so callers run this before
    /// touching a queue instead of on every clock tick.
    pub fn advance(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(arrival);
        }
        while let Some(&(visible_from, arrival)) = self.waiting.first() {
            if now < visible_from {
                break;
            }
            self.waiting.pop_first();
            self.ready.insert(arrival);
        }
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for m in self.messages.values() {
            match m.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Appends a message and returns the sequence number it was given (FIFO
    /// queues only).
    pub fn add(&mut self, mut message: Message, now: u64) -> Option<u64> {
        message.sequence_number = self.attributes.fifo.then(|| {
            let n = self.next_sequence;
            self.next_sequence += 1;
            n
        });
        let sequence = message.sequence_number;
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        if let Some(group) = &message.group_id {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(arrival);
        }
        self.schedule(arrival, message.visible_from(), now);
        self.messages.insert(arrival, message);
        sequence
    }

    /// Removes every message; sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.ready.clear();
        self.waiting.clear();
        self.groups.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    /// Files a message under `ready` or `waiting` according to when it
    /// becomes visible.
    fn schedule(&mut self, arrival: u64, visible_from: u64, now: u64) {
        if visible_from <= now {
            self.ready.insert(arrival);
        } else {
            self.waiting.insert((visible_from, arrival));
        }
    }

    /// Takes a message out of the ready/waiting indexes, e.g. to change its
    /// visibility.
    fn unschedule(&mut self, arrival: u64) {
        if !self.ready.remove(&arrival)
            && let Some(m) = self.messages.get(&arrival)
        {
            self.waiting.remove(&(m.visible_from(), arrival));
        }
    }

    fn remove(&mut self, arrival: u64) -> Option<Message> {
        self.unschedule(arrival);
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        if let Some(group) = &message.group_id
            && let Some(members) = self.groups.get_mut(group)
        {
            members.remove(&arrival);
            if members.is_empty() {
                self.groups.remove(group);
            }
        }
        Some(message)
    }

    /// The id of the message that makes a send with `dedup_id` a duplicate.
    pub fn duplicate_of(&self, dedup_id: &str, now: u64) -> Option<(u64, Option<u64>)> {
        let entry = self.dedup.get(dedup_id)?;
        (now - entry.accepted_at < DEDUP_WINDOW)
            .then_some((entry.message_id, entry.sequence_number))
    }

    /// Opens a deduplication window for an accepted send.
    pub fn record_send(
        &mut self,
        dedup_id: String,
        now: u64,
        message_id: u64,
        sequence_number: Option<u64>,
    ) {
        self.dedup.insert(
            dedup_id,
            DedupEntry {
                accepted_at: now,
                message_id,
                sequence_number,
            },
        );
    }

    /// Delivers up to `max` messages (SPEC section 6, *ReceiveMessage*).
    /// `next_handle` is the process-wide receipt handle counter. The queue
    /// must have been advanced to `now`.
    pub fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility_timeout: u64,
        next_handle: &mut u64,
    ) -> Received {
        let dead_letter_limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut received = Received::default();
        // Visible messages are examined once each in arrival order. Removing
        // or rescheduling entries as we go is fine because the cursor only
        // moves forward.
        let mut cursor = 0;
        while received.delivered.len() < max {
            let Some(&arrival) = self.ready.range(cursor..).next() else {
                break;
            };
            cursor = arrival + 1;
            let message = &self.messages[&arrival];
            // A message may only leave its FIFO group first-in-first-out.
            let blocked = message
                .group_id
                .as_ref()
                .and_then(|g| self.groups[g].first())
                .is_some_and(|&first| first != arrival);
            if blocked {
                continue;
            }
            if dead_letter_limit.is_some_and(|limit| message.receive_count >= limit) {
                let dead = self.remove(arrival).expect("ready messages exist");
                received.dead.push(dead.reset_for_dead_letter());
                continue;
            }

            self.ready.remove(&arrival);
            let message = self
                .messages
                .get_mut(&arrival)
                .expect("ready messages exist");
            message.receive_count += 1;
            message.first_received_at.get_or_insert(now);
            message.invisible_until = now + visibility_timeout;
            let handle = *next_handle;
            *next_handle += 1;
            if let Some(previous) = message.handle.replace(handle) {
                self.handles.remove(&previous);
            }
            self.handles.insert(handle, arrival);
            let visible_from = message.visible_from();
            received.delivered.push(message.clone());
            self.schedule(arrival, visible_from, now);
        }
        received
    }

    /// The arrival number of the message for which `handle` is the latest
    /// handle, provided it is in flight at `now`.
    fn in_flight_by_handle(&self, handle: u64, now: u64) -> Option<u64> {
        let arrival = *self.handles.get(&handle)?;
        let message = self.messages.get(&arrival)?;
        (message.handle == Some(handle) && message.state(now) == State::InFlight).then_some(arrival)
    }

    /// Removes the in-flight message `handle` refers to. Returns false if the
    /// handle is not usable.
    pub fn delete_by_handle(&mut self, handle: u64, now: u64) -> bool {
        match self.in_flight_by_handle(handle, now) {
            Some(arrival) => self.remove(arrival).is_some(),
            None => false,
        }
    }

    /// Makes the message `handle` refers to invisible until `until`. Returns
    /// false if the handle is not usable.
    pub fn set_invisible_until(&mut self, handle: u64, now: u64, until: u64) -> bool {
        let Some(arrival) = self.in_flight_by_handle(handle, now) else {
            return false;
        };
        self.unschedule(arrival);
        let message = self.messages.get_mut(&arrival).expect("found by handle");
        message.invisible_until = until;
        let visible_from = message.visible_from();
        self.schedule(arrival, visible_from, now);
        true
    }
}
