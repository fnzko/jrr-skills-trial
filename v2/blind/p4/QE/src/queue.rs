//! One queue: its messages in arrival order, its deduplication window and the
//! rules that move messages between delayed, in flight and visible.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::attributes::{Attributes, QueueKind};
use crate::command::SendMessage;
use crate::error::{Error, ErrorCode, Result};
use crate::types::{GroupId, MessageId, ReceiptHandle};

/// A queue holds at most this many messages, counted in every state.
pub const MAX_MESSAGES: usize = 100_000;
/// A FIFO send with a deduplication id already accepted this many seconds ago
/// is a duplicate.
const DEDUPLICATION_WINDOW: u64 = 300;

/// The place a message holds in the process, numbered across all queues.
///
/// Numbers are never reused, so a stale receipt handle cannot reach a
/// different message, even one in a queue that was deleted and recreated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Arrival(u64);

/// The counters that number messages across the whole process. Only accepted
/// sends and dead-letter moves take numbers.
#[derive(Debug)]
pub struct Ids {
    next_message: u64,
    next_arrival: u64,
    next_receipt: u64,
}

impl Ids {
    pub fn new() -> Self {
        Ids {
            next_message: 1,
            next_arrival: 0,
            next_receipt: 1,
        }
    }

    /// The handle of the next delivery. Each call numbers one delivery, so the
    /// caller must deliver the message it asks for.
    pub fn receipt(&mut self) -> ReceiptHandle {
        let handle = ReceiptHandle::new(self.next_receipt);
        self.next_receipt += 1;
        handle
    }

    pub fn message_id(&mut self) -> MessageId {
        let id = MessageId::new(self.next_message);
        self.next_message += 1;
        id
    }

    pub fn arrival(&mut self) -> Arrival {
        self.next_arrival += 1;
        Arrival(self.next_arrival)
    }
}

impl Default for Ids {
    fn default() -> Self {
        Ids::new()
    }
}

/// Where a message is at a given time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub(crate) id: MessageId,
    pub(crate) body: String,
    pub(crate) sent_at: u64,
    /// The message is delayed while the clock is below this time.
    pub(crate) available_at: u64,
    /// The message is in flight while the clock is below this time.
    pub(crate) invisible_until: u64,
    pub(crate) receive_count: u64,
    pub(crate) first_received_at: Option<u64>,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) sequence_number: Option<u64>,
    /// The handle of the latest delivery, the only one that can still act on
    /// the message.
    pub(crate) receipt: Option<ReceiptHandle>,
}

impl Message {
    pub(crate) fn state(&self, now: u64) -> State {
        if now < self.available_at {
            State::Delayed
        } else if now < self.invisible_until {
            State::InFlight
        } else {
            State::Visible
        }
    }

    /// Records a delivery: the count grows, the first delivery time is kept,
    /// and the message is in flight until `visibility_timeout` has passed.
    pub(crate) fn deliver(&mut self, now: u64, visibility_timeout: u64, receipt: ReceiptHandle) {
        self.receive_count += 1;
        self.first_received_at.get_or_insert(now);
        self.invisible_until = now + visibility_timeout;
        self.receipt = Some(receipt);
    }

    /// Ends the in-flight period at `now + visibility_timeout`, which may be
    /// now, so a timeout of 0 makes the message visible at once.
    pub(crate) fn change_visibility(&mut self, now: u64, visibility_timeout: u64) {
        self.invisible_until = now + visibility_timeout;
    }
}

/// The numbers a send reports to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    pub sequence_number: Option<u64>,
}

/// How many messages are in each state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

/// The messages one receive will deliver and the ones it will move to the
/// dead-letter queue, in the order it examines them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Examination {
    pub deliver: Vec<Arrival>,
    pub dead_letter: Vec<Arrival>,
}

#[derive(Debug)]
struct Accepted {
    sent_at: u64,
    message_id: MessageId,
    sequence_number: u64,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    next_sequence_number: u64,
    /// Accepted FIFO sends by deduplication id. Entries outlive their messages.
    accepted: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Queue {
            attributes,
            messages: BTreeMap::new(),
            next_sequence_number: 1,
            accepted: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.kind == QueueKind::Fifo
    }

    /// Replaces the attributes. A shorter retention removes messages at once.
    pub fn set_attributes(&mut self, now: u64, attributes: Attributes) {
        self.attributes = attributes;
        self.expire(now);
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
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

    /// Accepts a send, or reports the earlier message it duplicates.
    ///
    /// The checks follow rules 1 to 5 of section 6, in that order.
    pub fn send(&mut self, now: u64, request: &SendMessage, ids: &mut Ids) -> Result<Sent> {
        let fifo = self.is_fifo();
        let unsupported = if fifo {
            request.delay_seconds.is_some()
        } else {
            request.group_id.is_some() || request.deduplication_id.is_some()
        };
        if unsupported {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "this parameter is not supported by this type of queue",
            ));
        }

