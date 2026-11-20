//! A single queue: its configuration and its messages, with the rules for
//! sending, receiving and expiring them (SPEC.md sections 4 and 6).
//!
//! Messages live in one map in arrival order. Four indexes sit beside it so
//! that no command has to scan the whole queue: one for retention, one for
//! messages that are not visible yet, one for what a receive can deliver now,
//! and one per FIFO group. Every change to a message updates all the indexes
//! it belongs to.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::attributes::{AttributeChanges, Attributes, DeadLetter};
use crate::command::SendRequest;
use crate::{Error, ErrorCode};

/// The most messages one queue holds, counting every state.
pub const MAX_MESSAGES: usize = 100_000;

/// Seconds after an accepted FIFO send during which the same deduplication id
/// counts as a duplicate.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Message counts by state at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The state of a message at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
pub struct Message {
    id: u64,
    body: String,
    group_id: Option<String>,
    sequence_number: Option<u64>,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    /// The message is visible from this clock value on. A message that has
    /// never been received is delayed until then; one that has is in flight.
    visible_at: u64,
    /// The receipt of the latest delivery. Only this handle can still act on
    /// the message.
    latest_receipt: Option<u64>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now >= self.visible_at {
            State::Visible
        } else if self.receive_count == 0 {
            State::Delayed
        } else {
            State::InFlight
        }
    }
}

/// The record a FIFO send leaves, used to recognise duplicates.
#[derive(Debug)]
struct Accepted {
    at: u64,
    message_id: u64,
    sequence_number: u64,
}

/// A message as handed to a client by ReceiveMessage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub message_id: u64,
    pub receipt: u64,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
}

/// What one receive did to a queue.
#[derive(Debug, Default)]
pub struct ReceiveOutcome {
    pub deliveries: Vec<Delivery>,
    /// Messages moved out of this queue, in the order they were moved.
    pub dead_letters: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    /// Keyed by arrival order: the order in which messages joined this queue.
    /// Keys are never reused, so map order is arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, arrival)` for every message, so retention can stop at the
    /// first message that has not yet expired.
    by_sent: BTreeSet<(u64, u64)>,
    /// `(visible_at, arrival)` for messages that are not visible yet.
    waking: BTreeSet<(u64, u64)>,
    /// Arrivals a receive may deliver now: visible messages, and on a FIFO
    /// queue only the first message of each group.
    ready: BTreeSet<u64>,
    /// FIFO only: the arrivals of each group's messages still in the queue.
    groups: HashMap<String, BTreeSet<u64>>,
    next_arrival: u64,
    next_sequence: u64,
    deduplication: HashMap<String, Accepted>,
    /// Receipt number to the arrival of the message it was issued for. Entries
    /// are never removed; a stale one fails the in-flight check.
    receipts: HashMap<u64, u64>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Queue {
            attributes,
            messages: BTreeMap::new(),
            by_sent: BTreeSet::new(),
            waking: BTreeSet::new(),
            ready: BTreeSet::new(),
            groups: HashMap::new(),
            next_arrival: 0,
            next_sequence: 1,
            deduplication: HashMap::new(),
            receipts: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn reconfigure(&mut self, changes: &AttributeChanges, dead_letter: Option<DeadLetter>) {
        self.attributes.apply(changes, dead_letter);
    }

    /// Clears the dead-letter configuration if it points at `queue`, which is
    /// being deleted.
    pub fn detach_dead_letter(&mut self, queue: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue == queue)
        {
            self.attributes.dead_letter = None;
        }
    }

