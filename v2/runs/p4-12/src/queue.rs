//! A single queue: its attributes, its messages and the rules of SPEC sections 4 and 6 that only
//! concern one queue.
//!
//! Message states depend on the clock. A queue does not read the clock itself; every method that
//! needs it takes `now`, and the owner must call [`Queue::advance`] whenever the clock moves so
//! that the indexes below stay in step with it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use serde::Serialize;

use crate::dedup::{Deduplicator, Original};
use crate::error::{Error, ErrorCode, Result};
use crate::request::{AttributeChanges, SendMessage};

/// The most messages one queue may hold, counting every state.
pub const MAX_MESSAGES: usize = 100_000;

/// Hands out the numbers behind process-wide identifiers (`m-<n>`, `r-<n>`), starting at 1.
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// The highest number handed out so far (0 if none).
    pub fn issued(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub dead_letter: Option<DeadLetterConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetterConfig {
    #[serde(rename = "DeadLetterQueue")]
    pub queue_name: String,
    pub max_receive_count: u64,
}

impl Default for QueueAttributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl QueueAttributes {
    /// Overwrites the attributes present in `changes`. A dead-letter configuration is only taken
    /// when both of its halves are present; callers reject requests that name just one.
    pub fn apply(&mut self, changes: &AttributeChanges) {
        let AttributeChanges {
            visibility_timeout,
            delay_seconds,
            retention_seconds,
            fifo,
            content_based_deduplication,
            dead_letter_queue,
            max_receive_count,
        } = changes;
        if let Some(v) = visibility_timeout {
            self.visibility_timeout = *v;
        }
        if let Some(v) = delay_seconds {
            self.delay_seconds = *v;
        }
        if let Some(v) = retention_seconds {
            self.retention_seconds = *v;
        }
        if let Some(v) = fifo {
            self.fifo = *v;
        }
        if let Some(v) = content_based_deduplication {
            self.content_based_deduplication = *v;
        }
        if let (Some(queue_name), Some(max_receive_count)) = (dead_letter_queue, max_receive_count)
        {
            self.dead_letter = Some(DeadLetterConfig {
                queue_name: queue_name.clone(),
                max_receive_count: *max_receive_count,
            });
        }
    }
}

/// How many messages of a queue are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone)]
pub struct Message {
    id: u64,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    fifo: Option<FifoFields>,
    /// The message is delayed or in flight while `now < available_at`, and visible after.
    available_at: u64,
    /// The latest receipt handle issued for this message in its current queue.
    receipt: Option<u64>,
}

#[derive(Debug, Clone)]
struct FifoFields {
    group_id: String,
    sequence_number: u64,
}

/// The result of an accepted (or duplicate) send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: u64,
    pub sequence_number: Option<u64>,
}

/// One message handed out by [`Queue::receive`], in the shape of the response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: String,
    pub receipt_handle: String,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that reached the queue's `MaxReceiveCount` and must move to its dead-letter queue,
    /// in the order they were taken out.
    pub dead_letters: Vec<Message>,
}

/// Position of a message in its queue's arrival order.
type Key = u64;

#[derive(Debug)]
pub struct Queue {
    attributes: QueueAttributes,
    messages: BTreeMap<Key, Message>,
    next_key: Key,
    last_sequence_number: u64,
    /// `(sent_at, key)` of every message, so retention can drop the oldest first. Arrival order is
    /// not enough: dead-letter moves append messages that were sent long ago.
    by_sent_at: BTreeSet<(u64, Key)>,
    /// `(available_at, key)` of every delayed or in-flight message.
    hidden: BTreeSet<(u64, Key)>,
    /// Keys of the messages a receive may take: visible and, in a FIFO queue, first of their group.
    eligible: BTreeSet<Key>,
    /// Keys of the messages of each FIFO group, in arrival order.
    groups: HashMap<String, BTreeSet<Key>>,
    /// Latest receipt handle of each message that has one, to the message's key.
    receipts: HashMap<u64, Key>,
    dedup: Deduplicator,
}

