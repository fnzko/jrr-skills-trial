//! One queue: its attributes, its messages and the rules for sending,
//! receiving and acknowledging them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::command::{AttributeValues, Outgoing};
use crate::error::{Error, ErrorCode};
use crate::types::{
    DedupKey, GroupId, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber,
};

const MESSAGE_LIMIT: usize = 100_000;
const DEDUP_WINDOW: Seconds = Seconds::new(300);

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

/// Whether a queue keeps strict order per group. The type never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    fn requested(values: &AttributeValues) -> Self {
        if values.fifo.unwrap_or(false) {
            Self::Fifo {
                content_based_deduplication: values.content_based_deduplication.unwrap_or(false),
            }
        } else {
            Self::Standard
        }
    }
}

/// Where messages that were received too often are moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

impl Attributes {
    /// The attributes of a new queue: requested values over the defaults.
    pub fn create(values: &AttributeValues, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: values
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: values.delay.unwrap_or(Seconds::ZERO),
            retention: values.retention.unwrap_or(DEFAULT_RETENTION),
            kind: QueueKind::requested(values),
            dead_letter,
        }
    }

    /// Whether creating a queue with `values` and `dead_letter`, the dead-letter
    /// configuration they request, would give exactly these attributes.
    pub fn match_create(&self, values: &AttributeValues, dead_letter: Option<(&str, u32)>) -> bool {
        let dead_letter_matches = match (dead_letter, &self.dead_letter) {
            (None, None) => true,
            (Some((queue, count)), Some(own)) => {
                own.queue.as_str() == queue && own.max_receive_count == count
            }
            _ => false,
        };
        dead_letter_matches
            && self.visibility_timeout
                == values
                    .visibility_timeout
                    .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay == values.delay.unwrap_or(Seconds::ZERO)
            && self.retention == values.retention.unwrap_or(DEFAULT_RETENTION)
            && self.kind == QueueKind::requested(values)
    }

    /// Applies the attributes present in `values`; `dead_letter` replaces the
    /// current configuration when given.
    pub fn update(&mut self, values: &AttributeValues, dead_letter: Option<DeadLetter>) {
        self.visibility_timeout = values.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = values.delay.unwrap_or(self.delay);
        self.retention = values.retention.unwrap_or(self.retention);
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(requested),
        ) = (&mut self.kind, values.content_based_deduplication)
        {
            *content_based_deduplication = requested;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// Arrival order within one queue; never reused, never reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Group and sequence number of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placement {
    #[serde(rename = "GroupId")]
    group: GroupId,
    #[serde(rename = "SequenceNumber")]
    sequence: SequenceNumber,
}

/// What keeps a message from being received, if anything.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Released,
    Delayed {
        until: Seconds,
    },
    InFlight {
        until: Seconds,
        handle: ReceiptHandle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: String,
    sent_at: Seconds,
    receive_count: u32,
    first_received_at: Option<Seconds>,
    placement: Option<Placement>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Seconds) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// A message on its way into a queue. `group` is set exactly for FIFO queues.
#[derive(Debug)]
pub struct Entry {
    id: MessageId,
    body: String,
    sent_at: Seconds,
    group: Option<GroupId>,
    delay: Seconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DedupEntry {
    accepted_at: Seconds,
    message: MessageId,
    sequence: Option<SequenceNumber>,
}

/// Result of a send: the message that now stands for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sent {
    #[serde(rename = "MessageId")]
    pub message_id: MessageId,
    #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

/// A message as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: Seconds,
    first_received_at: Seconds,
    #[serde(flatten)]
    placement: Option<Placement>,
}

/// Messages moved out of a queue, to be added to `queue`.
#[derive(Debug)]
pub struct DeadLettered {
    pub queue: QueueName,
    pub entries: Vec<Entry>,
}

#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_lettered: Option<DeadLettered>,
}