    /// Brings the queue up to `now`: messages past their retention are removed,
    /// and messages whose delay or visibility timeout has ended become visible.
    /// Every command that reads the queue calls this first.
    pub fn advance(&mut self, now: u64) {
        let retention = u64::from(self.attributes.retention_seconds);
        if let Some(threshold) = now.checked_sub(retention) {
            // Expired means `now >= sent_at + retention`, that is `sent_at <= threshold`.
            while let Some(&(sent_at, key)) = self.by_sent.first() {
                if sent_at > threshold {
                    break;
                }
                self.remove(now, key);
            }
        }
        while let Some(&(visible_at, key)) = self.waking.first() {
            if visible_at > now {
                break;
            }
            self.waking.pop_first();
            self.make_ready_if_head(key);
        }
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

    pub fn purge(&mut self) {
        // Sequence numbers and deduplication records outlive the messages.
        self.messages.clear();
        self.by_sent.clear();
        self.waking.clear();
        self.ready.clear();
        self.groups.clear();
        self.receipts.clear();
    }

    /// The rules of SendMessage that depend on the queue's type (section 6,
    /// rules 1 to 3).
    pub fn check_send(&self, request: &SendRequest) -> Result<(), Error> {
        if !self.attributes.fifo {
            if request.group_id.is_some() || request.deduplication_id.is_some() {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "GroupId and DeduplicationId are only allowed on a FIFO queue",
                ));
            }
            return Ok(());
        }
        if request.delay_seconds.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "DelaySeconds is not allowed on a FIFO queue",
            ));
        }
        if request.group_id.is_none() {
            return Err(Error::new(
                ErrorCode::MissingParameter,
                "GroupId is required on a FIFO queue",
            ));
        }
        if request.deduplication_id.is_none() && !self.attributes.content_based_deduplication {
            return Err(Error::new(
                ErrorCode::MissingParameter,
                "DeduplicationId is required when ContentBasedDeduplication is false",
            ));
        }
        Ok(())
    }

    /// The earlier FIFO send that makes `request` a duplicate, as the
    /// `(message id, sequence number)` it was accepted with.
    pub fn duplicate_of(&self, now: u64, request: &SendRequest) -> Option<(u64, u64)> {
        if !self.attributes.fifo {
            return None;
        }
        self.deduplication
            .get(deduplication_id(request))
            .filter(|accepted| now < accepted.at.saturating_add(DEDUPLICATION_WINDOW))
            .map(|accepted| (accepted.message_id, accepted.sequence_number))
    }

    /// Rule 5 of SendMessage: the queue must have room for one more message.
    pub fn check_capacity(&self) -> Result<(), Error> {
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        Ok(())
    }

    /// Adds an accepted message at the end of the queue. Returns its sequence
    /// number on a FIFO queue.
    pub fn push(&mut self, now: u64, message_id: u64, request: SendRequest) -> Option<u64> {
        let delay = request
            .delay_seconds
            .unwrap_or(self.attributes.delay_seconds);
        let sequence_number = if self.attributes.fifo {
            let number = self.take_sequence();
            self.deduplication.insert(
                deduplication_id(&request).to_owned(),
                Accepted {
                    at: now,
                    message_id,
                    sequence_number: number,
                },
            );
            Some(number)
        } else {
            None
        };
        self.insert(
            now,
            Message {
                id: message_id,
                body: request.body,
                group_id: request.group_id,
                sequence_number,
                sent_at: now,
                receive_count: 0,
                first_received_at: None,
                visible_at: now.saturating_add(u64::from(delay)),
                latest_receipt: None,
            },
        );
        sequence_number
    }

    /// Adds a message moved from a source queue to the end of this queue, which
    /// is its dead-letter queue. It keeps its id, body, sent time and group, and
    /// starts again: no receives, visible at once, no receipt. A FIFO queue gives
    /// it the next sequence number, and a message already past this queue's
    /// retention is dropped on arrival.
    pub fn accept_dead_letter(&mut self, now: u64, mut message: Message) {
        let sequence_number = self.attributes.fifo.then(|| self.take_sequence());
        if expired(now, message.sent_at, self.attributes.retention_seconds) {
            return;
        }
        message.receive_count = 0;
        message.first_received_at = None;
        message.visible_at = now;
        message.latest_receipt = None;
        message.sequence_number = sequence_number;
        self.insert(now, message);
    }

    /// Runs ReceiveMessage (section 6): delivers up to `max` ready messages in
    /// arrival order, and moves messages that have used up their receives out
    /// of this queue. Each delivery takes the next receipt number from
    /// `next_receipt`.
    ///
    /// Moved messages do not count toward `max`. Moving one can make the next
    /// message of its FIFO group the group's first, so the scan keeps its place
    /// by arrival and picks that message up if it comes later.
    pub fn receive(
        &mut self,
        now: u64,
        max: u32,
        visibility_timeout: u32,
        next_receipt: &mut u64,
    ) -> ReceiveOutcome {
        let mut outcome = ReceiveOutcome::default();
        let mut delivered = 0;
        let mut cursor = 0;
        while delivered < max {
            let Some(key) = self.ready.range(cursor..).next().copied() else {
                break;
            };
            cursor = key + 1;
            if self.exhausted(key) {
                let message = self
                    .remove(now, key)
                    .expect("ready arrivals name stored messages");
                outcome.dead_letters.push(message);
            } else {
                outcome
                    .deliveries
                    .push(self.deliver(now, key, visibility_timeout, next_receipt));
                delivered += 1;
            }
        }
        outcome
    }

    /// Deletes the message that `receipt` is the latest handle of. The handle
    /// must still be in flight at `now`.
    pub fn delete_message(&mut self, now: u64, receipt: u64) -> Result<(), Error> {
        let key = self
            .in_flight_key(now, receipt)
            .ok_or_else(receipt_expired)?;
        self.remove(now, key);
        Ok(())
    }

    /// Ends the visibility timeout of the message `receipt` is the latest
    /// handle of, `visibility_timeout` seconds from `now`.
    pub fn change_visibility(
        &mut self,
        now: u64,
        receipt: u64,
        visibility_timeout: u32,
    ) -> Result<(), Error> {
        let key = self
            .in_flight_key(now, receipt)
            .ok_or_else(receipt_expired)?;
        let visible_at = now.saturating_add(u64::from(visibility_timeout));
        let previous = {
            let message = self
                .messages
                .get_mut(&key)
                .expect("in-flight keys name stored messages");
            std::mem::replace(&mut message.visible_at, visible_at)
        };
        self.waking.remove(&(previous, key));
        self.wake_or_wait(key, visible_at, now);
        Ok(())
    }

    /// Stores a message at the end of the queue and indexes it.
    fn insert(&mut self, now: u64, message: Message) {
        let key = self.next_arrival;
        self.next_arrival += 1;
        self.by_sent.insert((message.sent_at, key));
        if let Some(group) = &message.group_id {
            self.groups.entry(group.clone()).or_default().insert(key);
        }
        let visible_at = message.visible_at;
        self.messages.insert(key, message);
        self.wake_or_wait(key, visible_at, now);
    }

    /// Makes `key` ready if it is visible now, and otherwise schedules its wake-up.
    fn wake_or_wait(&mut self, key: u64, visible_at: u64, now: u64) {
        if visible_at <= now {
            self.make_ready_if_head(key);
        } else {
            self.waking.insert((visible_at, key));
        }
    }

    /// Makes a visible message ready, unless an earlier message of its FIFO
    /// group is still in the queue: then only that earlier message is ready.
    fn make_ready_if_head(&mut self, key: u64) {
        let message = self
            .messages
            .get(&key)
            .expect("indexed arrivals name stored messages");
        let is_head = match &message.group_id {
            None => true,
            Some(group) => self.groups.get(group).and_then(BTreeSet::first) == Some(&key),
        };
        if is_head {
            self.ready.insert(key);
        }
    }

    /// Removes a message from the queue and from every index. When it was the
    /// head of a FIFO group, the group's next message becomes the head.
    fn remove(&mut self, now: u64, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        self.by_sent.remove(&(message.sent_at, key));
        self.waking.remove(&(message.visible_at, key));
        self.ready.remove(&key);
        if let Some(group) = &message.group_id {
            let (next_head, now_empty) = match self.groups.get_mut(group) {
                Some(members) => {
                    members.remove(&key);
                    (members.first().copied(), members.is_empty())
                }
                None => (None, true),
            };
            if now_empty {
                self.groups.remove(group);
            }
            if let Some(head) = next_head {
                let visible = self
                    .messages
                    .get(&head)
                    .is_some_and(|next| now >= next.visible_at);
                if visible {
                    self.make_ready_if_head(head);
                }
            }
        }
        Some(message)
    }

    /// Whether a receive would move this message to the dead-letter queue.
    fn exhausted(&self, key: u64) -> bool {
        let message = self
            .messages
            .get(&key)
            .expect("ready arrivals name stored messages");
        self.attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| message.receive_count >= dead_letter.max_receive_count)
    }

    /// Delivers the ready message at `key`, which stays in the queue in flight.
    fn deliver(
        &mut self,
        now: u64,
        key: u64,
        visibility_timeout: u32,
        next_receipt: &mut u64,
    ) -> Delivery {
        let receipt = *next_receipt;
        *next_receipt += 1;
        self.ready.remove(&key);
        let visible_at = now.saturating_add(u64::from(visibility_timeout));
        let message = self
            .messages
            .get_mut(&key)
            .expect("ready arrivals name stored messages");
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.visible_at = visible_at;
        message.latest_receipt = Some(receipt);
        let delivery = Delivery {
            message_id: message.id,
            receipt,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group_id.clone(),
            sequence_number: message.sequence_number,
        };
        self.receipts.insert(receipt, key);
        self.wake_or_wait(key, visible_at, now);
        delivery
    }

    /// The arrival of the message `receipt` is the latest handle of, if that
    /// message is in flight at `now`.
    fn in_flight_key(&self, now: u64, receipt: u64) -> Option<u64> {
        let key = *self.receipts.get(&receipt)?;
        let message = self.messages.get(&key)?;
        (message.latest_receipt == Some(receipt) && message.state(now) == State::InFlight)
            .then_some(key)
    }

    fn take_sequence(&mut self) -> u64 {
        let number = self.next_sequence;
        self.next_sequence += 1;
        number
    }
}

/// The deduplication id of a FIFO send: its own id, or the body when the
/// queue derives ids from content.
fn deduplication_id(request: &SendRequest) -> &str {
    request.deduplication_id.as_deref().unwrap_or(&request.body)
}

/// Whether a message sent at `sent_at` is past its retention by `now`.
fn expired(now: u64, sent_at: u64, retention_seconds: u32) -> bool {
    now >= sent_at.saturating_add(u64::from(retention_seconds))
}

fn receipt_expired() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle is no longer the latest one of an in-flight message",
    )
}
