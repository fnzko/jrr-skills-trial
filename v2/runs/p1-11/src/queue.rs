//! One queue: its attributes, its messages in arrival order, and the
//! deduplication record. Time is passed in as `now`; the queue never reads a clock.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::Attributes;
use crate::command::SendRequest;
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{Ids, MessageId, ReceiptHandle};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug)]
struct InFlight {
    until: u64,
    handle: ReceiptHandle,
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    delayed_until: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    group_id: Option<String>,
    sequence: Option<u64>,
    /// The latest delivery. Kept after its timeout ends so the old handle can
    /// be recognised until the message is delivered again or removed.
    in_flight: Option<InFlight>,
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.in_flight.as_ref().is_some_and(|f| now < f.until) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What a client sees of a delivered message.
#[derive(Debug)]
pub struct Delivery {
    pub id: MessageId,
    pub handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<String>,
    pub sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The identity of an accepted (or deduplicated) send.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub id: MessageId,
    /// Present on FIFO queues only.
    pub sequence: Option<u64>,
}

#[derive(Debug)]
struct SeenDeduplicationId {
    at: u64,
    accepted: Accepted,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    /// Keyed by arrival number, so iteration is arrival order.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    next_sequence: u64,
    /// `(sent_at, arrival)` of every message, to find expired ones without a scan.
    by_age: BTreeSet<(u64, u64)>,
    /// Latest handle of each delivered message, by arrival number.
    by_handle: HashMap<ReceiptHandle, u64>,
    /// Outlives the messages it was recorded for.
    deduplication: HashMap<String, SeenDeduplicationId>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: 0,
            next_sequence: 1,
            by_age: BTreeSet::new(),
            by_handle: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    /// Removes the messages whose retention has run out. Retention can only be
    /// observed through the queue, so callers apply it before every access.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.attributes.retention_seconds);
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(arrival);
        }
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

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.by_handle.clear();
    }

    pub fn send(&mut self, now: u64, request: &SendRequest, ids: &mut Ids) -> Result<Accepted> {
        let fifo = self.attributes.fifo;
        let unsupported = if fifo {
            request.delay_seconds.map(|_| "DelaySeconds")
        } else if request.group_id.is_some() {
            Some("GroupId")
        } else {
            request.deduplication_id.as_ref().map(|_| "DeduplicationId")
        };
        if let Some(name) = unsupported {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                format!("{name} is not supported by this type of queue"),
            ));
        }

        let deduplication_id = if fifo {
            if request.group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            let id = match &request.deduplication_id {
                Some(id) => id,
                None if self.attributes.content_based_deduplication => &request.body,
                None => return Err(Error::missing("DeduplicationId")),
            };
            if let Some(seen) = self.deduplication.get(id)
                && now < seen.at + DEDUPLICATION_WINDOW
            {
                return Ok(seen.accepted);
            }
            Some(id)
        } else {
            None
        };

        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let delay = request
            .delay_seconds
            .unwrap_or(self.attributes.delay_seconds);
        let accepted = Accepted {
            id: ids.next_message(),
            sequence: self.take_sequence(),
        };
        if let Some(id) = deduplication_id {
            self.deduplication
                .insert(id.clone(), SeenDeduplicationId { at: now, accepted });
        }
        self.insert(Message {
            id: accepted.id,
            body: request.body.clone(),
            sent_at: now,
            delayed_until: now + u64::from(delay),
            receive_count: 0,
            first_received_at: None,
            group_id: request.group_id.clone(),
            sequence: accepted.sequence,
            in_flight: None,
        });
        Ok(accepted)
    }

    /// Delivers up to `max_messages` messages. Messages that have reached the
    /// dead-letter limit are taken out of the queue and returned instead, for
    /// the caller to add to the dead-letter queue.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: u32,
        ids: &mut Ids,
    ) -> (Vec<Delivery>, Vec<DeadLetterMove>) {
        let fifo = self.attributes.fifo;
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut deliveries = Vec::new();
        let mut dead = Vec::new();
        // FIFO groups that still have an earlier message in the queue.
        let mut occupied_groups: HashSet<&str> = HashSet::new();

        for (&arrival, message) in self.messages.iter_mut() {
            if deliveries.len() == max_messages {
                break;
            }
            let group = message.group_id.as_deref();
            let blocked = fifo && group.is_some_and(|g| occupied_groups.contains(g));
            let eligible = !blocked && message.state(now) == State::Visible;

            if eligible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                dead.push(arrival);
                continue;
            }
            if eligible {
                message.receive_count += 1;
                let first_received_at = *message.first_received_at.get_or_insert(now);
                let handle = ids.next_receipt();
                let previous = message.in_flight.replace(InFlight {
                    until: now + u64::from(visibility_timeout),
                    handle,
                });
                if let Some(previous) = previous {
                    self.by_handle.remove(&previous.handle);
                }
                self.by_handle.insert(handle, arrival);
                deliveries.push(Delivery {
                    id: message.id,
                    handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.group_id.clone(),
                    sequence: message.sequence,
                });
            }
            if let Some(group) = message.group_id.as_deref() {
                occupied_groups.insert(group);
            }
        }

        let moves = dead
            .into_iter()
            .filter_map(|arrival| self.remove(arrival))
            .map(DeadLetterMove)
            .collect();
        (deliveries, moves)
    }

    /// Adds a message taken from another queue by its dead-letter limit.
    pub fn accept_dead_letter(&mut self, DeadLetterMove(mut message): DeadLetterMove) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.in_flight = None;
        message.delayed_until = 0;
        message.sequence = self.take_sequence();
        self.insert(message);
    }

    pub fn delete_message(&mut self, handle: ReceiptHandle, now: u64) -> Result<()> {
        let arrival = self.in_flight_arrival(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        visibility_timeout: u32,
        now: u64,
    ) -> Result<()> {
        let arrival = self.in_flight_arrival(handle, now)?;
        if let Some(in_flight) = self
            .messages
            .get_mut(&arrival)
            .and_then(|m| m.in_flight.as_mut())
        {
            in_flight.until = now + u64::from(visibility_timeout);
        }
        Ok(())
    }

    /// The message that `handle` is the latest handle of, if it is in flight now.
    fn in_flight_arrival(&self, handle: ReceiptHandle, now: u64) -> Result<u64> {
        self.by_handle
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|m| m.state(now) == State::InFlight)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is no longer usable"),
                )
            })
    }

    fn take_sequence(&mut self) -> Option<u64> {
        self.attributes.fifo.then(|| {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            sequence
        })
    }

    fn insert(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: u64) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(in_flight) = &message.in_flight {
            self.by_handle.remove(&in_flight.handle);
        }
        Some(message)
    }
}

/// A message on its way to a dead-letter queue.
#[derive(Debug)]
pub struct DeadLetterMove(Message);
