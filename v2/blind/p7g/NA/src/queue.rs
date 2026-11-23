//! A single queue: its attributes and its messages.
//!
//! Time is passed in explicitly (`now`) by the broker; a queue never holds a clock.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::command::Attributes;
use crate::error::{Error, ErrorCode, Result};

pub const MAX_MESSAGES: usize = 100_000;
/// A FIFO send repeating a deduplication id is dropped for this many seconds.
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Config {
    /// Builds a configuration from a request, filling in defaults.
    ///
    /// The caller has already checked that `DeadLetterQueue` and
    /// `MaxReceiveCount` come as a pair.
    pub fn from_attributes(attributes: &Attributes) -> Self {
        Self {
            visibility_timeout: attributes.visibility_timeout.unwrap_or(30),
            delay_seconds: attributes.delay_seconds.unwrap_or(0),
            retention_seconds: attributes.retention_seconds.unwrap_or(345_600),
            fifo: attributes.fifo.unwrap_or(false),
            content_based_deduplication: attributes.content_based_deduplication.unwrap_or(false),
            dead_letter: dead_letter_of(attributes),
        }
    }

    /// Overwrites the attributes that are present in `attributes`.
    pub fn apply(&mut self, attributes: &Attributes) {
        if let Some(v) = attributes.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = attributes.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = attributes.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = attributes.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if let Some(dead_letter) = dead_letter_of(attributes) {
            self.dead_letter = Some(dead_letter);
        }
    }
}

