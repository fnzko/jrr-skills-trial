//! One queue: its messages, their states over virtual time, and the
//! deduplication memory of a FIFO queue.
//!
//! Messages live in a map keyed by *arrival* (a per-queue counter), which is
//! the arrival order of the spec. Beside it sit indexes that keep every
//! operation logarithmic instead of scanning a queue of up to 100000
//! messages:
//!
//! * `visible` and `hidden` split messages by state; `hidden` is ordered by
//!   the time a message turns visible, so [`Queue::sync`] only touches
//!   messages whose state actually changes;
//! * `by_sent_at` finds messages whose retention has run out;
//! * `groups` tells whether a message is the first of its FIFO group;
//! * `by_handle` finds the message a receipt handle belongs to.
//!
//! Time only moves between commands, so states are brought up to date
//! lazily: every public method that reads or changes messages expects the
//! caller to have called [`Queue::sync`] with the current time first.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use crate::config::Config;
use crate::error::{Error, ErrorCode};
use crate::ids::{DeduplicationId, GroupId, IdSource, MessageId, ReceiptHandle};
use crate::request::SendMessage;

/// Virtual time, in seconds since the start of the process.
pub type Time = u64;

const MAX_MESSAGES: usize = 100_000;
/// A FIFO send is a duplicate of an earlier one accepted less than this many
/// seconds ago.
const DEDUPLICATION_WINDOW: Time = 300;
const INDEXED: &str = "every indexed arrival has a message";

type Arrival = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    /// Hidden by a send delay until the given time.
    Delayed(Time),
    /// Hidden by a delivery until the given time.
    InFlight(Time),
}

impl State {
    fn hidden_until(self) -> Option<Time> {
        match self {
            Self::Visible => None,
            Self::Delayed(until) | Self::InFlight(until) => Some(until),
        }
    }
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u64,
    first_received_at: Option<Time>,
    group: Option<GroupId>,
    sequence: Option<u64>,
    state: State,
    /// The latest handle issued for this message.
    handle: Option<ReceiptHandle>,
}

impl Message {
    /// What a message looks like when it arrives in a dead-letter queue:
    /// as if never delivered, and visible at once.
    fn reset_for_dead_letter(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.state = State::Visible;
        self.handle = None;
        self.sequence = None;
    }
}

/// What the sender gets back for an accepted (or duplicate) send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub id: MessageId,
    /// Present on FIFO queues only.
    pub sequence: Option<u64>,
}

/// A message as handed to a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: Time,
    pub first_received_at: Time,
    pub group_id: Option<GroupId>,
    pub sequence: Option<u64>,
}

/// The outcome of one `ReceiveMessage`.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    /// Messages that hit `MaxReceiveCount`, in the order they must be added
    /// to the dead-letter queue. They have already left this queue.
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone, Copy)]
struct Original {
    at: Time,
    accepted: Accepted,
}

enum Admission {
    New,
    Duplicate(Accepted),
}

#[derive(Debug)]
pub struct Queue {
    config: Config,
    messages: BTreeMap<Arrival, Message>,
    visible: BTreeSet<Arrival>,
    /// `(time it turns visible, arrival)` of delayed and in-flight messages.
    hidden: BTreeSet<(Time, Arrival)>,
    by_sent_at: BTreeSet<(Time, Arrival)>,
    groups: HashMap<GroupId, BTreeSet<Arrival>>,
    by_handle: HashMap<ReceiptHandle, Arrival>,
    /// Latest accepted send per deduplication id. Entries outlive their
    /// messages by design and are only replaced, never dropped.
    deduplication: HashMap<String, Original>,
    next_arrival: Arrival,
    next_sequence: u64,
}

impl Queue {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            visible: BTreeSet::new(),
            hidden: BTreeSet::new(),
            by_sent_at: BTreeSet::new(),
            groups: HashMap::new(),
            by_handle: HashMap::new(),
            deduplication: HashMap::new(),
            next_arrival: 0,
            next_sequence: 1,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replaces the configuration. Messages are brought up to date with the
    /// *old* configuration first, so a retention that was lowered and raised
    /// again between two accesses cannot resurrect anything.
    pub fn set_config(&mut self, now: Time, config: Config) {
        self.sync(now);
        self.config = config;
        self.sync(now);
    }

