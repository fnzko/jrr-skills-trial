//! One queue: its attributes, its messages and the rules that act on them.
//!
//! All times are virtual seconds. A queue never reads a clock; callers pass
//! `now`, and call [`Queue::expire`] before they look at the messages so
//! that retention holds at every moment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ids::{
    DedupKey, GroupId, MessageId, QueueName, ReceiptHandle, ReceiptHandles, SequenceNumber,
};
use crate::request::AttributeInput;
use crate::response::{AttributesView, Counts, ReceivedMessage};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUP_WINDOW: u64 = 300;
const DEFAULT_VISIBILITY_TIMEOUT: u32 = 30;
const DEFAULT_RETENTION_SECONDS: u32 = 345_600;

/// The type of a queue, which never changes after creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    /// The kind a `CreateQueue` request asks for. A true
    /// `ContentBasedDeduplication` on a standard queue is rejected by the
    /// caller, so it is not represented here.
    pub fn requested(input: &AttributeInput) -> Self {
        if input.fifo == Some(true) {
            Self::Fifo {
                content_based_deduplication: input.content_based_deduplication == Some(true),
            }
        } else {
            Self::Standard
        }
    }

    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

/// Where messages that were received too often go, and what "too often" is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

impl Attributes {
    /// Attributes for a new queue: the given ones, defaults for the rest.
    pub fn new(input: &AttributeInput, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: input
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: input.delay_seconds.unwrap_or(0),
            retention_seconds: input.retention_seconds.unwrap_or(DEFAULT_RETENTION_SECONDS),
            kind: Kind::requested(input),
            dead_letter,
        }
    }

    /// Whether a `CreateQueue` request, after defaults, equals these attributes.
    pub fn matches(&self, input: &AttributeInput) -> bool {
        let same_dead_letter = match (
            &self.dead_letter,
            &input.dead_letter_queue,
            input.max_receive_count,
        ) {
            (None, None, None) => true,
            (Some(own), Some(queue), Some(max)) => {
                own.queue.as_str() == queue && own.max_receive_count == max
            }
            _ => false,
        };
        self.visibility_timeout
            == input
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay_seconds == input.delay_seconds.unwrap_or(0)
            && self.retention_seconds
                == input.retention_seconds.unwrap_or(DEFAULT_RETENTION_SECONDS)
            && self.kind == Kind::requested(input)
            && same_dead_letter
    }

    /// Takes over the attributes present in `input`. `dead_letter` is the
    /// resolved replacement, if the request has one. The caller has already
    /// rejected `Fifo` and a true `ContentBasedDeduplication` on a standard queue.
    pub fn apply(&mut self, input: &AttributeInput, dead_letter: Option<DeadLetter>) {
        if let Some(value) = input.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = input.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = input.retention_seconds {
            self.retention_seconds = value;
        }
        if let (
            Kind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, input.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    pub fn view(&self) -> AttributesView {
        AttributesView {
            visibility_timeout: self.visibility_timeout,
            delay_seconds: self.delay_seconds,
            retention_seconds: self.retention_seconds,
            fifo: self.kind.is_fifo(),
            content_based_deduplication: matches!(
                self.kind,
                Kind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter_queue: self.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: self.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// Position in a queue's arrival order; never reused within the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
struct InFlight {
    until: u64,
    handle: ReceiptHandle,
}

#[derive(Debug)]
struct FifoTag {
    group: GroupId,
    sequence: SequenceNumber,
}

/// A message together with its delivery bookkeeping.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    delayed_until: u64,
    in_flight: Option<InFlight>,
    fifo: Option<FifoTag>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self
            .in_flight
            .as_ref()
            .is_some_and(|flight| now < flight.until)
        {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What a queue needs to accept a message.
#[derive(Debug)]
pub struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub sent_at: u64,
    pub delay_seconds: u32,
    /// Present exactly when the queue is FIFO.
    pub group: Option<GroupId>,
    /// Recorded for deduplication when present; only FIFO sends have one.
    pub deduplication: Option<DedupKey>,
}

/// The original of a message that a duplicate send is answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: SequenceNumber,
}

#[derive(Debug)]
struct DedupRecord {
    at: u64,
    accepted: Accepted,
}

#[derive(Debug)]
struct FifoState {
    last_sequence: u64,
    dedup: HashMap<DedupKey, DedupRecord>,
}

enum Action {
    Deliver,
    DeadLetter,
}

/// The outcome of one `ReceiveMessage` scan.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<ReceivedMessage>,
    /// Messages removed from this queue that belong in the dead-letter queue.
    pub dead: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    last_arrival: u64,
    /// `(sent_at, arrival)` of every message, so expiry finds the oldest in O(log n).
    by_sent_at: BTreeSet<(u64, Arrival)>,
    /// Latest receipt handle of every delivered message still in the queue.
    handles: HashMap<ReceiptHandle, Arrival>,
    /// Present exactly when `attributes.kind` is FIFO.
    fifo: Option<FifoState>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        let fifo = attributes.kind.is_fifo().then(|| FifoState {
            last_sequence: 0,
            dedup: HashMap::new(),
        });
        Self {
            attributes,
            messages: BTreeMap::new(),
            last_arrival: 0,
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            fifo,
        }
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Removes every message whose retention has run out at `now`.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.attributes.retention_seconds);
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(arrival);
        }
    }

    /// Removes the message at `arrival`, which every caller took from this queue.
    fn remove(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrival was taken from this queue's messages");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(flight) = &message.in_flight {
            self.handles.remove(&flight.handle);
        }
        message
    }

    /// Removes every message; sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts {
            visible: 0,
            in_flight: 0,
            delayed: 0,
        };
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// The accepted message that makes a send with `key` a duplicate at `now`.
    pub fn find_duplicate(&self, key: &DedupKey, now: u64) -> Option<Accepted> {
        let record = self.fifo.as_ref()?.dedup.get(key)?;
        (now - record.at < DEDUP_WINDOW).then_some(record.accepted)
    }

    /// Adds a message at the end of the queue and returns its sequence
    /// number if the queue is FIFO.
    pub fn enqueue(&mut self, new: NewMessage) -> Option<SequenceNumber> {
        let tag = match (self.fifo.as_mut(), new.group) {
            (Some(state), Some(group)) => {
                state.last_sequence += 1;
                let sequence = SequenceNumber(state.last_sequence);
                if let Some(key) = new.deduplication {
                    let accepted = Accepted {
                        id: new.id,
                        sequence,
                    };
                    state.dedup.insert(
                        key,
                        DedupRecord {
                            at: new.sent_at,
                            accepted,
                        },
                    );
                }
                Some(FifoTag { group, sequence })
            }
            _ => None,
        };
        let sequence = tag.as_ref().map(|tag| tag.sequence);
        self.last_arrival += 1;
        let arrival = Arrival(self.last_arrival);
        self.by_sent_at.insert((new.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: new.id,
                body: new.body,
                sent_at: new.sent_at,
                receive_count: 0,
                first_received_at: None,
                delayed_until: new.sent_at + u64::from(new.delay_seconds),
                in_flight: None,
                fifo: tag,
            },
        );
        sequence
    }

    /// Accepts a message that was dead-lettered out of another queue.
    pub fn enqueue_dead(&mut self, message: Message) {
        let group = message.fifo.map(|tag| tag.group);
        self.enqueue(NewMessage {
            id: message.id,
            body: message.body,
            sent_at: message.sent_at,
            delay_seconds: 0,
            group,
            deduplication: None,
        });
    }

    /// Delivers up to `max_messages` messages, moving the ones that exceed
    /// the dead-letter limit into `Received::dead` instead.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: u32,
        visibility_timeout: u32,
        handles: &mut ReceiptHandles,
    ) -> Received {
        let plan = self.plan_receive(now, max_messages);
        let mut received = Received {
            delivered: Vec::new(),
            dead: Vec::new(),
        };
        for (arrival, action) in plan {
            match action {
                Action::DeadLetter => received.dead.push(self.remove(arrival)),
                Action::Deliver => {
                    let handle = handles.issue();
                    received
                        .delivered
                        .push(self.deliver(arrival, handle, now, visibility_timeout));
                }
            }
        }
        received
    }

    /// Examines the messages in arrival order, as `ReceiveMessage` defines.
    fn plan_receive(&self, now: u64, max_messages: u32) -> Vec<(Arrival, Action)> {
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        // Groups that still have an earlier message in the queue.
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        let mut plan = Vec::new();
        let mut delivering = 0;
        for (&arrival, message) in &self.messages {
            let group = message.fifo.as_ref().map(|tag| &tag.group);
            let eligible = message.state(now) == State::Visible
                && group.is_none_or(|group| !blocked.contains(group));
            if eligible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                plan.push((arrival, Action::DeadLetter));
                continue;
            }
            if eligible {
                plan.push((arrival, Action::Deliver));
                delivering += 1;
            }
            blocked.extend(group);
            if delivering == max_messages {
                break;
            }
        }
        plan
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        handle: ReceiptHandle,
        now: u64,
        visibility_timeout: u32,
    ) -> ReceivedMessage {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("planned arrivals are still in the queue");
        if let Some(previous) = message.in_flight.replace(InFlight {
            until: now + u64::from(visibility_timeout),
            handle,
        }) {
            self.handles.remove(&previous.handle);
        }
        self.handles.insert(handle, arrival);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        ReceivedMessage {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.fifo.as_ref().map(|tag| tag.group.clone()),
            sequence_number: message.fifo.as_ref().map(|tag| tag.sequence),
        }
    }

    /// The message whose latest handle is `handle`, if it is in flight at `now`.
    fn in_flight_arrival(&self, handle: ReceiptHandle, now: u64) -> Option<Arrival> {
        let arrival = *self.handles.get(&handle)?;
        let message = self
            .messages
            .get(&arrival)
            .expect("handles only refer to messages in the queue");
        (message.state(now) == State::InFlight).then_some(arrival)
    }

    /// Deletes the message `handle` refers to; false if the handle is not usable.
    pub fn delete_in_flight(&mut self, handle: ReceiptHandle, now: u64) -> bool {
        let Some(arrival) = self.in_flight_arrival(handle, now) else {
            return false;
        };
        self.remove(arrival);
        true
    }

    /// Moves the end of visibility of the message `handle` refers to to
    /// `now + visibility_timeout`; false if the handle is not usable.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: u64,
        visibility_timeout: u32,
    ) -> bool {
        let Some(arrival) = self.in_flight_arrival(handle, now) else {
            return false;
        };
        let flight = self
            .messages
            .get_mut(&arrival)
            .and_then(|message| message.in_flight.as_mut())
            .expect("an in-flight message has an in-flight record");
        flight.until = now + u64::from(visibility_timeout);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(kind: Kind) -> Attributes {
        let input = AttributeInput {
            fifo: Some(kind.is_fifo()),
            ..AttributeInput::default()
        };
        let mut attributes = Attributes::new(&input, None);
        attributes.kind = kind;
        attributes
    }

    fn standard_message(id: u64, sent_at: u64, delay_seconds: u32) -> NewMessage {
        NewMessage {
            id: MessageId(id),
            body: format!("body {id}"),
            sent_at,
            delay_seconds,
            group: None,
            deduplication: None,
        }
    }

    fn fifo_message(id: u64, group: &str) -> NewMessage {
        NewMessage {
            group: Some(GroupId::parse(group).unwrap()),
            ..standard_message(id, 0, 0)
        }
    }

    fn ids(received: &Received) -> Vec<MessageId> {
        received.delivered.iter().map(|m| m.message_id).collect()
    }

    #[test]
    fn expire_removes_messages_at_exactly_their_retention() {
        let mut queue = Queue::new(attributes(Kind::Standard));
        queue.attributes.retention_seconds = 60;
        queue.enqueue(standard_message(1, 0, 0));
        queue.enqueue(standard_message(2, 10, 0));
        queue.expire(59);
        assert_eq!(queue.message_count(), 2);
        queue.expire(60);
        assert_eq!(queue.message_count(), 1);
        queue.expire(70);
        assert_eq!(queue.message_count(), 0);
    }

    #[test]
    fn states_follow_delay_and_visibility_timeout() {
        let mut queue = Queue::new(attributes(Kind::Standard));
        let mut handles = ReceiptHandles::default();
        queue.enqueue(standard_message(1, 0, 5));
        queue.enqueue(standard_message(2, 0, 0));
        assert_eq!(
            queue.counts(0),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 1
            }
        );
        let received = queue.receive(0, 10, 10, &mut handles);
        assert_eq!(ids(&received), [MessageId(2)]);
        assert_eq!(
            queue.counts(0),
            Counts {
                visible: 0,
                in_flight: 1,
                delayed: 1
            }
        );
        assert_eq!(
            queue.counts(5),
            Counts {
                visible: 1,
                in_flight: 1,
                delayed: 0
            }
        );
        assert_eq!(
            queue.counts(10),
            Counts {
                visible: 2,
                in_flight: 0,
                delayed: 0
            }
        );
    }

    #[test]
    fn redelivery_keeps_arrival_order_and_expires_the_old_handle() {
        let mut queue = Queue::new(attributes(Kind::Standard));
        let mut handles = ReceiptHandles::default();
        queue.enqueue(standard_message(1, 0, 0));
        queue.enqueue(standard_message(2, 0, 0));
        let first = queue.receive(0, 1, 0, &mut handles);
        let second = queue.receive(0, 1, 0, &mut handles);
        assert_eq!(ids(&first), [MessageId(1)]);
        assert_eq!(ids(&second), [MessageId(1)]);
        assert_eq!(second.delivered[0].receive_count, 2);
        let old = first.delivered[0].receipt_handle;
        assert!(!queue.delete_in_flight(old, 0));
    }

    #[test]
    fn fifo_delivers_one_message_per_group_and_blocks_behind_in_flight() {
        let mut queue = Queue::new(attributes(Kind::Fifo {
            content_based_deduplication: false,
        }));
        let mut handles = ReceiptHandles::default();
        queue.enqueue(fifo_message(1, "a"));
        queue.enqueue(fifo_message(2, "a"));
        queue.enqueue(fifo_message(3, "b"));
        let first = queue.receive(0, 10, 30, &mut handles);
        assert_eq!(ids(&first), [MessageId(1), MessageId(3)]);
        let second = queue.receive(0, 10, 30, &mut handles);
        assert_eq!(ids(&second), Vec::<MessageId>::new());
        assert!(queue.delete_in_flight(first.delivered[0].receipt_handle, 0));
        let third = queue.receive(0, 10, 30, &mut handles);
        assert_eq!(ids(&third), [MessageId(2)]);
    }

    #[test]
    fn dead_lettered_message_stops_blocking_its_group_in_the_same_scan() {
        let mut queue = Queue::new(attributes(Kind::Fifo {
            content_based_deduplication: false,
        }));
        queue.attributes.dead_letter = Some(DeadLetter {
            queue: QueueName::parse("dlq.fifo").unwrap(),
            max_receive_count: 1,
        });
        let mut handles = ReceiptHandles::default();
        queue.enqueue(fifo_message(1, "a"));
        queue.enqueue(fifo_message(2, "a"));
        assert_eq!(ids(&queue.receive(0, 1, 0, &mut handles)), [MessageId(1)]);
        let received = queue.receive(0, 1, 0, &mut handles);
        assert_eq!(ids(&received), [MessageId(2)]);
        assert_eq!(received.dead.len(), 1);
        assert_eq!(received.dead[0].id, MessageId(1));
        assert_eq!(queue.message_count(), 1);
    }

    #[test]
    fn duplicate_window_is_300_seconds_and_survives_purge() {
        let mut queue = Queue::new(attributes(Kind::Fifo {
            content_based_deduplication: false,
        }));
        let key = DedupKey::from_id("d").unwrap();
        let mut message = fifo_message(1, "g");
        message.deduplication = Some(key.clone());
        assert_eq!(queue.enqueue(message), Some(SequenceNumber(1)));
        queue.purge();
        let original = queue.find_duplicate(&key, 299).unwrap();
        assert_eq!(
            (original.id, original.sequence),
            (MessageId(1), SequenceNumber(1))
        );
        assert_eq!(queue.find_duplicate(&key, 300), None);
    }

    #[test]
    fn change_visibility_to_zero_makes_the_message_visible() {
        let mut queue = Queue::new(attributes(Kind::Standard));
        let mut handles = ReceiptHandles::default();
        queue.enqueue(standard_message(1, 0, 0));
        let handle = queue.receive(0, 1, 30, &mut handles).delivered[0].receipt_handle;
        assert!(queue.change_visibility(handle, 5, 0));
        assert_eq!(
            queue.counts(5),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
        assert!(!queue.change_visibility(handle, 5, 10));
    }
}
