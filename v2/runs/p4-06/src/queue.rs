//! One queue's storage and the indexes that keep each command cheap.
//!
//! Messages are kept in a map keyed by arrival order within the queue. Three
//! indexes sit beside it:
//!
//! - `expiries` orders messages by the moment they are removed for good, so
//!   sweeping touches only messages that are actually gone;
//! - `handles` maps each message's latest receipt handle to its key, so a
//!   handle resolves without a scan;
//! - `dedup` remembers deduplication windows, which outlive their messages.
//!
//! Nothing here reads a clock: every method takes `now` from the caller.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use crate::attributes::Attributes;

/// Messages a queue holds, counting every state (SPEC section 6).
pub const MAX_MESSAGES: usize = 100_000;
/// Seconds during which a repeated deduplication id is a duplicate (SPEC section 6).
pub const DEDUPLICATION_WINDOW: u64 = 300;

/// The state a message is in at a given moment (SPEC section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

/// The latest delivery of a message. It is in flight while `now < visible_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receipt {
    pub handle: u64,
    pub visible_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The `n` of `m-<n>`.
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    /// The message is delayed while `now < delayed_until`.
    pub delayed_until: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
    pub receipt: Option<Receipt>,
}

impl Message {
    pub fn new(id: u64, body: String, sent_at: u64, delay: u64, group_id: Option<String>) -> Self {
        Message {
            id,
            body,
            sent_at,
            delayed_until: sent_at.saturating_add(delay),
            receive_count: 0,
            first_received_at: None,
            group_id,
            sequence_number: None,
            receipt: None,
        }
    }

    pub fn in_flight(&self, now: u64) -> bool {
        self.receipt.is_some_and(|receipt| now < receipt.visible_at)
    }

    pub fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.in_flight(now) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// The moment a message sent at `sent_at` is removed for good.
fn expiry(sent_at: u64, retention: u64) -> u64 {
    sent_at.saturating_add(retention)
}

/// What a deduplication window remembers about the send that opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupEntry {
    pub opened_at: u64,
    pub message_id: u64,
    pub sequence_number: u64,
}

