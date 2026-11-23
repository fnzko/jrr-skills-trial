//! One queue: its settings, its messages in arrival order, and the rules for
//! sending, receiving, retention and deduplication. Queue-to-queue and
//! process-wide concerns (names, id counters) belong to the broker.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::deliveries::Deliveries;
use crate::error::{Error, ErrorCode};
use crate::request::NewMessage;
use crate::types::{
    Arrival, Body, Delay, GroupId, Instant, MaxMessages, MaxReceiveCount, MessageId, QueueId,
    QueueName, ReceiptHandle, Retention, Seconds, SequenceNumber, VisibilityTimeout,
};

const MESSAGE_LIMIT: usize = 100_000;

const PLANNED_ARRIVAL: &str = "a planned arrival is still queued: the plan was made on this state";

/// A send repeating a deduplication id is dropped while `now` is before the
/// time of the accepted send plus this window.
const DEDUPLICATION_WINDOW: Seconds<300, 300> = Seconds::constant(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Standard,
    Fifo { content_based_deduplication: bool },
}

/// A dead-letter configuration as a request names it; the target may not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterRequest {
    pub queue: String,
    pub max_receive_count: MaxReceiveCount,
}

/// What a creation request asks for, after defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSpec {
    pub queue_type: QueueType,
    pub visibility_timeout: VisibilityTimeout,
    pub delay: Delay,
    pub retention: Retention,
    pub dead_letter: Option<DeadLetterRequest>,
}

/// An existing queue that receives the messages another queue gives up on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

/// Attribute changes of `SetQueueAttributes`; `None` keeps the current value.
#[derive(Debug, Default)]
pub struct Update {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<Delay>,
    pub retention: Option<Retention>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: Option<DeadLetter>,
}

#[derive(Debug)]
struct Settings {
    visibility_timeout: VisibilityTimeout,
    delay: Delay,
    retention: Retention,
    dead_letter: Option<DeadLetter>,
}

/// Where a message sits in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Placement {
    group_id: GroupId,
    sequence_number: SequenceNumber,
}

#[derive(Debug)]
struct DeduplicationEntry {
    accepted_at: Instant,
    original: Sent,
}

#[derive(Debug)]
struct FifoState {
    content_based_deduplication: bool,
    next_sequence: SequenceNumber,
    deduplication: HashMap<String, DeduplicationEntry>,
}

impl FifoState {
    fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.succ();
        sequence
    }
}

#[derive(Debug)]
enum Kind {
    Standard,
    Fifo(FifoState),
}

