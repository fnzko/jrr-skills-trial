//! The messages of one queue and the rules that decide which of them can be received.
//!
//! Every method takes the current time `now`. A message is hidden while
//! `now < hidden_until`; this one field covers both a send delay and a
//! visibility timeout. Two indexes keep receiving cheap on large queues:
//! `ready` holds the visible messages that are eligible for delivery, and
//! `hidden` is a min-heap that tells when a hidden message may join `ready`.
//!
//! Retention is applied lazily by [`Queue::expire`]; the owner calls it before
//! it reads or changes a queue, which is equivalent to removing messages at
//! the exact moment they expire.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::ops::Bound;

use serde::Serialize;

use crate::attributes::Attributes;
use crate::domain::{GroupId, MessageId, ReceiptHandle, SequenceNumber};

const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: u64 = 300;
const INDEXED: &str = "the indexes of a queue only name messages it holds";

/// A message held by a queue.
#[derive(Debug, Clone)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    group: Option<GroupId>,
    hidden_until: u64,
    /// Handle of the latest delivery.
    handle: Option<ReceiptHandle>,
}

impl Message {
    /// A message sent at `now` that stays hidden for `delay` seconds.
    pub fn new(id: MessageId, body: String, group: Option<GroupId>, now: u64, delay: u64) -> Self {
        Self {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            hidden_until: now + delay,
            handle: None,
        }
    }

    /// The message as it enters a dead-letter queue: never received, visible
    /// at once, with its original send time.
    pub fn restarted(self, now: u64) -> Self {
        Self {
            receive_count: 0,
            first_received_at: None,
            hidden_until: now,
            handle: None,
            ..self
        }
    }
}

/// A message as handed to a client by one delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

/// What one receive call did.
#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    /// Messages that left the queue for its dead-letter queue, in the order
    /// they were found.
    pub dead_lettered: Vec<Message>,
}

/// How many messages are in each state.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

/// The receipt handle does not belong to a message that is in flight.
#[derive(Debug, PartialEq, Eq)]
pub struct NotInFlight;