        let mut deduplication_key = None;
        if fifo {
            if request.group_id.is_none() {
                return Err(missing("GroupId is required on a FIFO queue"));
            }
            let key = match (
                &request.deduplication_id,
                self.attributes.content_based_deduplication,
            ) {
                (Some(id), _) => id.as_str().to_owned(),
                (None, true) => request.body.clone(),
                (None, false) => {
                    return Err(missing(
                        "DeduplicationId is required when ContentBasedDeduplication is false",
                    ))
                }
            };
            if let Some(previous) = self.accepted.get(&key) {
                if now - previous.sent_at < DEDUPLICATION_WINDOW {
                    return Ok(Sent {
                        message_id: previous.message_id,
                        sequence_number: Some(previous.sequence_number),
                    });
                }
            }
            deduplication_key = Some(key);
        }

        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let message_id = ids.message_id();
        let sequence_number = fifo.then(|| self.take_sequence_number());
        let delay = request
            .delay_seconds
            .unwrap_or(self.attributes.delay_seconds);
        let message = Message {
            id: message_id,
            body: request.body.clone(),
            sent_at: now,
            available_at: now + delay,
            invisible_until: 0,
            receive_count: 0,
            first_received_at: None,
            group_id: request.group_id.clone(),
            sequence_number,
            receipt: None,
        };
        self.messages.insert(ids.arrival(), message);

        if let (Some(key), Some(sequence_number)) = (deduplication_key, sequence_number) {
            self.accepted.insert(
                key,
                Accepted {
                    sent_at: now,
                    message_id,
                    sequence_number,
                },
            );
        }
        Ok(Sent {
            message_id,
            sequence_number,
        })
    }

    /// Works out what a receive of at most `max_messages` would do, without
    /// changing anything. Messages are examined once each in arrival order.
    ///
    /// On a FIFO queue a message is skipped while an earlier message of its
    /// group is still in the queue in any state, so the earlier one blocks it.
    pub fn examine(&self, now: u64, max_messages: u64) -> Examination {
        let fifo = self.is_fifo();
        let dead_letter_limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        let mut examination = Examination::default();
        let mut delivered: u64 = 0;
        for (&arrival, message) in &self.messages {
            if delivered == max_messages {
                break;
            }
            let group = message.group_id.as_ref().filter(|_| fifo);
            if group.is_some_and(|group| blocked.contains(group)) {
                continue;
            }
            if message.state(now) != State::Visible {
                if let Some(group) = group {
                    blocked.insert(group);
                }
                continue;
            }
            let exhausted =
                dead_letter_limit.is_some_and(|limit| u64::from(limit) <= message.receive_count);
            if exhausted {
                examination.dead_letter.push(arrival);
                continue;
            }
            examination.deliver.push(arrival);
            delivered += 1;
            if let Some(group) = group {
                blocked.insert(group);
            }
        }
        examination
    }

    pub fn message(&self, arrival: Arrival) -> Option<&Message> {
        self.messages.get(&arrival)
    }

    pub fn message_mut(&mut self, arrival: Arrival) -> Option<&mut Message> {
        self.messages.get_mut(&arrival)
    }

    pub fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        self.messages.remove(&arrival)
    }

    /// Adds a message that was moved here from a dead-letter source. It keeps
    /// its id, body, send time and group, and gets the state of a new message
    /// with no delay. If it is already older than this queue's retention, it
    /// is dropped at once.
    pub fn accept_dead_letter(&mut self, now: u64, arrival: Arrival, moved: Message) {
        if now >= moved.sent_at + self.attributes.retention_seconds {
            return;
        }
        let sequence_number = self.is_fifo().then(|| self.take_sequence_number());
        let message = Message {
            available_at: now,
            invisible_until: 0,
            receive_count: 0,
            first_received_at: None,
            sequence_number,
            receipt: None,
            ..moved
        };
        self.messages.insert(arrival, message);
    }

    /// Removes every message that the retention period has ended.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        self.messages
            .retain(|_, message| now < message.sent_at + retention);
    }

    /// Removes every message in every state. Sequence numbers and
    /// deduplication entries carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
    }

    fn take_sequence_number(&mut self) -> u64 {
        let number = self.next_sequence_number;
        self.next_sequence_number += 1;
        number
    }
}