#[derive(Debug, Clone, Copy)]
enum Availability {
    Visible,
    Delayed {
        until: Instant,
    },
    InFlight {
        until: Instant,
        handle: ReceiptHandle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    Delayed,
    InFlight,
}

impl Availability {
    fn state(self, now: Instant) -> State {
        match self {
            Self::Delayed { until } if now < until => State::Delayed,
            Self::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Instant,
    receive_count: u32,
    first_received_at: Option<Instant>,
    availability: Availability,
    placement: Option<Placement>,
}

impl Message {
    fn holds(&self, handle: ReceiptHandle, now: Instant) -> bool {
        matches!(
            self.availability,
            Availability::InFlight { until, handle: current } if current == handle && now < until
        )
    }

    fn deliver(
        &mut self,
        now: Instant,
        timeout: VisibilityTimeout,
        handle: ReceiptHandle,
    ) -> ReceivedMessage {
        self.receive_count = self.receive_count.saturating_add(1);
        let first_received_at = *self.first_received_at.get_or_insert(now);
        self.availability = Availability::InFlight {
            until: now.plus(timeout),
            handle,
        };
        ReceivedMessage {
            message_id: self.id,
            receipt_handle: handle,
            body: self.body.clone(),
            receive_count: self.receive_count,
            sent_at: self.sent_at,
            first_received_at,
            placement: self.placement.clone(),
        }
    }

    fn reset_for_dead_letter(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.availability = Availability::Visible;
    }
}

/// The identity a send returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Accepted(Sent),
    /// Nothing was stored; the identity is that of the original message.
    Duplicate(Sent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: Body,
    receive_count: u32,
    sent_at: Instant,
    first_received_at: Instant,
    #[serde(flatten)]
    placement: Option<Placement>,
}

/// Messages a receive moved out of a queue, with the queue to hand them to.
#[derive(Debug)]
pub struct DeadLettered {
    pub target: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub messages: Vec<ReceivedMessage>,
    pub dead_lettered: Option<DeadLettered>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The attributes `GetQueueAttributes` reports.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    visibility_timeout: VisibilityTimeout,
    #[serde(rename = "DelaySeconds")]
    delay: Delay,
    #[serde(rename = "RetentionSeconds")]
    retention: Retention,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Deliver,
    DeadLetter,
}

#[derive(Debug)]
pub struct Queue {
    id: QueueId,
    settings: Settings,
    kind: Kind,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: Arrival,
    /// A lower bound of the `sent_at` of all messages. Retention only scans
    /// the queue once this bound is reached, and then tightens it.
    oldest_sent_at: Option<Instant>,
}

impl Queue {
    pub fn new(id: QueueId, spec: &QueueSpec, dead_letter: Option<DeadLetter>) -> Self {
        let kind = match spec.queue_type {
            QueueType::Standard => Kind::Standard,
            QueueType::Fifo {
                content_based_deduplication,
            } => Kind::Fifo(FifoState {
                content_based_deduplication,
                next_sequence: SequenceNumber::FIRST,
                deduplication: HashMap::new(),
            }),
        };
        Self {
            id,
            settings: Settings {
                visibility_timeout: spec.visibility_timeout,
                delay: spec.delay,
                retention: spec.retention,
                dead_letter,
            },
            kind,
            messages: BTreeMap::new(),
            next_arrival: Arrival::FIRST,
            oldest_sent_at: None,
        }
    }

    pub fn id(&self) -> QueueId {
        self.id
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo(_))
    }

    /// Whether every attribute of `spec` equals the queue's current value.
    pub fn matches(&self, spec: &QueueSpec) -> bool {
        let same_type = match (&self.kind, spec.queue_type) {
            (Kind::Standard, QueueType::Standard) => true,
            (
                Kind::Fifo(fifo),
                QueueType::Fifo {
                    content_based_deduplication,
                },
            ) => fifo.content_based_deduplication == content_based_deduplication,
            _ => false,
        };
        let same_dead_letter = match (&self.settings.dead_letter, &spec.dead_letter) {
            (None, None) => true,
            (Some(have), Some(want)) => {
                have.queue.as_str() == want.queue
                    && have.max_receive_count == want.max_receive_count
            }
            _ => false,
        };
        same_type
            && same_dead_letter
            && self.settings.visibility_timeout == spec.visibility_timeout
            && self.settings.delay == spec.delay
            && self.settings.retention == spec.retention
    }

    pub fn attributes(&self) -> Attributes {
        Attributes {
            visibility_timeout: self.settings.visibility_timeout,
            delay: self.settings.delay,
            retention: self.settings.retention,
            fifo: self.is_fifo(),
            content_based_deduplication: matches!(
                &self.kind,
                Kind::Fifo(fifo) if fifo.content_based_deduplication
            ),
            dead_letter: self.settings.dead_letter.clone(),
        }
    }

    pub fn update(&mut self, update: Update) {
        let settings = &mut self.settings;
        settings.visibility_timeout = update
            .visibility_timeout
            .unwrap_or(settings.visibility_timeout);
        settings.delay = update.delay.unwrap_or(settings.delay);
        settings.retention = update.retention.unwrap_or(settings.retention);
        if update.dead_letter.is_some() {
            settings.dead_letter = update.dead_letter;
        }
        if let (Some(enabled), Kind::Fifo(fifo)) =
            (update.content_based_deduplication, &mut self.kind)
        {
            fifo.content_based_deduplication = enabled;
        }
    }

    /// Drops the dead-letter configuration if it points at `deleted`.
    pub fn forget_dead_letter(&mut self, deleted: &str) {
        if self
            .settings
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == deleted)
        {
            self.settings.dead_letter = None;
        }
    }