    pub fn clear_dead_letter(&mut self) {
        self.config.dead_letter = None;
    }

    /// Applies the passage of time: messages whose delay or visibility
    /// timeout ended become visible, and messages past retention are removed.
    pub fn sync(&mut self, now: Time) {
        while let Some(&(until, arrival)) = self.hidden.first()
            && until <= now
        {
            self.set_state(arrival, State::Visible);
        }
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first()
            && sent_at + self.config.retention_seconds <= now
        {
            self.remove(arrival);
        }
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            visible: 0,
            in_flight: 0,
            delayed: 0,
        };
        for message in self.messages.values() {
            match message.state {
                State::Visible => counts.visible += 1,
                State::InFlight(_) => counts.in_flight += 1,
                State::Delayed(_) => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.visible.clear();
        self.hidden.clear();
        self.by_sent_at.clear();
        self.groups.clear();
        self.by_handle.clear();
    }

    pub fn send(
        &mut self,
        now: Time,
        request: &SendMessage,
        ids: &mut IdSource,
    ) -> Result<Accepted, Error> {
        if let Admission::Duplicate(original) = self.admit(now, request)? {
            return Ok(original);
        }

        let accepted = Accepted {
            id: ids.next_message(),
            sequence: self.take_sequence(),
        };
        let delay = request.delay_seconds.unwrap_or(self.config.delay_seconds);
        self.insert(Message {
            id: accepted.id,
            body: request.body.clone(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group: request.group_id.clone(),
            sequence: accepted.sequence,
            state: if delay > 0 {
                State::Delayed(now + delay)
            } else {
                State::Visible
            },
            handle: None,
        });
        if self.config.fifo {
            self.deduplication.insert(
                deduplication_id(request).to_owned(),
                Original { at: now, accepted },
            );
        }
        Ok(accepted)
    }

    /// The `SendMessage` rules of spec section 6 that depend on the queue.
    fn admit(&self, now: Time, request: &SendMessage) -> Result<Admission, Error> {
        let unsupported = if self.config.fifo {
            request.delay_seconds.is_some()
        } else {
            request.group_id.is_some() || request.deduplication_id.is_some()
        };
        if unsupported {
            return Err(Error::unsupported(if self.config.fifo {
                "FIFO queues do not support per-message DelaySeconds"
            } else {
                "standard queues do not support GroupId or DeduplicationId"
            }));
        }
        if self.config.fifo {
            if request.group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            if request.deduplication_id.is_none() && !self.config.content_based_deduplication {
                return Err(Error::missing("DeduplicationId"));
            }
            if let Some(original) = self.deduplication.get(deduplication_id(request))
                && now - original.at < DEDUPLICATION_WINDOW
            {
                return Ok(Admission::Duplicate(original.accepted));
            }
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("queue already holds {MAX_MESSAGES} messages"),
            ));
        }
        Ok(Admission::New)
    }

    /// Examines the messages once each in arrival order, as described for
    /// `ReceiveMessage` in the spec.
    ///
    /// Dead-lettering is decided and applied here, but adding the messages
    /// to the dead-letter queue is left to the caller, which owns the other
    /// queue. That is equivalent because a queue is never its own
    /// dead-letter queue.
    pub fn receive(
        &mut self,
        now: Time,
        max_messages: u64,
        visibility_timeout: Option<u64>,
        ids: &mut IdSource,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.config.visibility_timeout);
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut received = Received {
            delivered: Vec::new(),
            dead_lettered: Vec::new(),
        };

