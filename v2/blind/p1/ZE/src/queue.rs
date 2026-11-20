//! One queue: its attributes, its messages and the rules that act on them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::attributes::{AttributeUpdate, DeadLetterConfig, QueueConfig};
use crate::error::{Error, ErrorCode};
use crate::receipts::{Issued, Receipt, ReceiptLog};

/// Most messages one queue holds, in any state.
const MAX_MESSAGES: usize = 100_000;
/// Seconds during which a deduplication id rejects repeats.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Identifies a queue for the life of the process; a recreated queue gets a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone, Copy)]
struct InFlight {
    receipt: Receipt,
    until: u64,
}

/// A message as a queue stores it. Other modules only move it between queues.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    /// Present exactly in FIFO queues.
    group_id: Option<String>,
    delayed_until: u64,
    /// The latest delivery, which may have timed out.
    in_flight: Option<InFlight>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.in_flight.is_some_and(|flight| now < flight.until) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// A send request whose parameters were checked on their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRequest {
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// What `Queue::send` did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    pub sequence: u64,
    /// False when the send repeated a recent deduplication id.
    pub accepted: bool,
}

/// A message handed out by `Queue::receive`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    pub sequence: u64,
    pub receipt: Receipt,
    pub message_id: MessageId,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<String>,
}

/// What one `Queue::receive` call did.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivered>,
    /// Messages that reached `MaxReceiveCount`: gone from this queue, to be
    /// handed to the dead-letter queue with `Queue::accept_moved`.
    pub dead: Vec<Message>,
}

#[derive(Debug)]
struct Accepted {
    at: u64,
    message_id: MessageId,
    sequence: u64,
}

#[derive(Debug)]
pub struct Queue {
    id: QueueId,
    config: QueueConfig,
    /// Keyed by sequence number, the count of messages this queue had added
    /// when the message arrived, so key order is arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, sequence)` of every message, so retention only looks at the oldest.
    by_sent: BTreeSet<(u64, u64)>,
    added: u64,
    /// FIFO queues only. Outlives the messages it was recorded for.
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(id: QueueId, config: QueueConfig) -> Self {
        Self {
            id,
            config,
            messages: BTreeMap::new(),
            by_sent: BTreeSet::new(),
            added: 0,
            deduplication: HashMap::new(),
        }
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    /// Removes the messages whose retention has run out. Every command that
    /// looks at the messages calls this first, which is equivalent to removing
    /// them the moment they expire.
    pub fn expire(&mut self, now: u64) {
        while let Some(&(sent_at, sequence)) = self.by_sent.first() {
            if now < sent_at + self.config.retention_seconds {
                break;
            }
            self.by_sent.pop_first();
            self.messages.remove(&sequence);
        }
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
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
        self.by_sent.clear();
    }

    /// Applies new attributes. Messages that a lower retention would already
    /// have removed under the old value are removed before it changes.
    pub fn set_attributes(
        &mut self,
        now: u64,
        update: AttributeUpdate,
        dead_letter: Option<DeadLetterConfig>,
    ) {
        self.expire(now);
        update.apply(&mut self.config, dead_letter);
    }

    /// Forgets the dead-letter configuration if it names `queue`.
    pub fn drop_dead_letter(&mut self, queue: &str) {
        if self
            .config
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue == queue)
        {
            self.config.dead_letter = None;
        }
    }

    fn push(&mut self, message: Message) -> u64 {
        self.added += 1;
        self.by_sent.insert((message.sent_at, self.added));
        self.messages.insert(self.added, message);
        self.added
    }

    /// Applies the send rules of the spec. `id` is the identifier the message
    /// gets if it is accepted.
    pub fn send(&mut self, now: u64, id: MessageId, request: SendRequest) -> Result<Sent, Error> {
        let SendRequest {
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        } = request;
        let deduplication_id = if self.config.fifo {
            if delay_seconds.is_some() {
                return Err(Error::unsupported(
                    "FIFO queues do not support per-message delay",
                ));
            }
            if group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            match deduplication_id {
                Some(deduplication_id) => Some(deduplication_id),
                None if self.config.content_based_deduplication => Some(body.clone()),
                None => return Err(Error::missing("DeduplicationId")),
            }
        } else {
            if group_id.is_some() || deduplication_id.is_some() {
                return Err(Error::unsupported(
                    "standard queues do not support GroupId or DeduplicationId",
                ));
            }
            None
        };

        if let Some(accepted) = deduplication_id
            .as_ref()
            .and_then(|key| self.deduplication.get(key))
            && now - accepted.at < DEDUPLICATION_WINDOW
        {
            return Ok(Sent {
                message_id: accepted.message_id,
                sequence: accepted.sequence,
                accepted: false,
            });
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let delay = delay_seconds.unwrap_or(self.config.delay_seconds);
        let sequence = self.push(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id,
            delayed_until: now + delay,
            in_flight: None,
        });
        if let Some(key) = deduplication_id {
            self.deduplication.insert(
                key,
                Accepted {
                    at: now,
                    message_id: id,
                    sequence,
                },
            );
        }
        Ok(Sent {
            message_id: id,
            sequence,
            accepted: true,
        })
    }