/// How many messages are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    /// Orders messages by age so retention can drop the oldest in logarithmic time.
    by_sent_at: BTreeSet<(Seconds, Arrival)>,
    /// Latest receipt handle of every message that has been delivered.
    handles: HashMap<ReceiptHandle, Arrival>,
    /// Deduplication ids outlive their messages, so they are kept apart.
    dedup: HashMap<DedupKey, DedupEntry>,
    next_arrival: u64,
    last_sequence: u64,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            dedup: HashMap::new(),
            next_arrival: 0,
            last_sequence: 0,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut Attributes {
        &mut self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.attributes.kind, QueueKind::Fifo { .. })
    }

    /// Removes the messages that retention has run out for. Callers run this
    /// before reading a queue, which makes retention hold at every moment.
    pub fn expire(&mut self, now: Seconds) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at.saturating_add(self.attributes.retention) {
                break;
            }
            self.remove(arrival);
        }
    }

    pub fn counts(&self, now: Seconds) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.state(now) {
                    State::Visible => counts.visible += 1,
                    State::InFlight => counts.in_flight += 1,
                    State::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    /// Appends a message and returns its sequence number on a FIFO queue.
    pub fn add(&mut self, entry: Entry) -> Option<SequenceNumber> {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        let placement = entry.group.map(|group| {
            self.last_sequence += 1;
            Placement {
                group,
                sequence: SequenceNumber::new(self.last_sequence),
            }
        });
        let sequence = placement.as_ref().map(|placement| placement.sequence);
        let hold = if entry.delay > Seconds::ZERO {
            Hold::Delayed {
                until: entry.sent_at.saturating_add(entry.delay),
            }
        } else {
            Hold::Released
        };
        self.by_sent_at.insert((entry.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: entry.id,
                body: entry.body,
                sent_at: entry.sent_at,
                receive_count: 0,
                first_received_at: None,
                placement,
                hold,
            },
        );
        sequence
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Hold::InFlight { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// Accepts a message into this queue. `next_id` runs only when the message
    /// is accepted, so failed and duplicate sends do not use up an id.
    pub fn send(
        &mut self,
        now: Seconds,
        outgoing: Outgoing,
        next_id: impl FnOnce() -> MessageId,
    ) -> Result<Sent, Error> {
        let Outgoing {
            body,
            delay,
            group,
            deduplication_id,
        } = outgoing;
        let fifo = match self.attributes.kind {
            QueueKind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(unsupported(
                        "a standard queue takes neither GroupId nor DeduplicationId",
                    ));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported(
                        "a FIFO queue takes no per-message DelaySeconds",
                    ));
                }
                let group = group.ok_or_else(|| Error::missing_parameter("GroupId"))?;
                let key = match deduplication_id {
                    Some(id) => DedupKey::from(id),
                    None if content_based_deduplication => DedupKey::from_body(&body),
                    None => return Err(Error::missing_parameter("DeduplicationId")),
                };
                let duplicate = self
                    .dedup
                    .get(&key)
                    .filter(|seen| now < seen.accepted_at.saturating_add(DEDUP_WINDOW));
                if let Some(seen) = duplicate {
                    return Ok(Sent {
                        message_id: seen.message,
                        sequence_number: seen.sequence,
                    });
                }
                Some((group, key))
            }
        };
        if self.messages.len() >= MESSAGE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MESSAGE_LIMIT} messages"),
            ));
        }
        let id = next_id();
        let (group, key) = fifo.unzip();
        let sequence_number = self.add(Entry {
            id,
            body,
            sent_at: now,
            group,
            delay: delay.unwrap_or(self.attributes.delay),
        });
        if let Some(key) = key {
            self.dedup.insert(
                key,
                DedupEntry {
                    accepted_at: now,
                    message: id,
                    sequence: sequence_number,
                },
            );
        }
        Ok(Sent {
            message_id: id,
            sequence_number,
        })
    }

    /// Delivers up to `max_messages` eligible messages and moves the ones that
    /// were received too often out of the queue.
    pub fn receive(
        &mut self,
        now: Seconds,
        max_messages: u32,
        visibility_timeout: Seconds,
        mut next_handle: impl FnMut() -> ReceiptHandle,
    ) -> Received {
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut blocked_groups: HashSet<&GroupId> = HashSet::new();
        let mut deliveries = Vec::new();
        let mut moves = Vec::new();
        for (&arrival, message) in &self.messages {
            if deliveries.len() == max_messages as usize {
                break;
            }
            let group = message.placement.as_ref().map(|placement| &placement.group);
            if group.is_some_and(|group| blocked_groups.contains(group)) {
                continue;
            }
            let moved = message.state(now) == State::Visible
                && max_receive_count.is_some_and(|max| message.receive_count >= max);
            if moved {
                moves.push(arrival);
                continue;
            }
            if message.state(now) == State::Visible {
                deliveries.push(arrival);
            }
            blocked_groups.extend(group);
        }
        let delivered = deliveries
            .into_iter()
            .map(|arrival| self.deliver(arrival, now, visibility_timeout, next_handle()))
            .collect();
        let entries: Vec<Entry> = moves
            .into_iter()
            .map(|arrival| {
                let message = self
                    .remove(arrival)
                    .expect("a planned arrival is still in the queue");
                Entry {
                    id: message.id,
                    body: message.body,
                    sent_at: message.sent_at,
                    group: message.placement.map(|placement| placement.group),
                    delay: Seconds::ZERO,
                }
            })
            .collect();
        let dead_lettered = self
            .attributes
            .dead_letter
            .as_ref()
            .filter(|_| !entries.is_empty())
            .map(|dead_letter| DeadLettered {
                queue: dead_letter.queue.clone(),
                entries,
            });
        Received {
            delivered,
            dead_lettered,
        }
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        now: Seconds,
        visibility_timeout: Seconds,
        handle: ReceiptHandle,
    ) -> Delivery {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("a planned arrival is still in the queue");
        if let Hold::InFlight { handle: old, .. } = message.hold {
            self.handles.remove(&old);
        }
        message.receive_count = message.receive_count.saturating_add(1);
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.hold = Hold::InFlight {
            until: now.saturating_add(visibility_timeout),
            handle,
        };
        self.handles.insert(handle, arrival);
        Delivery {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            placement: message.placement.clone(),
        }
    }

    /// The message `handle` stands for, if the handle is still its latest and
    /// the message is in flight.
    fn in_flight(&self, handle: ReceiptHandle, now: Seconds) -> Result<Arrival, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|message| message.state(now) == State::InFlight)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is not the handle of a message in flight in this queue"),
                )
            })
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Seconds) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    /// Ends the visibility timeout `timeout` seconds from `now`.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Seconds,
        timeout: Seconds,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("an in-flight arrival is in the queue");
        message.hold = Hold::InFlight {
            until: now.saturating_add(timeout),
            handle,
        };
        Ok(())
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DeduplicationId;

    fn standard() -> Queue {
        Queue::new(Attributes::create(&AttributeValues::default(), None))
    }

    fn fifo(content_based_deduplication: bool) -> Queue {
        let values = AttributeValues {
            fifo: Some(true),
            content_based_deduplication: Some(content_based_deduplication),
            ..AttributeValues::default()
        };
        Queue::new(Attributes::create(&values, None))
    }

    fn outgoing(body: &str) -> Outgoing {
        Outgoing {
            body: body.to_owned(),
            delay: None,
            group: None,
            deduplication_id: None,
        }
    }

    fn fifo_outgoing(body: &str, group: &str, dedup: &str) -> Outgoing {
        Outgoing {
            group: Some(GroupId::parse(group).unwrap()),
            deduplication_id: Some(DeduplicationId::parse(dedup).unwrap()),
            ..outgoing(body)
        }
    }

    /// Hands out ids `m-1`, `m-2`, ... and handles `r-1`, `r-2`, ... for a test.
    struct Ids {
        messages: u64,
        handles: u64,
    }

    impl Ids {
        fn new() -> Self {
            Self {
                messages: 0,
                handles: 0,
            }
        }

        fn send(&mut self, queue: &mut Queue, now: u64, outgoing: Outgoing) -> Result<Sent, Error> {
            let messages = &mut self.messages;
            queue.send(Seconds::new(now), outgoing, || {
                *messages += 1;
                MessageId::new(*messages)
            })
        }

        fn receive(&mut self, queue: &mut Queue, now: u64, max: u32, visibility: u64) -> Received {
            let handles = &mut self.handles;
            queue.receive(Seconds::new(now), max, Seconds::new(visibility), || {
                *handles += 1;
                ReceiptHandle::new(*handles)
            })
        }
    }

    fn bodies(received: &Received) -> Vec<&str> {
        received.delivered.iter().map(|d| d.body.as_str()).collect()
    }

    #[test]
    fn delayed_message_becomes_visible_when_the_delay_ends() {
        let mut ids = Ids::new();
        let mut queue = standard();
        let delayed = Outgoing {
            delay: Some(Seconds::new(10)),
            ..outgoing("a")
        };
        ids.send(&mut queue, 0, delayed).unwrap();
        assert_eq!(
            queue.counts(Seconds::new(9)),
            Counts {
                visible: 0,
                in_flight: 0,
                delayed: 1
            }
        );
        assert_eq!(
            queue.counts(Seconds::new(10)),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
        assert_eq!(bodies(&ids.receive(&mut queue, 9, 1, 30)), [] as [&str; 0]);
        assert_eq!(bodies(&ids.receive(&mut queue, 10, 1, 30)), ["a"]);
    }

    #[test]
    fn message_keeps_its_place_after_its_visibility_timeout_ends() {
        let mut ids = Ids::new();
        let mut queue = standard();
        for body in ["a", "b"] {
            ids.send(&mut queue, 0, outgoing(body)).unwrap();
        }
        assert_eq!(bodies(&ids.receive(&mut queue, 0, 1, 5)), ["a"]);
        assert_eq!(bodies(&ids.receive(&mut queue, 1, 1, 5)), ["b"]);
        assert_eq!(bodies(&ids.receive(&mut queue, 5, 2, 5)), ["a"]);
    }

    #[test]
    fn redelivery_counts_receives_and_keeps_first_received_at() {
        let mut ids = Ids::new();
        let mut queue = standard();
        ids.send(&mut queue, 0, outgoing("a")).unwrap();
        let first = ids.receive(&mut queue, 2, 1, 0).delivered;
        let second = ids.receive(&mut queue, 7, 1, 0).delivered;
        assert_eq!(
            (first[0].receive_count, first[0].first_received_at),
            (1, Seconds::new(2))
        );
        assert_eq!(
            (second[0].receive_count, second[0].first_received_at),
            (2, Seconds::new(2))
        );
    }

    #[test]
    fn fifo_delivers_one_message_per_group_and_blocks_busy_groups() {
        let mut ids = Ids::new();
        let mut queue = fifo(false);
        for (body, group, dedup) in [("a1", "a", "1"), ("a2", "a", "2"), ("b1", "b", "3")] {
            ids.send(&mut queue, 0, fifo_outgoing(body, group, dedup))
                .unwrap();
        }
        assert_eq!(bodies(&ids.receive(&mut queue, 0, 10, 30)), ["a1", "b1"]);
        assert_eq!(bodies(&ids.receive(&mut queue, 1, 10, 30)), [] as [&str; 0]);
        assert_eq!(bodies(&ids.receive(&mut queue, 30, 10, 30)), ["a1", "b1"]);
    }

    #[test]
    fn fifo_sequence_numbers_count_accepted_messages() {
        let mut ids = Ids::new();
        let mut queue = fifo(false);
        let first = ids
            .send(&mut queue, 0, fifo_outgoing("x", "g", "1"))
            .unwrap();
        let second = ids
            .send(&mut queue, 0, fifo_outgoing("y", "g", "2"))
            .unwrap();
        assert_eq!(first.sequence_number, Some(SequenceNumber::new(1)));
        assert_eq!(second.sequence_number, Some(SequenceNumber::new(2)));
    }

    #[test]
    fn duplicate_send_returns_the_original_and_uses_no_id() {
        let mut ids = Ids::new();
        let mut queue = fifo(false);
        let original = ids
            .send(&mut queue, 0, fifo_outgoing("x", "g", "d"))
            .unwrap();
        let duplicate = ids
            .send(&mut queue, 299, fifo_outgoing("other", "h", "d"))
            .unwrap();
        assert_eq!(duplicate, original);
        let accepted = ids
            .send(&mut queue, 300, fifo_outgoing("x", "g", "d"))
            .unwrap();
        assert_eq!(accepted.message_id, MessageId::new(2));
        assert_eq!(accepted.sequence_number, Some(SequenceNumber::new(2)));
    }

    #[test]
    fn duplicate_window_outlives_the_message_and_a_purge() {
        let mut ids = Ids::new();
        let mut queue = fifo(false);
        let original = ids
            .send(&mut queue, 0, fifo_outgoing("x", "g", "d"))
            .unwrap();
        queue.purge();
        assert_eq!(
            ids.send(&mut queue, 1, fifo_outgoing("x", "g", "d"))
                .unwrap(),
            original
        );
        assert_eq!(queue.counts(Seconds::new(1)), Counts::default());
    }

    #[test]
    fn content_based_deduplication_uses_the_body() {
        let mut ids = Ids::new();
        let mut queue = fifo(true);
        let with_group = |body: &str| Outgoing {
            group: Some(GroupId::parse("g").unwrap()),
            ..outgoing(body)
        };
        let original = ids.send(&mut queue, 0, with_group("same")).unwrap();
        assert_eq!(
            ids.send(&mut queue, 1, with_group("same")).unwrap(),
            original
        );
        assert_ne!(
            ids.send(&mut queue, 1, with_group("different")).unwrap(),
            original
        );
    }

    #[test]
    fn send_rules_pick_the_documented_error() {
        let mut ids = Ids::new();
        let delayed_fifo = Outgoing {
            delay: Some(Seconds::new(1)),
            ..outgoing("x")
        };
        let grouped_standard = Outgoing {
            group: Some(GroupId::parse("g").unwrap()),
            ..outgoing("x")
        };
        let cases = [
            (
                standard(),
                grouped_standard,
                ErrorCode::UnsupportedOperation,
            ),
            (fifo(false), delayed_fifo, ErrorCode::UnsupportedOperation),
            (fifo(false), outgoing("x"), ErrorCode::MissingParameter),
            (
                fifo(false),
                Outgoing {
                    group: Some(GroupId::parse("g").unwrap()),
                    ..outgoing("x")
                },
                ErrorCode::MissingParameter,
            ),
        ];
        for (mut queue, request, code) in cases {
            let error = ids.send(&mut queue, 0, request).unwrap_err();
            assert_eq!(error.code(), code);
        }
        assert_eq!(ids.messages, 0);
    }

    #[test]
    fn queue_refuses_message_100001() {
        let mut ids = Ids::new();
        let mut queue = standard();
        for _ in 0..MESSAGE_LIMIT {
            ids.send(&mut queue, 0, outgoing("x")).unwrap();
        }
        let error = ids.send(&mut queue, 0, outgoing("x")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::LimitExceeded);
        assert_eq!(ids.messages, MESSAGE_LIMIT as u64);
    }

    #[test]
    fn retention_removes_messages_in_every_state() {
        let mut ids = Ids::new();
        let mut queue = standard();
        ids.send(&mut queue, 0, outgoing("a")).unwrap();
        ids.send(
            &mut queue,
            0,
            Outgoing {
                delay: Some(Seconds::new(900)),
                ..outgoing("b")
            },
        )
        .unwrap();
        ids.receive(&mut queue, 0, 1, 43_200);
        queue.attributes_mut().retention = Seconds::new(60);
        queue.expire(Seconds::new(59));
        assert_eq!(
            queue.counts(Seconds::new(59)),
            Counts {
                visible: 0,
                in_flight: 1,
                delayed: 1
            }
        );
        queue.expire(Seconds::new(60));
        assert_eq!(queue.counts(Seconds::new(60)), Counts::default());
    }

    #[test]
    fn delete_requires_the_latest_handle_of_a_message_in_flight() {
        let mut ids = Ids::new();
        let mut queue = standard();
        ids.send(&mut queue, 0, outgoing("a")).unwrap();
        let old = ids.receive(&mut queue, 0, 1, 5).delivered[0].receipt_handle;
        let new = ids.receive(&mut queue, 5, 1, 5).delivered[0].receipt_handle;
        let expired = queue.delete(old, Seconds::new(5)).unwrap_err();
        assert_eq!(expired.code(), ErrorCode::ReceiptHandleExpired);
        let after_timeout = queue.delete(new, Seconds::new(10)).unwrap_err();
        assert_eq!(after_timeout.code(), ErrorCode::ReceiptHandleExpired);
        queue.delete(new, Seconds::new(9)).unwrap();
        assert_eq!(queue.counts(Seconds::new(9)), Counts::default());
        let gone = queue.delete(new, Seconds::new(9)).unwrap_err();
        assert_eq!(gone.code(), ErrorCode::ReceiptHandleExpired);
    }

    #[test]
    fn change_visibility_moves_the_end_of_the_timeout() {
        let mut ids = Ids::new();
        let mut queue = standard();
        ids.send(&mut queue, 0, outgoing("a")).unwrap();
        let handle = ids.receive(&mut queue, 0, 1, 5).delivered[0].receipt_handle;
        queue
            .change_visibility(handle, Seconds::new(3), Seconds::new(10))
            .unwrap();
        assert_eq!(queue.counts(Seconds::new(12)).in_flight, 1);
        assert_eq!(queue.counts(Seconds::new(13)).visible, 1);
        queue
            .change_visibility(handle, Seconds::new(4), Seconds::ZERO)
            .unwrap();
        assert_eq!(queue.counts(Seconds::new(4)).visible, 1);
        let ended = queue
            .change_visibility(handle, Seconds::new(4), Seconds::new(1))
            .unwrap_err();
        assert_eq!(ended.code(), ErrorCode::ReceiptHandleExpired);
    }

    #[test]
    fn dead_letter_configuration_moves_messages_that_were_received_enough() {
        let mut ids = Ids::new();
        let values = AttributeValues::default();
        let dead_letter = DeadLetter {
            queue: QueueName::parse("dlq").unwrap(),
            max_receive_count: 1,
        };
        let mut queue = Queue::new(Attributes::create(&values, Some(dead_letter)));
        ids.send(&mut queue, 0, outgoing("a")).unwrap();
        ids.send(&mut queue, 0, outgoing("b")).unwrap();
        assert_eq!(bodies(&ids.receive(&mut queue, 0, 1, 0)), ["a"]);
        let second = ids.receive(&mut queue, 1, 1, 0);
        assert_eq!(bodies(&second), ["b"]);
        let moved = second.dead_lettered.unwrap();
        assert_eq!(moved.queue.as_str(), "dlq");
        assert_eq!(moved.entries.len(), 1);
        assert_eq!(moved.entries[0].body, "a");
        assert_eq!(
            queue.counts(Seconds::new(1)),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
    }
}