        // Walk by cursor rather than by iterator: delivering or moving a
        // message changes the indexes, and a dead-lettered group head makes
        // the next message of its group eligible within this same call.
        let mut cursor = Bound::Unbounded;
        while (received.delivered.len() as u64) < max_messages
            && let Some(&arrival) = self.visible.range((cursor, Bound::Unbounded)).next()
        {
            cursor = Bound::Excluded(arrival);
            if !self.is_first_of_group(arrival) {
                continue;
            }
            let receive_count = self.messages.get(&arrival).expect(INDEXED).receive_count;
            if max_receive_count.is_some_and(|max| receive_count >= max) {
                let mut message = self.remove(arrival);
                message.reset_for_dead_letter();
                received.dead_lettered.push(message);
            } else {
                received
                    .delivered
                    .push(self.deliver(now, arrival, visibility_timeout, ids));
            }
        }
        received
    }

    fn deliver(
        &mut self,
        now: Time,
        arrival: Arrival,
        visibility_timeout: u64,
        ids: &mut IdSource,
    ) -> Delivery {
        let handle = ids.next_handle();
        let message = self.messages.get_mut(&arrival).expect(INDEXED);
        if let Some(previous) = message.handle.replace(handle) {
            self.by_handle.remove(&previous);
        }
        self.by_handle.insert(handle, arrival);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        let delivery = Delivery {
            id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group.clone(),
            sequence: message.sequence,
        };
        self.set_state(arrival, in_flight_until(now, visibility_timeout));
        delivery
    }

    /// Adds a message moved from another queue. It is removed again at once
    /// if it is already past this queue's retention.
    pub fn accept_dead_letter(&mut self, now: Time, mut message: Message) {
        message.sequence = self.take_sequence();
        self.insert(message);
        self.sync(now);
    }

    pub fn delete(&mut self, handle: ReceiptHandle) -> Result<(), Error> {
        let arrival = self.in_flight_with(handle)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        now: Time,
        handle: ReceiptHandle,
        visibility_timeout: u64,
    ) -> Result<(), Error> {
        let arrival = self.in_flight_with(handle)?;
        self.set_state(arrival, in_flight_until(now, visibility_timeout));
        Ok(())
    }

    /// The message whose latest handle this is, provided it is in flight.
    fn in_flight_with(&self, handle: ReceiptHandle) -> Result<Arrival, Error> {
        self.by_handle
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|m| matches!(m.state, State::InFlight(_)))
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!(
                        "receipt handle {handle} is not the current handle of an in-flight message"
                    ),
                )
            })
    }

    fn take_sequence(&mut self) -> Option<u64> {
        if !self.config.fifo {
            return None;
        }
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        Some(sequence)
    }

    /// No earlier message of the same group is still in the queue. Messages
    /// of a standard queue have no group and always qualify.
    fn is_first_of_group(&self, arrival: Arrival) -> bool {
        match &self.messages.get(&arrival).expect(INDEXED).group {
            None => true,
            Some(group) => {
                self.groups.get(group).and_then(|members| members.first()) == Some(&arrival)
            }
        }
    }

    fn insert(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        if let Some(group) = &message.group {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(arrival);
        }
        self.index_state(arrival, message.state);
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: Arrival) -> Message {
        let message = self.messages.remove(&arrival).expect(INDEXED);
        self.unindex_state(arrival, message.state);
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(group) = &message.group {
            let members = self.groups.get_mut(group).expect(INDEXED);
            members.remove(&arrival);
            if members.is_empty() {
                self.groups.remove(group);
            }
        }
        if let Some(handle) = message.handle {
            self.by_handle.remove(&handle);
        }
        message
    }

    fn set_state(&mut self, arrival: Arrival, state: State) {
        let message = self.messages.get_mut(&arrival).expect(INDEXED);
        let previous = std::mem::replace(&mut message.state, state);
        self.unindex_state(arrival, previous);
        self.index_state(arrival, state);
    }

    fn index_state(&mut self, arrival: Arrival, state: State) {
        match state.hidden_until() {
            None => self.visible.insert(arrival),
            Some(until) => self.hidden.insert((until, arrival)),
        };
    }

    fn unindex_state(&mut self, arrival: Arrival, state: State) {
        match state.hidden_until() {
            None => self.visible.remove(&arrival),
            Some(until) => self.hidden.remove(&(until, arrival)),
        };
    }
}

