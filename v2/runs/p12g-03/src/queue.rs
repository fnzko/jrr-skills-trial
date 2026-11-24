use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::Attributes;
use crate::handles::{HandleEntry, HandleLog};
use crate::response::{Counts, Received};
use crate::types::{GroupId, MessageId, QueueUid, ReceiptHandle, Seconds, SequenceNumber, Time};

/// How long a FIFO deduplication id keeps rejecting repeats.
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
const DEFAULT_RETENTION: Seconds = Seconds(345_600);

/// Whether a queue keeps strict per-group order and deduplicates sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Kind::Fifo { .. })
    }

    pub fn content_based_deduplication(self) -> bool {
        matches!(
            self,
            Kind::Fifo {
                content_based_deduplication: true
            }
        )
    }
}

/// Where a queue sends messages that were received too often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

impl Config {
    /// The caller has checked that `content_based_deduplication` is not set
    /// on a standard queue.
    pub fn new(attributes: &Attributes, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if attributes.fifo.unwrap_or(false) {
            Kind::Fifo {
                content_based_deduplication: attributes
                    .content_based_deduplication
                    .unwrap_or(false),
            }
        } else {
            Kind::Standard
        };
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: attributes.delay.unwrap_or(Seconds(0)),
            retention: attributes.retention.unwrap_or(DEFAULT_RETENTION),
            kind,
            dead_letter,
        }
    }

    /// Replaces the attributes that are present. The caller has checked the
    /// request against the queue's kind; `Fifo` is never changed.
    pub fn apply(&mut self, attributes: &Attributes, dead_letter: Option<DeadLetter>) {
        if let Some(timeout) = attributes.visibility_timeout {
            self.visibility_timeout = timeout;
        }
        if let Some(delay) = attributes.delay {
            self.delay = delay;
        }
        if let Some(retention) = attributes.retention {
            self.retention = retention;
        }
        if let (
            Kind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, attributes.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The place of a message in the order of one queue; unique per queue and
/// never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Why a message cannot be received right now, if it cannot.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Visible,
    Delayed { until: Time },
    InFlight { until: Time, handle: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

impl Hold {
    fn delayed(sent_at: Time, delay: Seconds) -> Self {
        if delay == Seconds(0) {
            Hold::Visible
        } else {
            Hold::Delayed {
                until: sent_at.plus(delay),
            }
        }
    }

    fn state(self, now: Time) -> State {
        match self {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

struct FifoPlace {
    group: GroupId,
    sequence: SequenceNumber,
}

struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    hold: Hold,
    fifo: Option<FifoPlace>,
}

impl Message {
    /// The message as it joins a dead-letter queue: same identity, sent time
    /// and group, but a fresh receive history and no delay.
    fn into_dead_letter(self) -> NewMessage<'static> {
        NewMessage {
            id: self.id,
            body: self.body,
            sent_at: self.sent_at,
            delay: Seconds(0),
            fifo: self.fifo.map(|place| FifoSend {
                group: place.group,
                deduplication_id: None,
            }),
        }
    }
}

/// A message about to join a queue.
pub struct NewMessage<'a> {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Time,
    pub delay: Seconds,
    pub fifo: Option<FifoSend<'a>>,
}

/// What a FIFO queue needs to know about an added message.
pub struct FifoSend<'a> {
    pub group: GroupId,
    /// The deduplication id to remember; `None` when the message is not a
    /// send and so does not take part in deduplication.
    pub deduplication_id: Option<&'a str>,
}

/// A deduplication id, a body or an explicit id.
#[derive(Debug, PartialEq, Eq, Hash)]
struct DeduplicationKey(String);

impl std::borrow::Borrow<str> for DeduplicationKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// The message that a deduplication id was last accepted for.
#[derive(Debug, Clone, Copy)]
pub struct Original {
    pub message: MessageId,
    pub sequence: SequenceNumber,
    accepted_at: Time,
}

pub struct Queue {
    uid: QueueUid,
    pub config: Config,
    messages: BTreeMap<Arrival, Message>,
    by_id: HashMap<MessageId, Arrival>,
    /// The same messages ordered by `sent_at`, so that retention only looks
    /// at the oldest ones.
    by_age: BTreeSet<(Time, Arrival)>,
    next_arrival: u64,
    sequences_issued: u64,
    deduplication: HashMap<DeduplicationKey, Original>,
}

impl Queue {
    pub fn new(uid: QueueUid, config: Config) -> Self {
        Self {
            uid,
            config,
            messages: BTreeMap::new(),
            by_id: HashMap::new(),
            by_age: BTreeSet::new(),
            next_arrival: 0,
            sequences_issued: 0,
            deduplication: HashMap::new(),
        }
    }

    /// Messages in every state.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes the messages whose retention has run out. Every command that
    /// reads or changes a queue calls this first, which is the same as
    /// removing them at the moment they expire.
    pub fn expire(&mut self, now: Time) {
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if sent_at.plus(self.config.retention) > now {
                break;
            }
            self.remove(arrival);
        }
    }

    /// Adds a message at the end of the queue and returns its sequence
    /// number if the queue is a FIFO queue.
    pub fn push(&mut self, new: NewMessage) -> Option<SequenceNumber> {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        let fifo = new.fifo.map(|send| {
            self.sequences_issued += 1;
            let sequence = SequenceNumber(self.sequences_issued);
            if let Some(id) = send.deduplication_id {
                let original = Original {
                    message: new.id,
                    sequence,
                    accepted_at: new.sent_at,
                };
                self.deduplication
                    .insert(DeduplicationKey(id.to_owned()), original);
            }
            FifoPlace {
                group: send.group,
                sequence,
            }
        });
        let sequence = fifo.as_ref().map(|place| place.sequence);
        self.by_id.insert(new.id, arrival);
        self.by_age.insert((new.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: new.id,
                body: new.body,
                sent_at: new.sent_at,
                receive_count: 0,
                first_received_at: None,
                hold: Hold::delayed(new.sent_at, new.delay),
                fifo,
            },
        );
        sequence
    }

    fn remove(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrivals passed to remove come from the queue's own indexes");
        self.by_id.remove(&message.id);
        self.by_age.remove(&(message.sent_at, arrival));
        message
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_id.clear();
        self.by_age.clear();
    }

    /// The original message if `id` was accepted less than 300 seconds ago.
    pub fn duplicate_of(&self, id: &str, now: Time) -> Option<Original> {
        self.deduplication
            .get(id)
            .filter(|original| original.accepted_at.plus(DEDUPLICATION_WINDOW) > now)
            .copied()
    }

    pub fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.hold.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Delivers up to `max` messages in arrival order. In a FIFO queue a
    /// message is skipped while an earlier message of its group is still
    /// in the queue, in any state. A visible message that was received
    /// `MaxReceiveCount` times is removed instead and handed back for the
    /// dead-letter queue; it does not count toward `max` and no longer
    /// blocks its group.
    pub fn receive(
        &mut self,
        now: Time,
        max: usize,
        visibility: Seconds,
        handles: &mut HandleLog,
    ) -> Delivery {
        let uid = self.uid;
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut delivered = Vec::new();
        let mut dead_arrivals = Vec::new();
        let mut blocked_groups = HashSet::new();
        for (&arrival, message) in self.messages.iter_mut() {
            if delivered.len() >= max {
                break;
            }
            if message
                .fifo
                .as_ref()
                .is_some_and(|place| blocked_groups.contains(&place.group))
            {
                continue;
            }
            if message.hold.state(now) == State::Visible {
                if max_receive_count.is_some_and(|max| message.receive_count >= max) {
                    dead_arrivals.push(arrival);
                    continue;
                }
                let handle = handles.issue(uid, message.id);
                message.receive_count += 1;
                let first_received_at = *message.first_received_at.get_or_insert(now);
                message.hold = Hold::InFlight {
                    until: now.plus(visibility),
                    handle,
                };
                delivered.push(Received {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.fifo.as_ref().map(|place| place.group.clone()),
                    sequence_number: message.fifo.as_ref().map(|place| place.sequence),
                });
            }
            if let Some(place) = &message.fifo {
                blocked_groups.insert(place.group.clone());
            }
        }
        let dead = dead_arrivals
            .into_iter()
            .map(|arrival| self.remove(arrival).into_dead_letter())
            .collect();
        Delivery {
            messages: delivered,
            dead,
        }
    }

    /// The message that `handle` is the latest handle of, if it is in flight
    /// in this queue right now.
    pub fn find_in_flight(
        &self,
        handle: ReceiptHandle,
        issued_for: HandleEntry,
        now: Time,
    ) -> Option<FlightTicket> {
        if issued_for.queue != self.uid {
            return None;
        }
        let arrival = *self.by_id.get(&issued_for.message)?;
        let message = self
            .messages
            .get(&arrival)
            .expect("by_id only holds messages of the queue");
        match message.hold {
            Hold::InFlight {
                until,
                handle: latest,
            } if latest == handle && now < until => Some(FlightTicket { arrival, handle }),
            _ => None,
        }
    }

    pub fn delete(&mut self, ticket: FlightTicket) {
        self.remove(ticket.arrival);
    }

    /// Makes the message in flight until `until`.
    pub fn set_in_flight_until(&mut self, ticket: FlightTicket, until: Time) {
        self.messages
            .get_mut(&ticket.arrival)
            .expect("a ticket names a message of the queue")
            .hold = Hold::InFlight {
            until,
            handle: ticket.handle,
        };
    }
}

/// The outcome of one `Queue::receive`.
pub struct Delivery {
    pub messages: Vec<Received>,
    pub dead: Vec<NewMessage<'static>>,
}

/// Proof that a message was found in flight, for `Queue::delete` and
/// `Queue::set_in_flight_until`.
#[derive(Debug, Clone, Copy)]
pub struct FlightTicket {
    arrival: Arrival,
    handle: ReceiptHandle,
}
