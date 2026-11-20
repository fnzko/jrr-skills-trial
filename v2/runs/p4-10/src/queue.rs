//! One queue: its configuration and the messages it holds.
//!
//! Messages are stored in arrival order, keyed by an arrival number that is
//! never reused within the queue. A second index, ordered by send time,
//! lets retention find expired messages without scanning the queue. The
//! two indexes are only changed through [`Queue::push`], [`Queue::remove`]
//! and [`Queue::purge`], so they cannot drift apart.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::QueueAttributes;

/// Section 6: a repeated FIFO send within this many seconds is a duplicate.
pub const DEDUP_WINDOW_SECONDS: u64 = 300;

/// The state of a message at some instant (section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

/// A stored message. Times are clock values in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The number of the `m-<n>` message id.
    pub id: u64,
    pub body: String,
    pub group_id: Option<String>,
    /// Set by [`Queue::push`] when the queue is FIFO.
    pub sequence_number: Option<u64>,
    pub sent_at: u64,
    /// The message is delayed while `now < delayed_until`.
    pub delayed_until: u64,
    /// The message is in flight while `now < invisible_until`.
    pub invisible_until: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    /// The latest receipt number issued for this message, if any.
    pub receipt: Option<u64>,
}

impl Message {
    /// A message just accepted by a send, with no delivery history.
    pub fn new(id: u64, body: String, group_id: Option<String>, sent_at: u64, delay: u64) -> Self {
        Message {
            id,
            body,
            group_id,
            sequence_number: None,
            sent_at,
            delayed_until: sent_at + delay,
            invisible_until: 0,
            receive_count: 0,
            first_received_at: None,
            receipt: None,
        }
    }

    /// The message as a dead-letter move leaves it: its delivery history is
    /// cleared, it has no delay, and it is visible at once. Its id, body,
    /// send time and group are kept.
    pub fn into_dead_letter(mut self) -> Self {
        self.sequence_number = None;
        self.delayed_until = 0;
        self.invisible_until = 0;
        self.receive_count = 0;
        self.first_received_at = None;
        self.receipt = None;
        self
    }

    pub fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.invisible_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// How many messages of a queue are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

/// What a send was accepted as, or what a duplicate send refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: u64,
    pub sequence_number: Option<u64>,
}

/// What ReceiveMessage does with a message it examines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Examined {
    Deliver,
    MoveToDeadLetter,
}

/// The record of the last accepted send with a given deduplication id.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    sent_at: u64,
    sent: Sent,
}

#[derive(Debug)]
pub struct Queue {
    /// Identifies this queue for as long as the process runs. A queue that
    /// is deleted and created again under the same name gets a new uid.
    pub uid: u64,
    pub attributes: QueueAttributes,
    /// Messages keyed by arrival number, in arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, arrival number)` for every stored message.
    by_sent_at: BTreeSet<(u64, u64)>,
    next_arrival: u64,
    /// Next `SequenceNumber` of a FIFO queue. Starts at 1.
    next_sequence: u64,
    /// Deduplication ids are per queue and outlive their messages.
    dedup: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(uid: u64, attributes: QueueAttributes) -> Self {
        Queue {
            uid,
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            next_arrival: 0,
            next_sequence: 1,
            dedup: HashMap::new(),
        }
    }

    /// The number of stored messages, whatever their state.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Stores `message` at the end of the queue. A FIFO queue gives it the
    /// next sequence number, which is returned.
    pub fn push(&mut self, mut message: Message) -> Option<u64> {
        if self.attributes.fifo {
            message.sequence_number = Some(self.next_sequence);
            self.next_sequence += 1;
        }
        let sequence_number = message.sequence_number;
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
        sequence_number
    }

    /// Accepts a send: stores the message and, if it has a deduplication id,
    /// remembers the send for the duplicate window.
    pub fn send(&mut self, message: Message, dedup_id: Option<String>) -> Sent {
        let sent_at = message.sent_at;
        let message_id = message.id;
        let sequence_number = self.push(message);
        let sent = Sent {
            message_id,
            sequence_number,
        };
        if let Some(dedup_id) = dedup_id {
            self.dedup.insert(dedup_id, Accepted { sent_at, sent });
        }
        sent
    }

    /// The earlier accepted send with this deduplication id, if `now` is
    /// still inside its window. A duplicate does not extend the window.
    pub fn duplicate_of(&self, dedup_id: &str, now: u64) -> Option<Sent> {
        self.dedup
            .get(dedup_id)
            .filter(|accepted| now - accepted.sent_at < DEDUP_WINDOW_SECONDS)
            .map(|accepted| accepted.sent)
    }

    pub fn message(&self, arrival: u64) -> Option<&Message> {
        self.messages.get(&arrival)
    }

    pub fn message_mut(&mut self, arrival: u64) -> Option<&mut Message> {
        self.messages.get_mut(&arrival)
    }