/// A timeout of 0 leaves the message visible: `now < now + 0` is false.
fn in_flight_until(now: Time, visibility_timeout: u64) -> State {
    if visibility_timeout == 0 {
        State::Visible
    } else {
        State::InFlight(now + visibility_timeout)
    }
}

/// The id a FIFO send is deduplicated by.
fn deduplication_id(request: &SendMessage) -> &str {
    request
        .deduplication_id
        .as_ref()
        .map_or(request.body.as_str(), DeduplicationId::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(body: &str, group: Option<&str>, dedup: Option<&str>) -> SendMessage {
        SendMessage {
            queue: "q".into(),
            body: body.into(),
            delay_seconds: None,
            group_id: group.map(|g| GroupId::parse(g).unwrap()),
            deduplication_id: dedup.map(|d| DeduplicationId::parse(d).unwrap()),
        }
    }

    fn fifo() -> Queue {
        Queue::new(Config {
            fifo: true,
            ..Config::default()
        })
    }

    fn bodies(received: &Received) -> Vec<&str> {
        received.delivered.iter().map(|d| d.body.as_str()).collect()
    }

    #[test]
    fn redelivery_keeps_arrival_order() {
        let mut ids = IdSource::default();
        let mut queue = Queue::new(Config::default());
        for body in ["a", "b", "c"] {
            queue.send(0, &send(body, None, None), &mut ids).unwrap();
        }
        let first = queue.receive(0, 1, Some(10), &mut ids);
        assert_eq!(bodies(&first), ["a"]);

        // While "a" is in flight the others are served; once its timeout has
        // ended it is served again before them.
        queue.sync(5);
        assert_eq!(bodies(&queue.receive(5, 1, None, &mut ids)), ["b"]);
        queue.sync(10);
        assert_eq!(bodies(&queue.receive(10, 2, None, &mut ids)), ["a", "c"]);
    }

    #[test]
    fn zero_visibility_timeout_never_leaves_a_usable_handle() {
        let mut ids = IdSource::default();
        let mut queue = Queue::new(Config::default());
        queue.send(0, &send("a", None, None), &mut ids).unwrap();
        let received = queue.receive(0, 1, Some(0), &mut ids);
        let handle = received.delivered[0].receipt_handle;
        assert_eq!(
            queue.delete(handle).unwrap_err().code(),
            ErrorCode::ReceiptHandleExpired
        );
        assert_eq!(queue.counts().visible, 1);
    }

    #[test]
    fn a_fifo_group_is_blocked_by_its_earliest_message() {
        let mut ids = IdSource::default();
        let mut queue = fifo();
        for (body, group) in [("a1", "a"), ("a2", "a"), ("b1", "b")] {
            queue
                .send(0, &send(body, Some(group), Some(body)), &mut ids)
                .unwrap();
        }
        let received = queue.receive(0, 10, None, &mut ids);
        assert_eq!(bodies(&received), ["a1", "b1"]);
    }

    #[test]
    fn retention_is_applied_with_the_configuration_of_the_moment() {
        let mut ids = IdSource::default();
        let mut queue = Queue::new(Config::default());
        queue.send(0, &send("a", None, None), &mut ids).unwrap();
        let lowered = Config {
            retention_seconds: 60,
            ..Config::default()
        };
        queue.set_config(60, lowered);
        assert_eq!(queue.counts().visible, 0);
        queue.set_config(60, Config::default());
        assert_eq!(
            queue.counts().visible,
            0,
            "raising retention restores nothing"
        );
    }

    #[test]
    fn deduplication_window_is_half_open_and_not_extended() {
        let mut ids = IdSource::default();
        let mut queue = fifo();
        let request = send("a", Some("g"), Some("d"));
        let original = queue.send(0, &request, &mut ids).unwrap();
        assert_eq!(queue.send(299, &request, &mut ids).unwrap(), original);
        let renewed = queue.send(300, &request, &mut ids).unwrap();
        assert_ne!(renewed.id, original.id);
        assert_eq!(renewed.sequence, Some(2));
    }
}
