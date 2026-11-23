//! A single queue: its attributes and its messages in arrival order.

use std::collections::{HashMap, HashSet};

use crate::attributes::Attributes;
use crate::params::is_name_byte;

pub const MAX_MESSAGES: usize = 100_000;
pub const MAX_QUEUES: usize = 1000;
const FIFO_SUFFIX: &str = ".fifo";

/// Checks the queue name syntax of SPEC section 4.
pub fn is_valid_queue_name(name: &str) -> bool {
    let (stem, max) = match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) => (stem, 75),
        None => (name, 80),
    };
    (1..=max).contains(&stem.len()) && stem.bytes().all(is_name_byte)
}

pub fn is_fifo_name(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: u64,
    pub body: String,
    pub sent_at: i64,
    pub receive_count: i64,
    pub first_received_at: Option<i64>,
    pub group_id: Option<String>,
    pub sequence: Option<u64>,
    /// Delayed while `now < delayed_until`.
    pub delayed_until: i64,
    /// In flight while `now < in_flight_until`.
    pub in_flight_until: i64,
    /// The latest receipt handle number issued for this message.
    pub receipt: Option<u64>,
}

impl Message {
    pub fn state(&self, now: i64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.in_flight_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// How long a deduplication id suppresses later sends, in seconds.
const DEDUP_WINDOW: i64 = 300;

/// The send that opened a deduplication window.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub at: i64,
    pub message_id: u64,
    pub sequence: u64,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    pub messages: Vec<Message>,
    /// The sequence number the next message added to this (FIFO) queue gets.
    next_sequence: u64,
    /// Deduplication ids outlive their messages, so they live beside them.
    dedup: HashMap<String, Accepted>,
    /// A lower bound of `sent_at` over `messages`, so retention only scans when
    /// something may actually have expired.
    oldest_sent_at: i64,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: Vec::new(),
            next_sequence: 1,
            dedup: HashMap::new(),
            oldest_sent_at: i64::MAX,
        }
    }

    /// Removes every message whose retention has run out at `now`.
    pub fn expire(&mut self, now: i64) {
        let retention = self.attributes.retention_seconds;
        if now < self.oldest_sent_at.saturating_add(retention) {
            return;
        }
        self.messages.retain(|m| now < m.sent_at + retention);
        self.oldest_sent_at = self
            .messages
            .iter()
            .map(|m| m.sent_at)
            .min()
            .unwrap_or(i64::MAX);
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    /// The send that still suppresses `dedup_id` at `now`, if any.
    pub fn duplicate_of(&self, dedup_id: &str, now: i64) -> Option<Accepted> {
        self.dedup
            .get(dedup_id)
            .copied()
            .filter(|a| now - a.at < DEDUP_WINDOW)
    }

    /// Records an accepted send so that later sends with `dedup_id` are duplicates.
    pub fn remember(&mut self, dedup_id: &str, accepted: Accepted) {
        self.dedup.insert(dedup_id.to_owned(), accepted);
    }

    /// Adds a message at the end of the queue, numbering it if the queue is FIFO.
    /// Returns the sequence number assigned, if any.
    pub fn push(&mut self, mut message: Message) -> Option<u64> {
        message.sequence = self.attributes.fifo.then(|| {
            let n = self.next_sequence;
            self.next_sequence += 1;
            n
        });
        self.oldest_sent_at = self.oldest_sent_at.min(message.sent_at);
        let sequence = message.sequence;
        self.messages.push(message);
        sequence
    }

    /// Examines the messages in arrival order and delivers up to `max` of them
    /// (SPEC section 6). Returns snapshots of the delivered messages and the
    /// messages that exceeded `MaxReceiveCount`, already reset for the
    /// dead-letter queue and removed from this one.
    ///
    /// `handles` is the process-wide count of receipt handles issued.
    pub fn receive(
        &mut self,
        now: i64,
        max: usize,
        visibility_timeout: i64,
        handles: &mut u64,
    ) -> (Vec<Message>, Vec<Message>) {
        let fifo = self.attributes.fifo;
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        // Groups that still have an earlier message in the queue.
        let mut blocked: HashSet<String> = HashSet::new();
        let mut delivered = Vec::new();
        let mut dead_indices = Vec::new();
        for (index, message) in self.messages.iter_mut().enumerate() {
            if delivered.len() >= max {
                break;
            }
            let group = message.group_id.as_ref().filter(|_| fifo);
            let eligible =
                message.state(now) == State::Visible && !group.is_some_and(|g| blocked.contains(g));
            if eligible && max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                dead_indices.push(index);
                continue;
            }
            if let Some(group) = group {
                blocked.insert(group.clone());
            }
            if !eligible {
                continue;
            }
            *handles += 1;
            message.receive_count += 1;
            message.first_received_at.get_or_insert(now);
            message.receipt = Some(*handles);
            message.in_flight_until = now + visibility_timeout;
            delivered.push(message.clone());
        }
        (delivered, self.take_dead(&dead_indices))
    }

    /// Removes the messages at `indices` (ascending) and resets them for their
    /// move to a dead-letter queue.
    fn take_dead(&mut self, indices: &[usize]) -> Vec<Message> {
        if indices.is_empty() {
            return Vec::new();
        }
        let mut dead = Vec::with_capacity(indices.len());
        let mut next = indices.iter().peekable();
        for (position, message) in std::mem::take(&mut self.messages).into_iter().enumerate() {
            if next.next_if_eq(&&position).is_some() {
                dead.push(Message {
                    receive_count: 0,
                    first_received_at: None,
                    delayed_until: 0,
                    in_flight_until: 0,
                    receipt: None,
                    ..message
                });
            } else {
                self.messages.push(message);
            }
        }
        dead
    }

    /// The position of the message whose latest handle is `handle`, if that
    /// message is in flight at `now`.
    pub fn find_in_flight(&self, handle: u64, now: i64) -> Option<usize> {
        self.messages
            .iter()
            .position(|m| m.receipt == Some(handle) && m.state(now) == State::InFlight)
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.oldest_sent_at = i64::MAX;
    }

    pub fn counts(&self, now: i64) -> Counts {
        let mut counts = Counts::default();
        for m in &self.messages {
            match m.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::AttributeSet;

    fn message(id: u64, sent_at: i64) -> Message {
        Message {
            id,
            body: "b".into(),
            sent_at,
            receive_count: 0,
            first_received_at: None,
            group_id: None,
            sequence: None,
            delayed_until: 0,
            in_flight_until: 0,
            receipt: None,
        }
    }

    fn queue_with(messages: Vec<Message>) -> Queue {
        let mut q = Queue::new(Attributes::from_set(&AttributeSet::default()));
        for m in messages {
            q.push(m);
        }
        q
    }

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        for ok in [
            "a",
            "A_z-9",
            long.as_str(),
            "q.fifo",
            &format!("{}.fifo", "a".repeat(75)),
        ] {
            assert!(is_valid_queue_name(ok), "{ok}");
        }
        let bad_fifo = format!("{}.fifo", "a".repeat(76));
        for bad in [
            "",
            too_long.as_str(),
            ".fifo",
            "a b",
            "a.b",
            "a.fifo.fifo",
            "é",
            "a.FIFO",
            bad_fifo.as_str(),
        ] {
            assert!(!is_valid_queue_name(bad), "{bad}");
        }
    }

    #[test]
    fn state_is_derived_from_the_clock() {
        let mut m = message(1, 0);
        m.delayed_until = 10;
        assert_eq!(m.state(9), State::Delayed);
        assert_eq!(m.state(10), State::Visible);
        m.in_flight_until = 20;
        assert_eq!(m.state(10), State::InFlight);
        assert_eq!(m.state(20), State::Visible);
    }

    #[test]
    fn expire_removes_messages_at_exactly_their_deadline() {
        let mut q = queue_with(vec![message(1, 0), message(2, 100)]);
        q.attributes.retention_seconds = 100;
        q.expire(99);
        assert_eq!(q.messages.len(), 2);
        q.expire(100);
        assert_eq!(q.messages.iter().map(|m| m.id).collect::<Vec<_>>(), [2]);
        q.expire(200);
        assert!(q.messages.is_empty());
    }
}