fn missing(message: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DeduplicationId, GroupId};

    fn standard() -> Queue {
        Queue::new(Attributes::default())
    }

    fn fifo() -> Queue {
        Queue::new(Attributes {
            kind: QueueKind::Fifo,
            ..Attributes::default()
        })
    }

    fn send(body: &str) -> SendMessage {
        SendMessage {
            queue: "q".into(),
            body: body.into(),
            delay_seconds: None,
            group_id: None,
            deduplication_id: None,
        }
    }

    fn fifo_send(body: &str, group: &str, dedup: Option<&str>) -> SendMessage {
        SendMessage {
            group_id: GroupId::parse(group),
            deduplication_id: dedup.map(|id| DeduplicationId::parse(id).unwrap()),
            ..send(body)
        }
    }

    #[test]
    fn a_new_message_is_visible_and_numbered_from_one() {
        let mut queue = standard();
        let mut ids = Ids::new();
        let sent = queue.send(0, &send("hello"), &mut ids).unwrap();
        assert_eq!(
            sent,
            Sent {
                message_id: MessageId::new(1),
                sequence_number: None
            }
        );
        assert_eq!(
            queue.counts(0),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
    }

    #[test]
    fn a_delayed_message_becomes_visible_when_the_delay_ends() {
        let mut queue = standard();
        let mut ids = Ids::new();
        let delayed = SendMessage {
            delay_seconds: Some(10),
            ..send("later")
        };
        queue.send(5, &delayed, &mut ids).unwrap();
        assert_eq!(
            queue.counts(14),
            Counts {
                visible: 0,
                in_flight: 0,
                delayed: 1
            }
        );
        assert_eq!(
            queue.counts(15),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
    }

    #[test]
    fn a_fifo_queue_rejects_a_per_message_delay() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        let request = SendMessage {
            delay_seconds: Some(1),
            ..fifo_send("x", "g", None)
        };
        let error = queue.send(0, &request, &mut ids).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedOperation);
    }

    #[test]
    fn a_standard_queue_rejects_group_and_deduplication_ids() {
        let mut queue = standard();
        let mut ids = Ids::new();
        let grouped = SendMessage {
            group_id: GroupId::parse("g"),
            ..send("x")
        };
        assert_eq!(
            queue.send(0, &grouped, &mut ids).unwrap_err().code(),
            ErrorCode::UnsupportedOperation
        );
    }

    #[test]
    fn a_fifo_send_needs_a_group_and_a_deduplication_id_or_content_deduplication() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        assert_eq!(
            queue.send(0, &send("x"), &mut ids).unwrap_err().code(),
            ErrorCode::MissingParameter
        );
        let no_dedup = fifo_send("x", "g", None);
        assert_eq!(
            queue.send(0, &no_dedup, &mut ids).unwrap_err().code(),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn fifo_sends_take_consecutive_sequence_numbers() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        let first = queue
            .send(0, &fifo_send("a", "g", Some("1")), &mut ids)
            .unwrap();
        let second = queue
            .send(0, &fifo_send("b", "g", Some("2")), &mut ids)
            .unwrap();
        assert_eq!(first.sequence_number, Some(1));
        assert_eq!(second.sequence_number, Some(2));
        assert_eq!(second.message_id, MessageId::new(2));
    }

    #[test]
    fn a_duplicate_within_the_window_reports_the_original() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        let original = queue
            .send(0, &fifo_send("a", "g", Some("d")), &mut ids)
            .unwrap();
        let duplicate = queue
            .send(299, &fifo_send("other", "g2", Some("d")), &mut ids)
            .unwrap();
        assert_eq!(duplicate, original);
        assert_eq!(queue.message_count(), 1);
    }

    #[test]
    fn a_duplicate_does_not_extend_the_window() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        queue
            .send(0, &fifo_send("a", "g", Some("d")), &mut ids)
            .unwrap();
        queue
            .send(200, &fifo_send("a", "g", Some("d")), &mut ids)
            .unwrap();
        let reopened = queue
            .send(300, &fifo_send("a", "g", Some("d")), &mut ids)
            .unwrap();
        assert_eq!(reopened.message_id, MessageId::new(2));
        assert_eq!(reopened.sequence_number, Some(2));
    }

    #[test]
    fn content_based_deduplication_uses_the_body() {
        let mut queue = Queue::new(Attributes {
            kind: QueueKind::Fifo,
            content_based_deduplication: true,
            ..Attributes::default()
        });
        let mut ids = Ids::new();
        let first = queue
            .send(0, &fifo_send("same", "g", None), &mut ids)
            .unwrap();
        let second = queue
            .send(1, &fifo_send("same", "g", None), &mut ids)
            .unwrap();
        let third = queue
            .send(2, &fifo_send("other", "g", None), &mut ids)
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(third.message_id, MessageId::new(2));
    }

    #[test]
    fn a_full_queue_refuses_a_send_until_a_message_leaves() {
        let mut queue = standard();
        let mut ids = Ids::new();
        for n in 0..MAX_MESSAGES {
            queue.send(0, &send(&n.to_string()), &mut ids).unwrap();
        }
        assert_eq!(
            queue
                .send(0, &send("one more"), &mut ids)
                .unwrap_err()
                .code(),
            ErrorCode::LimitExceeded
        );
        let first = Arrival(1);
        queue.remove(first);
        assert!(queue.send(0, &send("fits now"), &mut ids).is_ok());
    }

    #[test]
    fn examine_skips_a_group_behind_an_earlier_message() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        queue
            .send(0, &fifo_send("a1", "a", Some("1")), &mut ids)
            .unwrap();
        queue
            .send(0, &fifo_send("a2", "a", Some("2")), &mut ids)
            .unwrap();
        queue
            .send(0, &fifo_send("b1", "b", Some("3")), &mut ids)
            .unwrap();
        let examination = queue.examine(0, 10);
        assert_eq!(examination.deliver, [Arrival(1), Arrival(3)]);
        assert!(examination.dead_letter.is_empty());
    }

    #[test]
    fn examine_blocks_a_group_while_its_earlier_message_is_in_flight() {
        let mut queue = fifo();
        let mut ids = Ids::new();
        queue
            .send(0, &fifo_send("a1", "a", Some("1")), &mut ids)
            .unwrap();
        queue
            .send(0, &fifo_send("a2", "a", Some("2")), &mut ids)
            .unwrap();
        queue
            .send(0, &fifo_send("b1", "b", Some("3")), &mut ids)
            .unwrap();
        queue.message_mut(Arrival(1)).unwrap().invisible_until = 30;
        assert_eq!(queue.examine(0, 10).deliver, [Arrival(3)]);
    }

    #[test]
    fn examine_moves_exhausted_messages_without_counting_them() {
        let mut queue = Queue::new(Attributes {
            dead_letter: Some(crate::attributes::DeadLetter {
                queue: crate::types::QueueName::parse("dead").unwrap(),
                max_receive_count: 2,
            }),
            ..Attributes::default()
        });
        let mut ids = Ids::new();
        for body in ["a", "b", "c"] {
            queue.send(0, &send(body), &mut ids).unwrap();
        }
        queue.message_mut(Arrival(1)).unwrap().receive_count = 2;
        let examination = queue.examine(0, 1);
        assert_eq!(examination.dead_letter, [Arrival(1)]);
        assert_eq!(examination.deliver, [Arrival(2)]);
    }

    #[test]
    fn retention_removes_messages_once_it_has_passed() {
        let mut queue = Queue::new(Attributes {
            retention_seconds: 60,
            ..Attributes::default()
        });
        let mut ids = Ids::new();
        queue.send(0, &send("old"), &mut ids).unwrap();
        queue.expire(59);
        assert_eq!(queue.message_count(), 1);
        queue.expire(60);
        assert_eq!(queue.message_count(), 0);
    }

    #[test]
    fn a_dead_letter_already_past_the_destination_retention_is_dropped_on_arrival() {
        let mut destination = Queue::new(Attributes {
            retention_seconds: 60,
            ..Attributes::default()
        });
        let mut source = standard();
        let mut ids = Ids::new();
        source.send(0, &send("stale"), &mut ids).unwrap();
        let moved = source.remove(Arrival(1)).unwrap();
        destination.accept_dead_letter(60, ids.arrival(), moved);
        assert_eq!(destination.message_count(), 0);
    }

    #[test]
    fn a_dead_letter_within_retention_is_visible_at_once_with_a_fresh_count() {
        let mut destination = standard();
        let mut source = standard();
        let mut ids = Ids::new();
        source.send(0, &send("kept"), &mut ids).unwrap();
        source.message_mut(Arrival(1)).unwrap().receive_count = 4;
        let moved = source.remove(Arrival(1)).unwrap();
        destination.accept_dead_letter(10, ids.arrival(), moved);
        assert_eq!(
            destination.counts(10),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
        let message = destination.message(Arrival(2)).unwrap();
        assert_eq!(message.receive_count, 0);
        assert_eq!(message.sent_at, 0);
        assert_eq!(message.first_received_at, None);
    }

    #[test]
    fn lowering_retention_removes_messages_at_once() {
        let mut queue = standard();
        let mut ids = Ids::new();
        queue.send(0, &send("old"), &mut ids).unwrap();
        queue.set_attributes(
            100,
            Attributes {
                retention_seconds: 60,
                ..Attributes::default()
            },
        );
        assert_eq!(queue.message_count(), 0);
    }
}