    /// Removes a message from the queue and returns it.
    pub fn take(&mut self, arrival: u64) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        Some(message)
    }

    /// Decides what ReceiveMessage does, without changing anything. Returns
    /// the examined messages in arrival order, with what happens to each.
    ///
    /// A message is examined unless `max_messages` deliveries have already
    /// been planned. It is blocked if it is not visible, or, in a FIFO
    /// queue, if an earlier message of its group is still in the queue.
    /// A visible message whose `ReceiveCount` has reached `max_receive_count`
    /// is moved instead of delivered and does not count toward the limit.
    pub fn plan_receive(
        &self,
        now: u64,
        max_messages: usize,
        max_receive_count: Option<u64>,
    ) -> Vec<(u64, Examined)> {
        let mut blocked_groups: HashSet<&str> = HashSet::new();
        let mut plan = Vec::new();
        let mut delivered = 0;
        for (&arrival, message) in &self.messages {
            if delivered == max_messages {
                break;
            }
            let group = if self.attributes.fifo {
                message.group_id.as_deref()
            } else {
                None
            };
            if group.is_some_and(|group| blocked_groups.contains(group)) {
                continue;
            }
            if message.state(now) != State::Visible {
                blocked_groups.extend(group);
                continue;
            }
            if max_receive_count.is_some_and(|max| message.receive_count >= max) {
                plan.push((arrival, Examined::MoveToDeadLetter));
                continue;
            }
            blocked_groups.extend(group);
            plan.push((arrival, Examined::Deliver));
            delivered += 1;
        }
        plan
    }

    /// Removes every message whose retention has run out at `now`:
    /// `now >= sent_at + RetentionSeconds`.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if sent_at.saturating_add(retention) > now {
                break;
            }
            self.take(arrival);
        }
    }

    /// Removes every message, whatever its state. Sequence numbers and
    /// deduplication records carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                State::Delayed => counts.delayed += 1,
                State::InFlight => counts.in_flight += 1,
                State::Visible => counts.visible += 1,
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u64, sent_at: u64) -> Message {
        Message::new(id, format!("body {id}"), None, sent_at, 0)
    }

    #[test]
    fn a_message_is_delayed_then_in_flight_then_visible() {
        let message = Message {
            delayed_until: 10,
            invisible_until: 20,
            ..message(1, 0)
        };
        assert_eq!(message.state(9), State::Delayed);
        assert_eq!(message.state(10), State::InFlight);
        assert_eq!(message.state(19), State::InFlight);
        assert_eq!(message.state(20), State::Visible);
    }

    #[test]
    fn an_empty_queue_has_no_messages_in_any_state() {
        let queue = Queue::new(0, QueueAttributes::default());
        assert_eq!(queue.counts(0), Counts::default());
    }

    #[test]
    fn expire_removes_messages_at_exactly_sent_at_plus_retention() {
        let mut queue = Queue::new(
            0,
            QueueAttributes {
                retention_seconds: 60,
                ..QueueAttributes::default()
            },
        );
        queue.push(message(1, 0));
        queue.push(message(2, 10));
        queue.expire(59);
        assert_eq!(queue.message_count(), 2);
        queue.expire(60);
        assert_eq!(queue.message_count(), 1);
        queue.expire(70);
        assert_eq!(queue.message_count(), 0);
    }

    #[test]
    fn fifo_queues_number_their_messages_from_one() {
        let mut queue = Queue::new(
            0,
            QueueAttributes {
                fifo: true,
                ..QueueAttributes::default()
            },
        );
        assert_eq!(queue.push(message(1, 0)), Some(1));
        assert_eq!(queue.push(message(2, 0)), Some(2));
    }

    #[test]
    fn standard_queues_have_no_sequence_numbers() {
        let mut queue = Queue::new(0, QueueAttributes::default());
        assert_eq!(queue.push(message(1, 0)), None);
    }

    #[test]
    fn a_duplicate_is_recognised_until_the_window_closes() {
        let mut queue = Queue::new(
            0,
            QueueAttributes {
                fifo: true,
                ..QueueAttributes::default()
            },
        );
        let sent = queue.send(message(1, 5), Some("k".to_owned()));
        assert_eq!(
            queue.duplicate_of("k", 5 + DEDUP_WINDOW_SECONDS - 1),
            Some(sent)
        );
        assert_eq!(queue.duplicate_of("k", 5 + DEDUP_WINDOW_SECONDS), None);
        assert_eq!(queue.duplicate_of("other", 5), None);
    }

    #[test]
    fn purge_keeps_sequence_numbers_and_deduplication() {
        let mut queue = Queue::new(
            0,
            QueueAttributes {
                fifo: true,
                ..QueueAttributes::default()
            },
        );
        queue.send(message(1, 0), Some("k".to_owned()));
        queue.purge();
        assert_eq!(queue.message_count(), 0);
        assert!(queue.duplicate_of("k", 0).is_some());
        assert_eq!(queue.push(message(2, 0)), Some(2));
    }
}
