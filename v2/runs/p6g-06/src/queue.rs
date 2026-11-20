//! One queue: its attributes, its messages in arrival order and its
//! deduplication window.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::{AttributePatch, Attributes};
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};
use crate::reply::{Counts, ReceivedMessage};
use crate::request::SendRequest;

const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
pub(crate) struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    group_id: Option<String>,
    /// The message is hidden while `now < visible_at`: delayed if it was never
    /// delivered, in flight otherwise.
    visible_at: u64,
    /// The latest handle issued for this message.
    handle: Option<ReceiptHandle>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now >= self.visible_at {
            State::Visible
        } else if self.handle.is_some() {
            State::InFlight
        } else {
            State::Delayed
        }
    }
}

struct DedupEntry {
    accepted_at: u64,
    message_id: MessageId,
    sequence_number: u64,
}

pub(crate) struct Sent {
    pub message_id: MessageId,
    pub sequence_number: Option<u64>,
}

pub(crate) struct Delivery {
    pub sequence_number: u64,
    pub message: ReceivedMessage,
}

pub(crate) struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages removed from this queue for the dead-letter queue, in order.
    pub dead: Vec<Message>,
}

/// Identifies a queue even after another one reuses its name.
pub(crate) type QueueId = u64;

pub(crate) struct Queue {
    id: QueueId,
    attributes: Attributes,
    /// Keyed by sequence number: every message added to the queue takes the
    /// next one, so key order is arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, sequence number)` of every message, to find expired ones.
    by_sent_at: BTreeSet<(u64, u64)>,
    added: u64,
    deduplication: HashMap<String, DedupEntry>,
}

impl Queue {
    pub fn new(id: QueueId, attributes: Attributes) -> Self {
        Self {
            id,
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            added: 0,
            deduplication: HashMap::new(),
        }
    }

    pub fn id(&self) -> QueueId {
        self.id
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn apply_attributes(&mut self, patch: &AttributePatch, now: u64) {
        self.attributes.apply(patch);
        self.expire(now);
    }

    pub fn clear_dead_letter(&mut self) {
        self.attributes.dead_letter = None;
    }

    /// Removes every message whose retention has run out.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.attributes.retention_seconds);
        while let Some(&(sent_at, sequence_number)) = self.by_sent_at.first() {
            if now < sent_at.saturating_add(retention) {
                break;
            }
            self.remove(sequence_number);
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

    /// Removes all messages. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
    }

    fn remove(&mut self, sequence_number: u64) -> Option<Message> {
        let message = self.messages.remove(&sequence_number)?;
        self.by_sent_at.remove(&(message.sent_at, sequence_number));
        Some(message)
    }

    fn push(&mut self, message: Message) -> u64 {
        self.added += 1;
        self.by_sent_at.insert((message.sent_at, self.added));
        self.messages.insert(self.added, message);
        self.added
    }

    pub fn send(
        &mut self,
        request: &SendRequest,
        now: u64,
        message_ids: &mut u64,
    ) -> Result<Sent, Error> {
        let fifo = self.attributes.fifo;
        let unsupported = |what: &str| Error::new(ErrorCode::UnsupportedOperation, what);
        if fifo && request.delay_seconds.is_some() {
            return Err(unsupported("FIFO queues do not take a per-message delay"));
        }
        if !fifo && (request.group_id.is_some() || request.deduplication_id.is_some()) {
            return Err(unsupported(
                "standard queues do not take GroupId or DeduplicationId",
            ));
        }
        if fifo && request.group_id.is_none() {
            return Err(Error::missing("GroupId"));
        }
        let deduplication_key = if fifo {
            let explicit = request.deduplication_id.as_deref();
            if explicit.is_none() && !self.attributes.content_based_deduplication {
                return Err(Error::missing("DeduplicationId"));
            }
            Some(explicit.unwrap_or(&request.body))
        } else {
            None
        };
        if let Some(original) = deduplication_key.and_then(|key| self.deduplication.get(key))
            && now.saturating_sub(original.accepted_at) < DEDUPLICATION_WINDOW
        {
            return Ok(Sent {
                message_id: original.message_id,
                sequence_number: Some(original.sequence_number),
            });
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds 100000 messages",
            ));
        }
        let message_id = MessageId::issue(message_ids);
        let delay = request
            .delay_seconds
            .unwrap_or(self.attributes.delay_seconds);
        let sequence_number = self.push(Message {
            id: message_id,
            body: request.body.clone(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: request.group_id.clone(),
            visible_at: now + u64::from(delay),
            handle: None,
        });
        if let Some(key) = deduplication_key {
            self.deduplication.insert(
                key.to_owned(),
                DedupEntry {
                    accepted_at: now,
                    message_id,
                    sequence_number,
                },
            );
        }
        Ok(Sent {
            message_id,
            sequence_number: fifo.then_some(sequence_number),
        })
    }

    /// Delivers up to `max_messages` messages. `handles` holds the number of
    /// the last receipt handle issued by the process.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: Option<u32>,
        handles: &mut u64,
    ) -> Received {
        let timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let fifo = self.attributes.fifo;
        // Groups with an earlier message still in the queue. Only FIFO
        // queues have groups.
        let mut occupied_groups: HashSet<String> = HashSet::new();
        let mut deliveries = Vec::new();
        let mut dead = Vec::new();
        for (&sequence_number, message) in &mut self.messages {
            if deliveries.len() == max_messages {
                break;
            }
            if let Some(group) = message.group_id.as_deref() {
                if occupied_groups.contains(group) {
                    continue;
                }
                occupied_groups.insert(group.to_owned());
            }
            if message.state(now) != State::Visible {
                continue;
            }
            if max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                if let Some(group) = message.group_id.as_deref() {
                    occupied_groups.remove(group);
                }
                dead.push(sequence_number);
                continue;
            }
            let handle = ReceiptHandle::issue(handles);
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.receive_count += 1;
            message.handle = Some(handle);
            message.visible_at = now + u64::from(timeout);
            deliveries.push(Delivery {
                sequence_number,
                message: ReceivedMessage {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.group_id.clone(),
                    sequence_number: fifo.then_some(sequence_number),
                },
            });
        }
        let dead = dead
            .into_iter()
            .map(|sequence_number| {
                self.remove(sequence_number)
                    .expect("dead-lettered sequence numbers were just read from the queue")
            })
            .collect();
        Received { deliveries, dead }
    }

    /// Adds a message moved from another queue, as the spec's dead-letter
    /// move: counters reset, visible at once, no deduplication record.
    pub fn accept_dead(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.handle = None;
        message.visible_at = now;
        self.push(message);
        self.expire(now);
    }

    fn in_flight_mut(
        &mut self,
        sequence_number: u64,
        handle: ReceiptHandle,
        now: u64,
    ) -> Result<&mut Message, Error> {
        self.messages
            .get_mut(&sequence_number)
            .filter(|message| {
                message.handle == Some(handle) && message.state(now) == State::InFlight
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

    pub fn delete_in_flight(
        &mut self,
        sequence_number: u64,
        handle: ReceiptHandle,
        now: u64,
    ) -> Result<(), Error> {
        self.in_flight_mut(sequence_number, handle, now)?;
        self.remove(sequence_number);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        sequence_number: u64,
        handle: ReceiptHandle,
        timeout: u32,
        now: u64,
    ) -> Result<(), Error> {
        let message = self.in_flight_mut(sequence_number, handle, now)?;
        message.visible_at = now + u64::from(timeout);
        Ok(())
    }
}
