//! A single queue: its attributes and the messages it holds.

mod receive;

pub use receive::{DeadLettered, Delivery, Received};

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};

use serde::Serialize;

use crate::attributes::{AttributeUpdate, Attributes, DeadLetter, Kind};
use crate::error::{Error, ErrorCode, Result};
use crate::types::{
    Body, DeduplicationId, Delay, GroupId, MessageId, ReceiptHandle, SequenceNumber, Time,
};

const MAX_MESSAGES: usize = 100_000;
/// Seconds a deduplication id stays reserved after the send that opened its window.
const DEDUP_WINDOW: u64 = 300;

/// The position of a message in arrival order. Slots are never reused, so they also serve as
/// stable keys for the indexes below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Slot(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed { until: Time },
    InFlight { until: Time },
    Visible,
}

impl State {
    /// When a hidden message becomes visible again.
    fn until(self) -> Option<Time> {
        match self {
            Self::Delayed { until } | Self::InFlight { until } => Some(until),
            Self::Visible => None,
        }
    }
}

/// What a FIFO queue records about a message besides its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoPlacement {
    group_id: GroupId,
    sequence_number: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    placement: Option<FifoPlacement>,
    state: State,
    /// The handle of the latest delivery; older handles are expired.
    handle: Option<ReceiptHandle>,
}

/// The deduplication key of a FIFO send: an explicit id, or else the body itself. Both live in
/// one namespace, which is why the key is plain text.
#[derive(Debug, PartialEq, Eq, Hash)]
struct DedupKey(Box<str>);

impl From<&str> for DedupKey {
    fn from(text: &str) -> Self {
        Self(text.into())
    }
}

/// The send that opened the current deduplication window of a key.
#[derive(Debug)]
struct Accepted {
    at: Time,
    message_id: MessageId,
    sequence: SequenceNumber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub body: Body,
    pub delay: Option<Delay>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    pub sequence: Option<SequenceNumber>,
    /// A duplicate send reports the original message and stores nothing.
    pub duplicate: bool,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    visible: usize,
    in_flight: usize,
    delayed: usize,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Slot, Message>,
    next_slot: u64,
    next_sequence: SequenceNumber,
    /// When hidden messages come back. Entries go stale when a message changes state first, so
    /// the message's own `until` decides.
    wake: BinaryHeap<Reverse<(Time, Slot)>>,
    /// Messages by send time, to find the ones retention removes without scanning the queue.
    expiry: BTreeSet<(Time, Slot)>,
    /// Visible messages that a receive may deliver: in a FIFO queue, only the earliest message
    /// of each group can be among them.
    ready: BTreeSet<Slot>,
    /// FIFO only: the messages of each group in arrival order, in any state.
    groups: HashMap<GroupId, BTreeSet<Slot>>,
    /// The slot of the message each latest handle belongs to.
    handles: HashMap<ReceiptHandle, Slot>,
    /// Outlives the messages themselves, and is only ever used on FIFO queues.
    dedup: HashMap<DedupKey, Accepted>,
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_slot: 0,
            next_sequence: SequenceNumber::FIRST,
            wake: BinaryHeap::new(),
            expiry: BTreeSet::new(),
            ready: BTreeSet::new(),
            groups: HashMap::new(),
            handles: HashMap::new(),
            dedup: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn set_attributes(
        &mut self,
        update: &AttributeUpdate,
        dead_letter: Option<DeadLetter>,
        now: Time,
    ) {
        self.attributes.apply(update, dead_letter);
        self.expire(now);
    }