    /// Removes every message that retention has expired at `now`.
    pub fn expire(&mut self, now: Instant) {
        let retention = self.settings.retention;
        let Some(oldest) = self.oldest_sent_at else {
            return;
        };
        if now < oldest.plus(retention) {
            return;
        }
        self.messages
            .retain(|_, message| now < message.sent_at.plus(retention));
        self.oldest_sent_at = self.messages.values().map(|m| m.sent_at).min();
    }

    pub fn counts(&self, now: Instant) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.availability.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Removes all messages. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.oldest_sent_at = None;
    }

    fn insert(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival = arrival.succ();
        self.oldest_sent_at = Some(
            self.oldest_sent_at
                .map_or(message.sent_at, |oldest| oldest.min(message.sent_at)),
        );
        self.messages.insert(arrival, message);
    }

    /// Applies the send rules in their documented order. `id` is used only if
    /// the message is accepted.
    pub fn send(
        &mut self,
        now: Instant,
        id: MessageId,
        message: NewMessage,
    ) -> Result<SendOutcome, Error> {
        let NewMessage {
            body,
            delay,
            group,
            deduplication_id,
        } = message;
        let full = self.messages.len() >= MESSAGE_LIMIT;
        let limit_error = || {
            Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MESSAGE_LIMIT} messages"),
            )
        };
        let (sent, placement) = match &mut self.kind {
            Kind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "GroupId and DeduplicationId are only supported by FIFO queues",
                    ));
                }
                if full {
                    return Err(limit_error());
                }
                (
                    Sent {
                        message_id: id,
                        sequence_number: None,
                    },
                    None,
                )
            }
            Kind::Fifo(fifo) => {
                if delay.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "DelaySeconds cannot be set per message on a FIFO queue",
                    ));
                }
                let group_id = group.ok_or_else(|| {
                    Error::new(ErrorCode::MissingParameter, "GroupId is required")
                })?;
                let key = match (&deduplication_id, fifo.content_based_deduplication) {
                    (Some(explicit), _) => explicit.as_str(),
                    (None, true) => body.as_str(),
                    (None, false) => {
                        return Err(Error::new(
                            ErrorCode::MissingParameter,
                            "DeduplicationId is required",
                        ));
                    }
                };
                if let Some(entry) = fifo.deduplication.get(key)
                    && now < entry.accepted_at.plus(DEDUPLICATION_WINDOW)
                {
                    return Ok(SendOutcome::Duplicate(entry.original));
                }
                if full {
                    return Err(limit_error());
                }
                let sequence_number = fifo.take_sequence();
                let sent = Sent {
                    message_id: id,
                    sequence_number: Some(sequence_number),
                };
                fifo.deduplication.insert(
                    key.to_owned(),
                    DeduplicationEntry {
                        accepted_at: now,
                        original: sent,
                    },
                );
                (
                    sent,
                    Some(Placement {
                        group_id,
                        sequence_number,
                    }),
                )
            }
        };
        let delay = delay.unwrap_or(self.settings.delay);
        let availability = if delay.is_zero() {
            Availability::Visible
        } else {
            Availability::Delayed {
                until: now.plus(delay),
            }
        };
        self.insert(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            availability,
            placement,
        });
        Ok(SendOutcome::Accepted(sent))
    }

    /// Takes in a message another queue gave up on. The limit and the
    /// deduplication ids are not consulted; retention still applies.
    pub fn accept_dead_letter(&mut self, now: Instant, mut message: Message) {
        message.reset_for_dead_letter();
        if let (Kind::Fifo(fifo), Some(placement)) = (&mut self.kind, &mut message.placement) {
            placement.sequence_number = fifo.take_sequence();
        }
        if now >= message.sent_at.plus(self.settings.retention) {
            return;
        }
        self.insert(message);
    }

    /// Decides what a receive does, without changing anything: messages are
    /// examined once each in arrival order until `max` are delivered.
    fn plan_receive(&self, now: Instant, max: MaxMessages) -> Vec<(Arrival, Action)> {
        let limit = self
            .settings
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count.get());
        let mut groups_in_queue = HashSet::new();
        let mut plan = Vec::new();
        let mut delivered = 0;
        for (&arrival, message) in &self.messages {
            if delivered == max.get() {
                break;
            }
            let group = message.placement.as_ref().map(|p| &p.group_id);
            if let Some(group) = group
                && !groups_in_queue.insert(group)
            {
                continue;
            }
            if message.availability.state(now) != State::Visible {
                continue;
            }
            if limit.is_some_and(|limit| message.receive_count >= limit) {
                // A message that leaves the queue no longer blocks its group.
                if let Some(group) = group {
                    groups_in_queue.remove(group);
                }
                plan.push((arrival, Action::DeadLetter));
            } else {
                plan.push((arrival, Action::Deliver));
                delivered += 1;
            }
        }
        plan
    }

    pub fn receive(
        &mut self,
        now: Instant,
        max: MaxMessages,
        timeout: Option<VisibilityTimeout>,
        deliveries: &mut Deliveries,
    ) -> Received {
        let timeout = timeout.unwrap_or(self.settings.visibility_timeout);
        let mut messages = Vec::new();
        let mut dead = Vec::new();
        for (arrival, action) in self.plan_receive(now, max) {
            match action {
                Action::Deliver => {
                    let handle = deliveries.issue(self.id, arrival);
                    let message = self.messages.get_mut(&arrival).expect(PLANNED_ARRIVAL);
                    messages.push(message.deliver(now, timeout, handle));
                }
                Action::DeadLetter => {
                    dead.push(self.messages.remove(&arrival).expect(PLANNED_ARRIVAL));
                }
            }
        }
        let dead_lettered = match &self.settings.dead_letter {
            Some(dead_letter) if !dead.is_empty() => Some(DeadLettered {
                target: dead_letter.queue.clone(),
                messages: dead,
            }),
            _ => None,
        };
        Received {
            messages,
            dead_lettered,
        }
    }

    fn expired_handle(handle: ReceiptHandle) -> Error {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            format!(
                "receipt handle {handle} is no longer the current delivery of an in-flight message"
            ),
        )
    }

    pub fn delete(
        &mut self,
        now: Instant,
        arrival: Arrival,
        handle: ReceiptHandle,
    ) -> Result<(), Error> {
        match self.messages.entry(arrival) {
            Entry::Occupied(entry) if entry.get().holds(handle, now) => {
                entry.remove();
                Ok(())
            }
            _ => Err(Self::expired_handle(handle)),
        }
    }

    pub fn change_visibility(
        &mut self,
        now: Instant,
        arrival: Arrival,
        handle: ReceiptHandle,
        timeout: VisibilityTimeout,
    ) -> Result<(), Error> {
        let message = self
            .messages
            .get_mut(&arrival)
            .filter(|message| message.holds(handle, now))
            .ok_or_else(|| Self::expired_handle(handle))?;
        message.availability = Availability::InFlight {
            until: now.plus(timeout),
            handle,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(queue_type: QueueType) -> QueueSpec {
        QueueSpec {
            queue_type,
            visibility_timeout: VisibilityTimeout::constant(30),
            delay: Delay::constant(0),
            retention: Retention::constant(345_600),
            dead_letter: None,
        }
    }

    fn standard() -> Queue {
        Queue::new(QueueId::FIRST, &spec(QueueType::Standard), None)
    }

    fn fifo(content_based_deduplication: bool) -> Queue {
        Queue::new(
            QueueId::FIRST,
            &spec(QueueType::Fifo {
                content_based_deduplication,
            }),
            None,
        )
    }

    fn at(seconds: u32) -> Instant {
        Instant::START.plus(crate::types::TickSeconds::new(i64::from(seconds)).unwrap())
    }

    fn new_message(body: &str) -> NewMessage {
        NewMessage {
            body: Body::parse(body).unwrap(),
            delay: None,
            group: None,
            deduplication_id: None,
        }
    }

    fn fifo_message(body: &str, group: &str, id: Option<&str>) -> NewMessage {
        NewMessage {
            group: Some(GroupId::parse(group).unwrap()),
            deduplication_id: id.map(|id| crate::types::DeduplicationId::parse(id).unwrap()),
            ..new_message(body)
        }
    }

    fn id(n: u32) -> MessageId {
        (1..n).fold(MessageId::FIRST, |id, _| id.succ())
    }

    fn code<T: std::fmt::Debug>(result: Result<T, Error>) -> ErrorCode {
        result.unwrap_err().code()
    }

    #[test]
    fn standard_queue_rejects_fifo_parameters() {
        let mut queue = standard();
        let mut message = new_message("x");
        message.group = Some(GroupId::parse("g").unwrap());
        assert_eq!(
            code(queue.send(at(0), id(1), message)),
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(queue.counts(at(0)), Counts::default());
    }

    #[test]
    fn fifo_send_requires_group_then_deduplication_id() {
        let mut queue = fifo(false);
        assert_eq!(
            code(queue.send(at(0), id(1), new_message("x"))),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(queue.send(at(0), id(1), fifo_message("x", "g", None))),
            ErrorCode::MissingParameter
        );
        let mut delayed = fifo_message("x", "g", Some("d"));
        delayed.delay = Some(Delay::constant(1));
        assert_eq!(
            code(queue.send(at(0), id(1), delayed)),
            ErrorCode::UnsupportedOperation
        );
    }

    #[test]
    fn duplicate_send_returns_the_original_identity_inside_the_window_only() {
        let mut queue = fifo(false);
        let first = queue
            .send(at(0), id(1), fifo_message("a", "g", Some("d")))
            .unwrap();
        let original = Sent {
            message_id: id(1),
            sequence_number: Some(SequenceNumber::FIRST),
        };
        assert_eq!(first, SendOutcome::Accepted(original));

        // A duplicate does not extend the window.
        let inside = queue
            .send(at(299), id(2), fifo_message("b", "other", Some("d")))
            .unwrap();
        assert_eq!(inside, SendOutcome::Duplicate(original));

        let outside = queue
            .send(at(300), id(2), fifo_message("c", "g", Some("d")))
            .unwrap();
        assert_eq!(
            outside,
            SendOutcome::Accepted(Sent {
                message_id: id(2),
                sequence_number: Some(SequenceNumber::FIRST.succ()),
            })
        );
        assert_eq!(queue.counts(at(300)).visible, 2);
    }

    #[test]
    fn content_based_deduplication_uses_the_body() {
        let mut queue = fifo(true);
        let send = |queue: &mut Queue, n, time| {
            queue
                .send(at(time), id(n), fifo_message("same body", "g", None))
                .unwrap()
        };
        assert!(matches!(send(&mut queue, 1, 0), SendOutcome::Accepted(_)));
        assert!(matches!(send(&mut queue, 2, 10), SendOutcome::Duplicate(_)));
    }

    #[test]
    fn retention_removes_messages_at_the_deadline_in_any_state() {
        let mut queue = standard();
        queue.send(at(0), id(1), new_message("a")).unwrap();
        queue.send(at(100), id(2), new_message("b")).unwrap();
        queue.update(Update {
            retention: Some(Retention::constant(100)),
            ..Update::default()
        });
        queue.expire(at(99));
        assert_eq!(queue.counts(at(99)).visible, 2);
        queue.expire(at(100));
        assert_eq!(queue.counts(at(100)).visible, 1);
        queue.expire(at(200));
        assert_eq!(queue.counts(at(200)), Counts::default());
    }

    #[test]
    fn receive_marks_messages_in_flight_until_the_timeout() {
        let mut queue = standard();
        let mut deliveries = Deliveries::default();
        queue.send(at(0), id(1), new_message("a")).unwrap();
        let first = queue.receive(at(5), MaxMessages::constant(1), None, &mut deliveries);
        assert_eq!(first.messages.len(), 1);
        assert_eq!(first.messages[0].receive_count, 1);
        assert_eq!(queue.counts(at(34)).in_flight, 1);
        assert_eq!(queue.counts(at(35)).visible, 1);
        let again = queue.receive(at(35), MaxMessages::constant(1), None, &mut deliveries);
        assert_eq!(again.messages[0].receive_count, 2);
        assert_eq!(again.messages[0].first_received_at, at(5));
    }

    #[test]
    fn fifo_receive_delivers_one_message_per_group_in_arrival_order() {
        let mut queue = fifo(false);
        let mut deliveries = Deliveries::default();
        for (n, (body, group)) in [("a1", "a"), ("a2", "a"), ("b1", "b")]
            .into_iter()
            .enumerate()
        {
            let n = u32::try_from(n).unwrap() + 1;
            queue
                .send(at(0), id(n), fifo_message(body, group, Some(body)))
                .unwrap();
        }
        let received = queue.receive(at(0), MaxMessages::constant(10), None, &mut deliveries);
        let bodies: Vec<&str> = received.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["a1", "b1"]);
    }

    #[test]
    fn dead_lettering_unblocks_the_group_within_the_same_receive() {
        let dead_letter = DeadLetter {
            queue: QueueName::parse("dead.fifo").unwrap(),
            max_receive_count: MaxReceiveCount::constant(1),
        };
        let mut queue = Queue::new(
            QueueId::FIRST,
            &spec(QueueType::Fifo {
                content_based_deduplication: false,
            }),
            Some(dead_letter),
        );
        let mut deliveries = Deliveries::default();
        queue
            .send(at(0), id(1), fifo_message("first", "g", Some("1")))
            .unwrap();
        queue
            .send(at(0), id(2), fifo_message("second", "g", Some("2")))
            .unwrap();
        let timeout = Some(VisibilityTimeout::constant(0));
        queue.receive(at(0), MaxMessages::constant(1), timeout, &mut deliveries);

        let received = queue.receive(at(0), MaxMessages::constant(1), None, &mut deliveries);
        let bodies: Vec<&str> = received.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["second"]);
        let moved = received.dead_lettered.unwrap();
        assert_eq!(moved.target.as_str(), "dead.fifo");
        assert_eq!(moved.messages.len(), 1);
    }

    #[test]
    fn delete_requires_the_latest_live_handle() {
        let mut queue = standard();
        let mut deliveries = Deliveries::default();
        queue.send(at(0), id(1), new_message("a")).unwrap();
        let first = queue.receive(
            at(0),
            MaxMessages::constant(1),
            Some(VisibilityTimeout::constant(5)),
            &mut deliveries,
        );
        let old = first.messages[0].receipt_handle;
        let arrival = Arrival::FIRST;
        let second = queue.receive(at(5), MaxMessages::constant(1), None, &mut deliveries);
        let current = second.messages[0].receipt_handle;

        assert_eq!(
            code(queue.delete(at(6), arrival, old)),
            ErrorCode::ReceiptHandleExpired
        );
        assert_eq!(
            code(queue.delete(at(35), arrival, current)),
            ErrorCode::ReceiptHandleExpired
        );
        queue.delete(at(34), arrival, current).unwrap();
        assert_eq!(queue.counts(at(34)), Counts::default());
    }
}