impl Queue {
    pub fn new(attributes: QueueAttributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_key: 0,
            last_sequence_number: 0,
            by_sent_at: BTreeSet::new(),
            hidden: BTreeSet::new(),
            eligible: BTreeSet::new(),
            groups: HashMap::new(),
            receipts: HashMap::new(),
            dedup: Deduplicator::default(),
        }
    }

    pub fn attributes(&self) -> &QueueAttributes {
        &self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.fifo
    }

    /// Applies new attribute values. A lower retention takes effect at once.
    pub fn update_attributes(&mut self, changes: &AttributeChanges, now: u64) {
        self.attributes.apply(changes);
        self.expire(now);
    }

    pub fn clear_dead_letter_config(&mut self) {
        self.attributes.dead_letter = None;
    }

    pub fn counts(&self) -> Counts {
        let in_flight = self
            .hidden
            .iter()
            .filter(|(_, key)| self.messages[key].receipt.is_some())
            .count();
        Counts {
            visible: self.messages.len() - self.hidden.len(),
            in_flight,
            delayed: self.hidden.len() - in_flight,
        }
    }

    /// Brings the queue up to date with a clock that moved forward to `now`.
    pub fn advance(&mut self, now: u64) {
        while let Some(&(available_at, key)) = self.hidden.first() {
            if available_at > now {
                break;
            }
            self.hidden.pop_first();
            if self.is_group_head(key) {
                self.eligible.insert(key);
            }
        }
        self.expire(now);
    }

    /// Removes every message, whatever its state. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.hidden.clear();
        self.eligible.clear();
        self.groups.clear();
        self.receipts.clear();
    }

    /// Applies the numbered rules of `SendMessage` and adds the message unless it is a duplicate.
    pub fn send(
        &mut self,
        request: SendMessage,
        now: u64,
        message_ids: &mut Counter,
    ) -> Result<Sent> {
        let SendMessage {
            body,
            delay_seconds,
            group_id,
            deduplication_id,
            ..
        } = request;
        let fifo = self.is_fifo();
        if !fifo && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(Error::unsupported(
                "GroupId and DeduplicationId need a FIFO queue",
            ));
        }
        if fifo && delay_seconds.is_some() {
            return Err(Error::unsupported(
                "a FIFO queue does not take a per-message DelaySeconds",
            ));
        }

        let fifo_ids = if fifo {
            let group_id = group_id.ok_or_else(|| Error::missing_parameter("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if self.attributes.content_based_deduplication => body.clone(),
                None => return Err(Error::missing_parameter("DeduplicationId")),
            };
            if let Some(original) = self.dedup.find(&deduplication_id, now) {
                return Ok(Sent {
                    message_id: original.message_id,
                    sequence_number: Some(original.sequence_number),
                });
            }
            Some((group_id, deduplication_id))
        } else {
            None
        };

        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let message_id = message_ids.next();
        let delay = delay_seconds.unwrap_or(self.attributes.delay_seconds);
        let fifo_fields = fifo_ids.map(|(group_id, deduplication_id)| {
            let sequence_number = self.take_sequence_number();
            let original = Original {
                message_id,
                sequence_number,
            };
            self.dedup.record(deduplication_id, now, original);
            FifoFields {
                group_id,
                sequence_number,
            }
        });
        let sent = Sent {
            message_id,
            sequence_number: fifo_fields.as_ref().map(|f| f.sequence_number),
        };
        self.insert(
            Message {
                id: message_id,
                body,
                sent_at: now,
                receive_count: 0,
                first_received_at: None,
                fifo: fifo_fields,
                available_at: now + delay,
                receipt: None,
            },
            now,
        );
        Ok(sent)
    }

    /// Examines eligible messages in arrival order, delivering up to `max_messages` of them and
    /// taking out those that are due for the dead-letter queue.
    pub fn receive(
        &mut self,
        max_messages: usize,
        visibility_timeout: Option<u64>,
        now: u64,
        receipt_handles: &mut Counter,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|config| config.max_receive_count);
        let mut received = Received::default();
        let mut last_examined = None;

        while received.deliveries.len() < max_messages {
            // Walk forward from the last examined key rather than from the start: a delivery with a
            // zero timeout leaves the message eligible, and it must not be examined twice. Taking a
            // FIFO message out can make a later message of its group eligible, and the walk then
            // reaches it in its turn, as a plain scan in arrival order would.
            let lower = last_examined.map_or(Bound::Unbounded, Bound::Excluded);
            let Some(&key) = self.eligible.range((lower, Bound::Unbounded)).next() else {
                break;
            };
            last_examined = Some(key);

            if max_receive_count.is_some_and(|max| self.messages[&key].receive_count >= max) {
                let message = self.remove(key, now);
                received.dead_letters.push(message);
                continue;
            }

            let receipt = receipt_handles.next();
            self.unindex_visibility(key);
            let message = self
                .messages
                .get_mut(&key)
                .expect("eligible message exists");
            if let Some(old) = message.receipt.replace(receipt) {
                self.receipts.remove(&old);
            }
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.available_at = now + visibility_timeout;
            received.deliveries.push(Delivery {
                message_id: format_message_id(message.id),
                receipt_handle: format_receipt_handle(receipt),
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.fifo.as_ref().map(|f| f.group_id.clone()),
                sequence_number: message.fifo.as_ref().map(|f| f.sequence_number),
            });
            self.receipts.insert(receipt, key);
            self.index_visibility(key, now);
        }
        received
    }

    /// Adds a message moved here from the queue whose dead-letter queue this is.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.receipt = None;
        message.available_at = now;
        if message.fifo.is_some() {
            let sequence_number = self.take_sequence_number();
            if let Some(fifo) = &mut message.fifo {
                fifo.sequence_number = sequence_number;
            }
        }
        self.insert(message, now);
        // The message keeps its SentAt, so it may already be too old for this queue's retention.
        self.expire(now);
    }

    /// Deletes the in-flight message whose latest receipt handle is `receipt`.
    pub fn delete_message(&mut self, receipt: u64, now: u64) -> Result<()> {
        let key = self.in_flight_key(receipt, now)?;
        self.remove(key, now);
        Ok(())
    }

    /// Makes the in-flight message whose latest receipt handle is `receipt` visible again at
    /// `now + visibility_timeout`.
    pub fn change_visibility(
        &mut self,
        receipt: u64,
        visibility_timeout: u64,
        now: u64,
    ) -> Result<()> {
        let key = self.in_flight_key(receipt, now)?;
        self.unindex_visibility(key);
        self.messages
            .get_mut(&key)
            .expect("in-flight message exists")
            .available_at = now + visibility_timeout;
        self.index_visibility(key, now);
        Ok(())
    }

    fn in_flight_key(&self, receipt: u64, now: u64) -> Result<Key> {
        self.receipts
            .get(&receipt)
            .copied()
            .filter(|key| self.messages[key].available_at > now)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    "the receipt handle is not the latest one of a message in flight in this queue",
                )
            })
    }

    fn take_sequence_number(&mut self) -> u64 {
        self.last_sequence_number += 1;
        self.last_sequence_number
    }

    /// Drops every message whose retention period has ended.
    fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, key)) = self.by_sent_at.first() {
            if sent_at + retention > now {
                break;
            }
            self.remove(key, now);
        }
    }

    fn insert(&mut self, message: Message, now: u64) {
        let key = self.next_key;
        self.next_key += 1;
        self.by_sent_at.insert((message.sent_at, key));
        if let Some(fifo) = &message.fifo {
            self.groups
                .entry(fifo.group_id.clone())
                .or_default()
                .insert(key);
        }
        self.messages.insert(key, message);
        self.index_visibility(key, now);
    }

    fn remove(&mut self, key: Key, now: u64) -> Message {
        self.unindex_visibility(key);
        let message = self.messages.remove(&key).expect("removed message exists");
        self.by_sent_at.remove(&(message.sent_at, key));
        if let Some(receipt) = message.receipt {
            self.receipts.remove(&receipt);
        }
        if let Some(fifo) = &message.fifo {
            let group = self
                .groups
                .get_mut(&fifo.group_id)
                .expect("FIFO message has a group");
            group.remove(&key);
            match group.first() {
                None => {
                    self.groups.remove(&fifo.group_id);
                }
                // The next message of the group may have just become its head.
                Some(&head) if head > key && self.messages[&head].available_at <= now => {
                    self.eligible.insert(head);
                }
                Some(_) => {}
            }
        }
        message
    }

    fn is_group_head(&self, key: Key) -> bool {
        match &self.messages[&key].fifo {
            None => true,
            Some(fifo) => self.groups[&fifo.group_id].first() == Some(&key),
        }
    }

    /// Files a message under `hidden` or `eligible` according to its `available_at`.
    fn index_visibility(&mut self, key: Key, now: u64) {
        let available_at = self.messages[&key].available_at;
        if available_at > now {
            self.hidden.insert((available_at, key));
        } else if self.is_group_head(key) {
            self.eligible.insert(key);
        }
    }

    /// Undoes [`index_visibility`](Self::index_visibility); call before changing `available_at`.
    fn unindex_visibility(&mut self, key: Key) {
        let available_at = self.messages[&key].available_at;
        self.hidden.remove(&(available_at, key));
        self.eligible.remove(&key);
    }
}