/// A FIFO send accepted earlier, remembered for deduplication.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: u64,
    id: MessageId,
    sequence: SequenceNumber,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    /// Messages in arrival order.
    messages: BTreeMap<SequenceNumber, Message>,
    /// Number of messages ever added; the last one's sequence number.
    added: u64,
    /// Messages by send time, to find the ones that outlived the retention.
    by_sent: BTreeSet<(u64, SequenceNumber)>,
    /// Visible messages that are not blocked by an earlier message of their group.
    ready: BTreeSet<SequenceNumber>,
    /// `(hidden_until, message)` for hidden messages. An entry is stale when
    /// the message is gone or its `hidden_until` differs; stale entries are
    /// skipped when they are popped.
    hidden: BinaryHeap<Reverse<(u64, SequenceNumber)>>,
    /// Members of each group in arrival order; the first one is the only
    /// member that may be delivered.
    groups: HashMap<GroupId, BTreeSet<SequenceNumber>>,
    /// The latest handle of each delivered message.
    handles: HashMap<ReceiptHandle, SequenceNumber>,
    /// Deduplication id to the send accepted under it. It outlives the message.
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            added: 0,
            by_sent: BTreeSet::new(),
            ready: BTreeSet::new(),
            hidden: BinaryHeap::new(),
            groups: HashMap::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn set_attributes(&mut self, attributes: Attributes) {
        self.attributes = attributes;
    }

    pub fn clear_dead_letter(&mut self) {
        self.attributes.dead_letter = None;
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.fifo
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    /// Removes the messages whose retention has run out.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, sequence)) = self.by_sent.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(sequence, now);
        }
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            if now >= message.hidden_until {
                counts.visible += 1;
            } else if message.receive_count == 0 {
                counts.delayed += 1;
            } else {
                counts.in_flight += 1;
            }
        }
        counts
    }

    /// The message accepted under `key` less than the deduplication window ago.
    pub fn duplicate_of(&self, key: &str, now: u64) -> Option<(MessageId, SequenceNumber)> {
        let accepted = self.deduplication.get(key)?;
        (now < accepted.at + DEDUPLICATION_WINDOW).then_some((accepted.id, accepted.sequence))
    }

    /// Remembers that the message `id` was accepted under `key` at `now`.
    pub fn record_deduplication(
        &mut self,
        key: String,
        id: MessageId,
        sequence: SequenceNumber,
        now: u64,
    ) {
        let accepted = Accepted {
            at: now,
            id,
            sequence,
        };
        self.deduplication.insert(key, accepted);
    }

    /// Adds a message at the end of the queue, whatever the queue holds.
    pub fn push(&mut self, message: Message, now: u64) -> SequenceNumber {
        self.added += 1;
        let sequence = SequenceNumber::new(self.added);
        self.by_sent.insert((message.sent_at, sequence));
        if let Some(group) = &message.group {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(sequence);
        }
        self.messages.insert(sequence, message);
        self.schedule(sequence, now);
        sequence
    }

    /// Delivers up to `max` messages, each hidden for `visibility` seconds,
    /// and takes out the messages that exhausted their receive count.
    ///
    /// `issue_handle` supplies the receipt handle of each delivery.
    pub fn receive(
        &mut self,
        max: usize,
        visibility: u64,
        now: u64,
        mut issue_handle: impl FnMut() -> ReceiptHandle,
    ) -> Received {
        self.release_due(now);
        let mut received = Received::default();
        let mut cursor = Bound::Unbounded;
        while received.delivered.len() < max {
            let Some(&sequence) = self.ready.range((cursor, Bound::Unbounded)).next() else {
                break;
            };
            cursor = Bound::Excluded(sequence);

            let exhausted = self
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| {
                    self.message(sequence).receive_count >= dead_letter.max_receive_count
                });
            if exhausted {
                received
                    .dead_lettered
                    .push(self.remove(sequence, now).expect(INDEXED));
                continue;
            }

            let handle = issue_handle();
            let message = self.messages.get_mut(&sequence).expect(INDEXED);
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.hidden_until = now + visibility;
            let previous = message.handle.replace(handle);
            received.delivered.push(Delivery {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.group.as_ref().map(|g| g.as_str().to_owned()),
                sequence_number: self.attributes.fifo.then_some(sequence),
            });
            if let Some(previous) = previous {
                self.handles.remove(&previous);
            }
            self.handles.insert(handle, sequence);
            self.schedule(sequence, now);
        }
        received
    }

    /// Removes the message that `handle` was issued for.
    pub fn delete(&mut self, handle: ReceiptHandle, now: u64) -> Result<(), NotInFlight> {
        let sequence = self.in_flight(handle, now)?;
        self.remove(sequence, now);
        Ok(())
    }

    /// Makes the message that `handle` was issued for hidden for `timeout`
    /// seconds from now.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: u64,
        now: u64,
    ) -> Result<(), NotInFlight> {
        let sequence = self.in_flight(handle, now)?;
        self.messages
            .get_mut(&sequence)
            .expect(INDEXED)
            .hidden_until = now + timeout;
        self.schedule(sequence, now);
        Ok(())
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent.clear();
        self.ready.clear();
        self.hidden.clear();
        self.groups.clear();
        self.handles.clear();
    }

    fn message(&self, sequence: SequenceNumber) -> &Message {
        self.messages.get(&sequence).expect(INDEXED)
    }

    fn in_flight(&self, handle: ReceiptHandle, now: u64) -> Result<SequenceNumber, NotInFlight> {
        let &sequence = self.handles.get(&handle).ok_or(NotInFlight)?;
        if self.message(sequence).hidden_until > now {
            Ok(sequence)
        } else {
            Err(NotInFlight)
        }
    }

    /// Files a message whose `hidden_until` just changed (or that was just
    /// added) under `ready` or `hidden`.
    fn schedule(&mut self, sequence: SequenceNumber, now: u64) {
        self.ready.remove(&sequence);
        let hidden_until = self.message(sequence).hidden_until;
        if hidden_until <= now {
            self.ready_if_group_head(sequence);
        } else {
            self.hidden.push(Reverse((hidden_until, sequence)));
        }
    }

    /// Adds a visible message to `ready` unless an earlier message of its
    /// group is still in the queue. A blocked message joins `ready` when the
    /// messages before it are removed.
    fn ready_if_group_head(&mut self, sequence: SequenceNumber) {
        let is_head = match &self.message(sequence).group {
            Some(group) => self.groups.get(group).and_then(BTreeSet::first) == Some(&sequence),
            None => true,
        };
        if is_head {
            self.ready.insert(sequence);
        }
    }

    /// Moves the hidden messages whose time has come to `ready`.
    fn release_due(&mut self, now: u64) {
        while let Some(&Reverse((until, sequence))) = self.hidden.peek() {
            if until > now {
                break;
            }
            self.hidden.pop();
            let current = self
                .messages
                .get(&sequence)
                .is_some_and(|message| message.hidden_until == until);
            if current {
                self.ready_if_group_head(sequence);
            }
        }
    }

    fn remove(&mut self, sequence: SequenceNumber, now: u64) -> Option<Message> {
        let message = self.messages.remove(&sequence)?;
        self.by_sent.remove(&(message.sent_at, sequence));
        self.ready.remove(&sequence);
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        if let Some(group) = &message.group {
            let members = self.groups.get_mut(group).expect(INDEXED);
            let was_head = members.first() == Some(&sequence);
            members.remove(&sequence);
            let next_head = members.first().copied();
            if members.is_empty() {
                self.groups.remove(group);
            }
            if let Some(next) = next_head.filter(|_| was_head)
                && self.message(next).hidden_until <= now
            {
                self.ready.insert(next);
            }
        }
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::DeadLetter;

    fn queue(fifo: bool) -> Queue {
        Queue::new(Attributes {
            fifo,
            ..Attributes::default()
        })
    }

    fn message(id: u64, group: Option<&str>, now: u64, delay: u64) -> Message {
        let group = group.map(|g| g.parse().unwrap());
        Message::new(MessageId::new(id), format!("body-{id}"), group, now, delay)
    }

    /// Receives with handles numbered from `next_handle`.
    fn receive(
        queue: &mut Queue,
        max: usize,
        visibility: u64,
        now: u64,
        next_handle: &mut u64,
    ) -> Received {
        queue.receive(max, visibility, now, || {
            *next_handle += 1;
            ReceiptHandle::new(*next_handle)
        })
    }

    fn ids(received: &Received) -> Vec<String> {
        received
            .delivered
            .iter()
            .map(|d| d.message_id.to_string())
            .collect()
    }

    #[test]
    fn delayed_message_is_delivered_once_its_delay_has_passed() {
        let mut queue = queue(false);
        queue.push(message(1, None, 0, 10), 0);
        let mut handle = 0;
        assert_eq!(
            ids(&receive(&mut queue, 1, 30, 9, &mut handle)),
            Vec::<String>::new()
        );
        assert_eq!(
            queue.counts(9),
            Counts {
                visible: 0,
                in_flight: 0,
                delayed: 1
            }
        );
        assert_eq!(ids(&receive(&mut queue, 1, 30, 10, &mut handle)), ["m-1"]);
        assert_eq!(
            queue.counts(10),
            Counts {
                visible: 0,
                in_flight: 1,
                delayed: 0
            }
        );
    }

    #[test]
    fn message_whose_timeout_ended_keeps_its_place_in_arrival_order() {
        let mut queue = queue(false);
        queue.push(message(1, None, 0, 0), 0);
        queue.push(message(2, None, 0, 0), 0);
        let mut handle = 0;
        assert_eq!(ids(&receive(&mut queue, 1, 5, 0, &mut handle)), ["m-1"]);
        assert_eq!(ids(&receive(&mut queue, 1, 5, 1, &mut handle)), ["m-2"]);
        assert_eq!(ids(&receive(&mut queue, 2, 5, 5, &mut handle)), ["m-1"]);
        assert_eq!(ids(&receive(&mut queue, 2, 5, 6, &mut handle)), ["m-2"]);
    }

    #[test]
    fn fifo_group_is_blocked_until_its_first_message_is_deleted() {
        let mut queue = queue(true);
        queue.push(message(1, Some("a"), 0, 0), 0);
        queue.push(message(2, Some("a"), 0, 0), 0);
        queue.push(message(3, Some("b"), 0, 0), 0);
        let mut handle = 0;
        let first = receive(&mut queue, 10, 30, 0, &mut handle);
        assert_eq!(ids(&first), ["m-1", "m-3"]);
        assert_eq!(
            ids(&receive(&mut queue, 10, 30, 1, &mut handle)),
            Vec::<String>::new()
        );
        queue.delete(first.delivered[0].receipt_handle, 2).unwrap();
        assert_eq!(ids(&receive(&mut queue, 10, 30, 2, &mut handle)), ["m-2"]);
    }

    #[test]
    fn fifo_group_waits_for_a_delayed_successor_of_a_removed_head() {
        let mut queue = queue(true);
        queue.push(message(1, Some("a"), 0, 0), 0);
        queue.push(message(2, Some("a"), 0, 50), 0);
        let mut handle = 0;
        let first = receive(&mut queue, 1, 30, 0, &mut handle);
        queue.delete(first.delivered[0].receipt_handle, 1).unwrap();
        assert_eq!(
            ids(&receive(&mut queue, 1, 30, 49, &mut handle)),
            Vec::<String>::new()
        );
        assert_eq!(ids(&receive(&mut queue, 1, 30, 50, &mut handle)), ["m-2"]);
    }

    #[test]
    fn stale_handles_cannot_delete() {
        let mut queue = queue(false);
        queue.push(message(1, None, 0, 0), 0);
        let mut handle = 0;
        let first = receive(&mut queue, 1, 5, 0, &mut handle)
            .delivered
            .remove(0);
        let second = receive(&mut queue, 1, 5, 5, &mut handle)
            .delivered
            .remove(0);
        assert_eq!(queue.delete(first.receipt_handle, 6), Err(NotInFlight));
        assert_eq!(queue.delete(second.receipt_handle, 10), Err(NotInFlight));
        assert_eq!(queue.delete(second.receipt_handle, 9), Ok(()));
        assert_eq!(queue.counts(9), Counts::default());
    }

    #[test]
    fn change_visibility_to_zero_makes_the_message_receivable_at_once() {
        let mut queue = queue(false);
        queue.push(message(1, None, 0, 0), 0);
        let mut handle = 0;
        let first = receive(&mut queue, 1, 100, 0, &mut handle)
            .delivered
            .remove(0);
        queue.change_visibility(first.receipt_handle, 0, 1).unwrap();
        assert_eq!(ids(&receive(&mut queue, 1, 100, 1, &mut handle)), ["m-1"]);
    }

    #[test]
    fn expire_removes_messages_that_outlived_the_retention() {
        let mut queue = Queue::new(Attributes {
            retention_seconds: 60,
            ..Attributes::default()
        });
        queue.push(message(1, None, 0, 0), 0);
        queue.push(message(2, None, 30, 0), 30);
        queue.expire(59);
        assert_eq!(queue.counts(59).visible, 2);
        queue.expire(60);
        assert_eq!(queue.counts(60).visible, 1);
        queue.expire(90);
        assert_eq!(queue.counts(90), Counts::default());
    }

    #[test]
    fn exhausted_messages_are_handed_back_for_the_dead_letter_queue() {
        let mut queue = Queue::new(Attributes {
            dead_letter: Some(DeadLetter {
                queue: "dlq".to_owned(),
                max_receive_count: 1,
            }),
            ..Attributes::default()
        });
        queue.push(message(1, None, 0, 0), 0);
        queue.push(message(2, None, 0, 0), 0);
        let mut handle = 0;
        assert_eq!(ids(&receive(&mut queue, 1, 0, 0, &mut handle)), ["m-1"]);
        let second = receive(&mut queue, 2, 0, 0, &mut handle);
        assert_eq!(ids(&second), ["m-2"]);
        assert_eq!(second.dead_lettered.len(), 1);
        assert_eq!(second.dead_lettered[0].id, MessageId::new(1));
        assert_eq!(second.dead_lettered[0].receive_count, 1);
        assert_eq!(queue.counts(0).visible, 1);
    }

    #[test]
    fn deduplication_window_is_300_seconds_from_the_accepted_send() {
        let mut queue = queue(true);
        let sequence = queue.push(message(1, Some("g"), 0, 0), 0);
        queue.record_deduplication("k".to_owned(), MessageId::new(1), sequence, 0);
        assert_eq!(
            queue.duplicate_of("k", 299),
            Some((MessageId::new(1), sequence))
        );
        assert_eq!(queue.duplicate_of("k", 300), None);
        assert_eq!(queue.duplicate_of("other", 0), None);
    }
}