    /// Examines the messages in arrival order and delivers up to `max` of the
    /// eligible ones. Eligible messages that have reached the dead-letter
    /// limit are taken out instead and do not count toward `max`.
    pub fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility_timeout: u64,
        receipts: &mut ReceiptLog,
    ) -> Received {
        enum Step {
            Deliver(u64),
            MoveToDeadLetter(u64),
        }

        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut steps = Vec::new();
        let mut delivering = 0;
        // A message blocks the later ones of its group for as long as it
        // stays in this queue, whatever its state.
        let mut seen_groups: HashSet<&str> = HashSet::new();
        for (&sequence, message) in &self.messages {
            let group = message.group_id.as_deref();
            let blocked = group.is_some_and(|group| seen_groups.contains(group));
            if blocked || message.state(now) != State::Visible {
                seen_groups.extend(group);
            } else if max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                steps.push(Step::MoveToDeadLetter(sequence));
            } else {
                seen_groups.extend(group);
                steps.push(Step::Deliver(sequence));
                delivering += 1;
                if delivering == max {
                    break;
                }
            }
        }

        let mut received = Received {
            delivered: Vec::new(),
            dead: Vec::new(),
        };
        for step in steps {
            match step {
                Step::Deliver(sequence) => {
                    let message = self
                        .messages
                        .get_mut(&sequence)
                        .expect("steps were read from this map");
                    let receipt = receipts.issue(self.id, sequence);
                    message.receive_count += 1;
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    message.in_flight = Some(InFlight {
                        receipt,
                        until: now + visibility_timeout,
                    });
                    received.delivered.push(Delivered {
                        sequence,
                        receipt,
                        message_id: message.id,
                        body: message.body.clone(),
                        receive_count: message.receive_count,
                        sent_at: message.sent_at,
                        first_received_at,
                        group_id: message.group_id.clone(),
                    });
                }
                Step::MoveToDeadLetter(sequence) => {
                    let message = self
                        .remove(sequence)
                        .expect("steps were read from this map");
                    received.dead.push(message);
                }
            }
        }
        received
    }

    /// Adds a message that another queue gave up on, as a fresh message that
    /// is visible at once. Retention still counts from the original send, so
    /// a message that is already too old is dropped on arrival; it still
    /// takes its sequence number. Deduplication and the size limit do not apply.
    pub fn accept_moved(&mut self, now: u64, mut message: Message) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delayed_until = 0;
        message.in_flight = None;
        if now < message.sent_at + self.config.retention_seconds {
            self.push(message);
        } else {
            self.added += 1;
        }
    }

    fn remove(&mut self, sequence: u64) -> Option<Message> {
        let message = self.messages.remove(&sequence)?;
        self.by_sent.remove(&(message.sent_at, sequence));
        Some(message)
    }

    /// The message that `receipt` was issued for, if it is still in flight
    /// under that receipt. A receipt of another queue never matches.
    fn in_flight_mut(
        &mut self,
        now: u64,
        receipt: Receipt,
        issued: Issued,
    ) -> Result<&mut Message, Error> {
        let expired = || {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("{receipt} no longer refers to a message in flight"),
            )
        };
        if issued.queue != self.id {
            return Err(expired());
        }
        self.messages
            .get_mut(&issued.sequence)
            .filter(|message| {
                message
                    .in_flight
                    .is_some_and(|flight| flight.receipt == receipt && now < flight.until)
            })
            .ok_or_else(expired)
    }

    pub fn delete(&mut self, now: u64, receipt: Receipt, issued: Issued) -> Result<(), Error> {
        self.in_flight_mut(now, receipt, issued)?;
        self.remove(issued.sequence);
        Ok(())
    }

    /// Makes the message in flight until `now + timeout`.
    pub fn change_visibility(
        &mut self,
        now: u64,
        receipt: Receipt,
        issued: Issued,
        timeout: u64,
    ) -> Result<(), Error> {
        let message = self.in_flight_mut(now, receipt, issued)?;
        message.in_flight = Some(InFlight {
            receipt,
            until: now + timeout,
        });
        Ok(())
    }
}