fn dead_letter_of(attributes: &Attributes) -> Option<DeadLetter> {
    match (&attributes.dead_letter_queue, attributes.max_receive_count) {
        (Some(queue), Some(max_receive_count)) => Some(DeadLetter {
            queue: queue.clone(),
            max_receive_count,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    id: u64,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    group_id: Option<String>,
    sequence_number: Option<u64>,
    /// Delayed while `now < delayed_until`.
    delayed_until: u64,
    /// In flight while `now < in_flight_until` (and not delayed).
    in_flight_until: u64,
    /// The latest receipt handle issued for this message.
    handle: Option<u64>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.in_flight_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// How many messages are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A validated send, as far as the queue is concerned.
#[derive(Debug, Clone, Copy)]
pub struct Send<'a> {
    pub body: &'a str,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<&'a str>,
    pub deduplication_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    /// The message was added under the id offered by the caller.
    Accepted { sequence_number: Option<u64> },
    /// A FIFO duplicate: nothing changed; the original's identity is returned.
    Duplicate {
        message_id: u64,
        sequence_number: u64,
    },
}

/// A message handed out by `receive`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub message_id: u64,
    pub receipt_handle: u64,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
}

/// The result of one `receive` pass.
#[derive(Debug, Default)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that exceeded `MaxReceiveCount`, to be moved to the dead-letter queue.
    pub dead_letters: Vec<Message>,
}

#[derive(Debug, Clone, Copy)]
struct Deduplication {
    accepted_at: u64,
    message_id: u64,
    sequence_number: u64,
}

#[derive(Debug)]
pub struct Queue {
    pub config: Config,
    /// Messages in arrival order, keyed by a never-reused arrival counter.
    messages: BTreeMap<u64, Message>,
    next_key: u64,
    /// Latest receipt handle of a message -> its key in `messages`.
    handles: HashMap<u64, u64>,
    deduplication: HashMap<String, Deduplication>,
    next_sequence_number: u64,
    /// The clock value of the last retention sweep; `None` forces a sweep.
    swept_at: Option<u64>,
}

impl Queue {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            next_key: 0,
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_sequence_number: 1,
            swept_at: None,
        }
    }

    /// Removes messages past their retention period.
    ///
    /// Retention only matters when the queue is looked at, so the broker calls
    /// this on access; it is skipped while neither the clock nor the retention
    /// period has changed since the last sweep.
    pub fn expire(&mut self, now: u64) {
        if self.swept_at == Some(now) {
            return;
        }
        self.swept_at = Some(now);
        let retention = u64::from(self.config.retention_seconds);
        let expired: Vec<u64> = self
            .messages
            .iter()
            .filter(|(_, m)| now >= m.sent_at + retention)
            .map(|(&key, _)| key)
            .collect();
        for key in expired {
            self.remove(key);
        }
        self.deduplication
            .retain(|_, d| now - d.accepted_at < DEDUPLICATION_WINDOW);
    }

    /// Forces the next [`expire`](Self::expire) to sweep, after a retention change.
    pub fn invalidate_sweep(&mut self) {
        self.swept_at = None;
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
        self.handles.clear();
    }

    pub fn send(&mut self, now: u64, id: u64, send: Send<'_>) -> Result<SendOutcome> {
        let fifo = self.config.fifo;
        if (!fifo && (send.group_id.is_some() || send.deduplication_id.is_some()))
            || (fifo && send.delay_seconds.is_some())
        {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                if fifo {
                    "FIFO queues do not support per-message delay"
                } else {
                    "standard queues do not support GroupId or DeduplicationId"
                },
            ));
        }
        let mut dedup_key = None;
        if fifo {
            if send.group_id.is_none() {
                return Err(missing("GroupId"));
            }
            let key = match send.deduplication_id {
                Some(id) => id,
                None if self.config.content_based_deduplication => send.body,
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(d) = self.deduplication.get(key)
                && now - d.accepted_at < DEDUPLICATION_WINDOW
            {
                return Ok(SendOutcome::Duplicate {
                    message_id: d.message_id,
                    sequence_number: d.sequence_number,
                });
            }
            dedup_key = Some(key);
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let delay = send.delay_seconds.unwrap_or(self.config.delay_seconds);
        let sequence_number = fifo.then(|| self.take_sequence_number());
        if let (Some(key), Some(sequence_number)) = (dedup_key, sequence_number) {
            self.deduplication.insert(
                key.to_owned(),
                Deduplication {
                    accepted_at: now,
                    message_id: id,
                    sequence_number,
                },
            );
        }
        self.push(Message {
            id,
            body: send.body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: send.group_id.map(str::to_owned),
            sequence_number,
            delayed_until: now + u64::from(delay),
            in_flight_until: 0,
            handle: None,
        });
        Ok(SendOutcome::Accepted { sequence_number })
    }

    /// Examines the messages once each in arrival order (spec section 6).
    ///
    /// `next_handle` is the process-wide receipt handle counter.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: Option<u32>,
        next_handle: &mut u64,
    ) -> Received {
        let timeout = u64::from(visibility_timeout.unwrap_or(self.config.visibility_timeout));
        let mut received = Received::default();
        let mut moved = Vec::new();
        // FIFO groups that still have an earlier message in the queue.
        let mut blocked_groups: HashSet<String> = HashSet::new();

        for (&key, message) in self.messages.iter_mut() {
            if received.deliveries.len() == max_messages {
                break;
            }
            let blocked = |groups: &HashSet<String>, m: &Message| {
                m.group_id.as_ref().is_some_and(|g| groups.contains(g))
            };
            if blocked(&blocked_groups, message) {
                continue;
            }
            let remains = if message.state(now) != State::Visible {
                true
            } else if self
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|dl| message.receive_count >= dl.max_receive_count)
            {
                moved.push(key);
                false
            } else {
                message.receive_count += 1;
                message.first_received_at.get_or_insert(now);
                let handle = *next_handle;
                *next_handle += 1;
                if let Some(old) = message.handle.replace(handle) {
                    self.handles.remove(&old);
                }
                self.handles.insert(handle, key);
                message.in_flight_until = now + timeout;
                received.deliveries.push(Delivery {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at: message.first_received_at.unwrap_or(now),
                    group_id: message.group_id.clone(),
                    sequence_number: message.sequence_number,
                });
                true
            };
            if remains && let Some(group) = &message.group_id {
                blocked_groups.insert(group.clone());
            }
        }

        received.dead_letters = moved
            .into_iter()
            .filter_map(|key| self.remove(key))
            .collect();
        received
    }

    /// Adds a message that was dead-lettered from another queue.
    pub fn accept_dead_letter(&mut self, now: u64, mut message: Message) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delayed_until = 0;
        message.in_flight_until = 0;
        message.handle = None;
        // The number is used up even if the message is dropped right away.
        message.sequence_number = self.config.fifo.then(|| self.take_sequence_number());
        if now < message.sent_at + u64::from(self.config.retention_seconds) {
            self.push(message);
        }
    }

    /// Finds the message that `handle` currently grants access to, if it is
    /// the latest handle of a message that is in flight.
    pub fn find_in_flight(&self, now: u64, handle: u64) -> Result<u64> {
        self.handles
            .get(&handle)
            .and_then(|&key| self.messages.get(&key).map(|m| (key, m)))
            .filter(|(_, m)| m.handle == Some(handle) && m.state(now) == State::InFlight)
            .map(|(key, _)| key)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    "the receipt handle is no longer usable",
                )
            })
    }

    /// Deletes the message found by [`find_in_flight`](Self::find_in_flight).
    pub fn delete(&mut self, key: u64) {
        self.remove(key);
    }

    /// Sets the end of the in-flight period of the message at `key`.
    pub fn set_visibility(&mut self, key: u64, now: u64, timeout: u32) {
        if let Some(message) = self.messages.get_mut(&key) {
            message.in_flight_until = now + u64::from(timeout);
        }
    }

    fn take_sequence_number(&mut self) -> u64 {
        let n = self.next_sequence_number;
        self.next_sequence_number += 1;
        n
    }

    fn push(&mut self, message: Message) {
        self.messages.insert(self.next_key, message);
        self.next_key += 1;
    }

    fn remove(&mut self, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }
}

fn missing(name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("parameter `{name}` is required"),
    )
}