/// Message counts per state, as `GetQueueAttributes` reports them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<u64, Message>,
    expiries: BTreeSet<(u64, u64)>,
    handles: HashMap<u64, u64>,
    dedup: HashMap<String, DedupEntry>,
    next_arrival: u64,
    next_sequence_number: u64,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Queue {
            attributes,
            messages: BTreeMap::new(),
            expiries: BTreeSet::new(),
            handles: HashMap::new(),
            dedup: HashMap::new(),
            next_arrival: 0,
            next_sequence_number: 1,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Number of stored messages. Call [`sweep`](Self::sweep) first if expiry matters.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Swaps in new attributes. A new retention moves every message's expiry,
    /// so the expiry index is rebuilt before sweeping.
    pub fn replace_attributes(&mut self, attributes: Attributes, now: u64) {
        self.attributes = attributes;
        let retention = self.attributes.retention_seconds;
        self.expiries = self
            .messages
            .iter()
            .map(|(key, message)| (expiry(message.sent_at, retention), *key))
            .collect();
        self.sweep(now);
    }

    /// Removes every message whose retention has ended by `now`.
    pub fn sweep(&mut self, now: u64) {
        while let Some(&(expires_at, key)) = self.expiries.first() {
            if expires_at > now {
                break;
            }
            self.remove(key);
        }
    }

    /// The sequence number for the next message added to this queue, or `None`
    /// for a standard queue. Each call consumes one number.
    pub fn take_sequence_number(&mut self) -> Option<u64> {
        if !self.attributes.fifo {
            return None;
        }
        let number = self.next_sequence_number;
        self.next_sequence_number += 1;
        Some(number)
    }

    /// Adds a message at the end of the queue and returns its arrival key.
    pub fn push(&mut self, message: Message) -> u64 {
        let key = self.next_arrival;
        self.next_arrival += 1;
        let expires_at = expiry(message.sent_at, self.attributes.retention_seconds);
        self.expiries.insert((expires_at, key));
        self.messages.insert(key, message);
        key
    }

    /// Adds a message moved here from a dead-letter source. It starts over as
    /// visible with a zero receive count. A message already past this queue's
    /// retention is dropped on arrival and takes no sequence number.
    pub fn accept_moved(&mut self, mut message: Message, now: u64) {
        if now >= expiry(message.sent_at, self.attributes.retention_seconds) {
            return;
        }
        message.receive_count = 0;
        message.first_received_at = None;
        message.delayed_until = 0;
        message.receipt = None;
        message.sequence_number = self.take_sequence_number();
        self.push(message);
    }

    /// Removes and returns the message stored under `key`.
    pub fn remove(&mut self, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        let expires_at = expiry(message.sent_at, self.attributes.retention_seconds);
        self.expiries.remove(&(expires_at, key));
        if let Some(receipt) = message.receipt {
            self.handles.remove(&receipt.handle);
        }
        Some(message)
    }

    /// Removes every message. Deduplication windows and sequence numbers carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.expiries.clear();
        self.handles.clear();
    }

    pub fn message(&self, key: u64) -> Option<&Message> {
        self.messages.get(&key)
    }

    /// The first message after `after` in arrival order, or the first one overall.
    pub fn next_after(&self, after: Option<u64>) -> Option<(u64, &Message)> {
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        self.messages
            .range((lower, Bound::Unbounded))
            .next()
            .map(|(key, message)| (*key, message))
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// The message whose latest handle is `handle`, provided it is in flight now.
    /// Any other handle, including one whose visibility has ended, yields `None`.
    pub fn key_for_handle(&self, handle: u64, now: u64) -> Option<u64> {
        let key = *self.handles.get(&handle)?;
        let message = self.messages.get(&key)?;
        let current = message.receipt.map(|receipt| receipt.handle);
        (current == Some(handle) && message.in_flight(now)).then_some(key)
    }

    /// Records a delivery: counts it, issues `handle`, and keeps the message
    /// invisible for `visibility_timeout` seconds. A key that is not stored is ignored.
    pub fn deliver(&mut self, key: u64, handle: u64, now: u64, visibility_timeout: u64) {
        let Some(message) = self.messages.get_mut(&key) else {
            return;
        };
        if let Some(previous) = message.receipt {
            self.handles.remove(&previous.handle);
        }
        message.receive_count += 1;
        message.first_received_at.get_or_insert(now);
        message.receipt = Some(Receipt {
            handle,
            visible_at: now.saturating_add(visibility_timeout),
        });
        self.handles.insert(handle, key);
    }

    /// Replaces the end of the message's visibility timeout with `now + timeout`.
    pub fn change_visibility(&mut self, key: u64, now: u64, timeout: u64) {
        if let Some(receipt) = self
            .messages
            .get_mut(&key)
            .and_then(|message| message.receipt.as_mut())
        {
            receipt.visible_at = now.saturating_add(timeout);
        }
    }

    /// The window that makes a send with this deduplication id a duplicate, if one is open.
    pub fn open_dedup_window(&self, dedup_id: &str, now: u64) -> Option<DedupEntry> {
        self.dedup
            .get(dedup_id)
            .copied()
            .filter(|entry| now.saturating_sub(entry.opened_at) < DEDUPLICATION_WINDOW)
    }

    pub fn record_dedup(&mut self, dedup_id: String, entry: DedupEntry) {
        self.dedup.insert(dedup_id, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> Queue {
        Queue::new(Attributes::default())
    }

    fn message(id: u64, sent_at: u64) -> Message {
        Message::new(id, format!("body-{id}"), sent_at, 0, None)
    }

    #[test]
    fn a_message_expires_exactly_at_sent_at_plus_retention() {
        let mut q = queue();
        q.push(message(1, 10));
        let retention = q.attributes().retention_seconds;

        q.sweep(10 + retention - 1);
        assert_eq!(q.len(), 1);
        q.sweep(10 + retention);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn lowering_retention_removes_at_once_and_raising_it_does_not_restore() {
        let mut q = queue();
        q.push(message(1, 100));

        let mut shorter = q.attributes().clone();
        shorter.retention_seconds = 60;
        q.replace_attributes(shorter, 170);
        assert_eq!(q.len(), 0);

        // The message is gone for good; a longer retention cannot bring it back.
        let mut longer = q.attributes().clone();
        longer.retention_seconds = 1_000_000;
        q.replace_attributes(longer, 171);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn a_handle_resolves_only_while_its_message_is_in_flight() {
        let mut q = queue();
        let key = q.push(message(1, 0));
        q.deliver(key, 7, 0, 30);

        assert_eq!(q.key_for_handle(7, 29), Some(key));
        assert_eq!(q.key_for_handle(7, 30), None, "visibility ended");
        assert_eq!(q.key_for_handle(8, 0), None, "never issued here");
    }

    #[test]
    fn a_redelivery_makes_the_older_handle_expired() {
        let mut q = queue();
        let key = q.push(message(1, 0));
        q.deliver(key, 1, 0, 0);
        q.deliver(key, 2, 5, 30);

        assert_eq!(q.key_for_handle(1, 5), None);
        assert_eq!(q.key_for_handle(2, 5), Some(key));
        assert_eq!(q.message(key).unwrap().receive_count, 2);
    }

    #[test]
    fn removing_a_message_retires_its_handle() {
        let mut q = queue();
        let key = q.push(message(1, 0));
        q.deliver(key, 1, 0, 30);
        q.remove(key);
        assert_eq!(q.key_for_handle(1, 0), None);
    }

    #[test]
    fn counts_split_messages_by_state() {
        let mut q = queue();
        q.push(message(1, 0));
        let flying = q.push(message(2, 0));
        q.push(Message::new(3, "d".into(), 0, 50, None));
        q.deliver(flying, 1, 0, 30);

        assert_eq!(
            q.counts(0),
            Counts {
                visible: 1,
                in_flight: 1,
                delayed: 1
            }
        );
        assert_eq!(
            q.counts(40),
            Counts {
                visible: 2,
                in_flight: 0,
                delayed: 1
            }
        );
    }

    #[test]
    fn purge_keeps_sequence_numbers_and_dedup_windows() {
        let mut q = queue();
        q.record_dedup(
            "d".into(),
            DedupEntry {
                opened_at: 0,
                message_id: 1,
                sequence_number: 1,
            },
        );
        q.push(message(1, 0));
        q.purge();
        assert_eq!(q.len(), 0);
        assert!(q.open_dedup_window("d", 1).is_some());
    }

    #[test]
    fn a_dedup_window_closes_at_300_seconds() {
        let mut q = queue();
        q.record_dedup(
            "d".into(),
            DedupEntry {
                opened_at: 10,
                message_id: 1,
                sequence_number: 1,
            },
        );
        assert!(q.open_dedup_window("d", 309).is_some());
        assert!(q.open_dedup_window("d", 310).is_none());
    }

    #[test]
    fn a_moved_message_that_is_too_old_is_dropped_without_a_sequence_number() {
        let mut fifo = Queue::new(Attributes {
            fifo: true,
            ..Attributes::default()
        });
        let old = Message::new(1, "old".into(), 0, 0, Some("g".into()));
        fifo.accept_moved(old, 345_600);
        assert_eq!(fifo.len(), 0);

        let fresh = Message::new(2, "fresh".into(), 345_600, 0, Some("g".into()));
        fifo.accept_moved(fresh, 345_600);
        let (key, _) = fifo.next_after(None).unwrap();
        assert_eq!(fifo.message(key).unwrap().sequence_number, Some(1));
    }
}