pub fn format_message_id(id: u64) -> String {
    format!("m-{id}")
}

pub fn format_receipt_handle(handle: u64) -> String {
    format!("r-{handle}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fifo_queue() -> Queue {
        Queue::new(QueueAttributes {
            fifo: true,
            content_based_deduplication: true,
            ..QueueAttributes::default()
        })
    }

    fn send(
        queue: &mut Queue,
        ids: &mut Counter,
        body: &str,
        group: Option<&str>,
        now: u64,
    ) -> Sent {
        let request = SendMessage {
            queue_name: String::new(),
            body: body.into(),
            delay_seconds: None,
            group_id: group.map(Into::into),
            deduplication_id: None,
        };
        queue.send(request, now, ids).expect("send succeeds")
    }

    fn bodies(received: &Received) -> Vec<&str> {
        received
            .deliveries
            .iter()
            .map(|d| d.body.as_str())
            .collect()
    }

    #[test]
    fn fifo_receive_takes_one_message_per_group_head() {
        let (mut queue, mut ids, mut handles) =
            (fifo_queue(), Counter::default(), Counter::default());
        for (body, group) in [("a1", "a"), ("a2", "a"), ("b1", "b")] {
            send(&mut queue, &mut ids, body, Some(group), 0);
        }
        let first = queue.receive(10, None, 0, &mut handles);
        assert_eq!(bodies(&first), ["a1", "b1"]);
        assert_eq!(queue.receive(10, None, 0, &mut handles).deliveries, []);

        queue.delete_message(1, 0).unwrap();
        assert_eq!(bodies(&queue.receive(10, None, 0, &mut handles)), ["a2"]);
    }

    #[test]
    fn zero_visibility_timeout_examines_each_message_once() {
        let mut queue = Queue::new(QueueAttributes::default());
        let (mut ids, mut handles) = (Counter::default(), Counter::default());
        send(&mut queue, &mut ids, "x", None, 0);
        let received = queue.receive(10, Some(0), 0, &mut handles);
        assert_eq!(bodies(&received), ["x"]);
        assert_eq!(queue.counts().visible, 1);
    }

    #[test]
    fn counts_follow_the_clock() {
        let mut queue = Queue::new(QueueAttributes {
            delay_seconds: 5,
            ..QueueAttributes::default()
        });
        let (mut ids, mut handles) = (Counter::default(), Counter::default());
        send(&mut queue, &mut ids, "x", None, 0);
        assert_eq!(
            queue.counts(),
            Counts {
                visible: 0,
                in_flight: 0,
                delayed: 1
            }
        );
        queue.advance(5);
        assert_eq!(
            queue.counts(),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
        queue.receive(1, Some(10), 5, &mut handles);
        assert_eq!(
            queue.counts(),
            Counts {
                visible: 0,
                in_flight: 1,
                delayed: 0
            }
        );
        queue.advance(15);
        assert_eq!(
            queue.counts(),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
    }

    #[test]
    fn retention_drops_old_dead_letters_on_arrival() {
        let mut queue = Queue::new(QueueAttributes {
            retention_seconds: 60,
            ..QueueAttributes::default()
        });
        let mut source = Queue::new(QueueAttributes {
            dead_letter: Some(DeadLetterConfig {
                queue_name: "dlq".into(),
                max_receive_count: 1,
            }),
            ..QueueAttributes::default()
        });
        let (mut ids, mut handles) = (Counter::default(), Counter::default());
        send(&mut source, &mut ids, "x", None, 0);
        source.receive(1, Some(0), 0, &mut handles);
        source.advance(100);
        let received = source.receive(1, None, 100, &mut handles);
        assert_eq!(received.deliveries, []);
        for message in received.dead_letters {
            queue.accept_dead_letter(message, 100);
        }
        assert_eq!(queue.counts(), Counts::default());
        assert_eq!(source.counts(), Counts::default());
    }
}