    /// Drops the dead-letter configuration if it points at the queue called `name`.
    pub fn forget_dead_letter(&mut self, name: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == name)
        {
            self.attributes.dead_letter = None;
        }
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state {
                State::Visible => counts.visible += 1,
                State::InFlight { .. } => counts.in_flight += 1,
                State::Delayed { .. } => counts.delayed += 1,
            }
        }
        counts
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.wake.clear();
        self.expiry.clear();
        self.ready.clear();
        self.groups.clear();
        self.handles.clear();
    }

    /// Brings the queue up to date after the clock moved to `now`.
    pub fn advance(&mut self, now: Time) {
        self.expire(now);
        while let Some(&Reverse((until, slot))) = self.wake.peek()
            && until <= now
        {
            self.wake.pop();
            let hidden_until_then = self
                .messages
                .get(&slot)
                .is_some_and(|message| message.state.until() == Some(until));
            if hidden_until_then {
                self.set_state(slot, State::Visible);
            }
        }
    }

    /// `message_id` is only used up when the send is accepted and not a duplicate.
    pub fn send(&mut self, now: Time, message_id: MessageId, new: NewMessage) -> Result<Sent> {
        let NewMessage {
            body,
            delay,
            group_id,
            deduplication_id,
        } = new;
        let fifo = match self.attributes.kind {
            Kind::Standard => {
                if group_id.is_some() || deduplication_id.is_some() {
                    return Err(unsupported(
                        "`GroupId` and `DeduplicationId` need a FIFO queue",
                    ));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported(
                        "`DelaySeconds` is not supported on a FIFO queue",
                    ));
                }
                let group = group_id.ok_or_else(|| missing("GroupId"))?;
                let key = match deduplication_id {
                    Some(id) => DedupKey::from(id.as_str()),
                    None if content_based_deduplication => DedupKey::from(body.as_str()),
                    None => return Err(missing("DeduplicationId")),
                };
                Some((group, key))
            }
        };
        if let Some((_, key)) = &fifo
            && let Some(prior) = self.dedup.get(key)
            && now < prior.at.after(DEDUP_WINDOW)
        {
            return Ok(Sent {
                message_id: prior.message_id,
                sequence: Some(prior.sequence),
                duplicate: true,
            });
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        let (placement, key): (Option<FifoPlacement>, Option<DedupKey>) = fifo
            .map(|(group_id, key)| {
                let sequence_number = self.next_sequence;
                self.next_sequence = sequence_number.next();
                (
                    FifoPlacement {
                        group_id,
                        sequence_number,
                    },
                    key,
                )
            })
            .unzip();
        let sequence = placement.as_ref().map(|p| p.sequence_number);
        let until = now.after(delay.unwrap_or(self.attributes.delay));
        let state = if until > now {
            State::Delayed { until }
        } else {
            State::Visible
        };
        self.push(Message {
            id: message_id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            placement,
            state,
            handle: None,
        });
        if let Some((key, sequence)) = key.zip(sequence) {
            self.dedup.insert(
                key,
                Accepted {
                    at: now,
                    message_id,
                    sequence,
                },
            );
        }
        Ok(Sent {
            message_id,
            sequence,
            duplicate: false,
        })
    }

    fn push(&mut self, message: Message) {
        let slot = Slot(self.next_slot);
        self.next_slot += 1;
        self.expiry.insert((message.sent_at, slot));
        if let Some(until) = message.state.until() {
            self.wake.push(Reverse((until, slot)));
        }
        if let Some(placement) = &message.placement {
            self.groups
                .entry(placement.group_id.clone())
                .or_default()
                .insert(slot);
        }
        self.messages.insert(slot, message);
        self.refresh_ready(slot);
    }

    fn remove(&mut self, slot: Slot) -> Message {
        let message = self
            .messages
            .remove(&slot)
            .expect("slots in the indexes always hold a message");
        self.expiry.remove(&(message.sent_at, slot));
        self.ready.remove(&slot);
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        if let Some(placement) = &message.placement
            && let Some(members) = self.groups.get_mut(&placement.group_id)
        {
            members.remove(&slot);
            match members.first().copied() {
                Some(head) => self.refresh_ready(head),
                None => {
                    self.groups.remove(&placement.group_id);
                }
            }
        }
        message
    }

    fn set_state(&mut self, slot: Slot, state: State) {
        if let Some(until) = state.until() {
            self.wake.push(Reverse((until, slot)));
        }
        if let Some(message) = self.messages.get_mut(&slot) {
            message.state = state;
        }
        self.refresh_ready(slot);
    }

    /// Puts the message in `ready` or takes it out, from its current state and group position.
    fn refresh_ready(&mut self, slot: Slot) {
        let Some(message) = self.messages.get(&slot) else {
            return;
        };
        let first_of_group = message.placement.as_ref().is_none_or(|placement| {
            self.groups
                .get(&placement.group_id)
                .and_then(BTreeSet::first)
                == Some(&slot)
        });
        if message.state == State::Visible && first_of_group {
            self.ready.insert(slot);
        } else {
            self.ready.remove(&slot);
        }
    }

    /// Removes every message whose retention has run out at `now`.
    fn expire(&mut self, now: Time) {
        while let Some(&(sent_at, slot)) = self.expiry.first()
            && now >= sent_at.after(self.attributes.retention)
        {
            self.remove(slot);
        }
    }
}
