//! One queue: its attributes, its messages and the rules that act on them.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Bound;

use crate::attributes::AttributeSet;
use crate::error::{ApiError, ErrorCode};
use crate::reply::{AttributesView, Counts, ReceivedMessage, SendOutput};
use crate::types::{
    DeduplicationId, GroupId, HandleIssuer, MessageId, QueueName, ReceiptHandle, Seconds,
    SequenceNumber, Time,
};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo,
}

/// Where a queue sends messages that were received too often.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The attributes of a queue after defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    content_based_deduplication: bool,
    dead_letter: Option<DeadLetter>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visibility_timeout: Seconds(30),
            delay_seconds: Seconds(0),
            retention_seconds: Seconds(345_600),
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl Settings {
    /// Overwrites the settings that `attributes` gives; `dead_letter` is the
    /// resolved form of its dead-letter members. `Fifo` is not a setting.
    pub fn apply(&mut self, attributes: &AttributeSet, dead_letter: Option<DeadLetter>) {
        if let Some(value) = attributes.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = attributes.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = attributes.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = attributes.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// Whether creating a queue with `attributes` would give these settings.
    /// The dead-letter queue is compared by name, without resolving it.
    pub fn matches(&self, attributes: &AttributeSet) -> bool {
        let defaults = Settings::default();
        let requested_dead_letter = attributes
            .dead_letter_queue
            .as_ref()
            .zip(attributes.max_receive_count)
            .map(|(queue, max)| (queue.as_str(), max));
        let current_dead_letter = self
            .dead_letter
            .as_ref()
            .map(|dead| (dead.queue.as_str(), dead.max_receive_count));
        self.visibility_timeout
            == attributes
                .visibility_timeout
                .unwrap_or(defaults.visibility_timeout)
            && self.delay_seconds == attributes.delay_seconds.unwrap_or(defaults.delay_seconds)
            && self.retention_seconds
                == attributes
                    .retention_seconds
                    .unwrap_or(defaults.retention_seconds)
            && self.content_based_deduplication
                == attributes
                    .content_based_deduplication
                    .unwrap_or(defaults.content_based_deduplication)
            && current_dead_letter == requested_dead_letter
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    /// Present exactly in FIFO queues.
    group: Option<GroupId>,
    /// The message is hidden (delayed or in flight) until this time.
    hidden_until: Time,
    /// The latest delivery; a message that was never delivered has none.
    handle: Option<ReceiptHandle>,
}

impl Message {
    fn state(&self, now: Time) -> State {
        if now >= self.hidden_until {
            State::Visible
        } else if self.handle.is_some() {
            State::InFlight
        } else {
            State::Delayed
        }
    }
}

/// The key of a deduplication id: the id itself or, without one, the body.
#[derive(Debug, PartialEq, Eq, Hash)]
struct DeduplicationKey(String);

impl Borrow<str> for DeduplicationKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
struct Accepted {
    at: Time,
    message_id: MessageId,
    sequence: SequenceNumber,
}

/// The parameters of a `SendMessage` after validation of their values.
#[derive(Debug)]
pub struct SendParams {
    pub body: String,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

pub enum SendOutcome {
    Accepted(SendOutput),
    /// Nothing changed; the output is the original message's.
    Duplicate(SendOutput),
}

/// What one `ReceiveMessage` produced.
pub struct Received {
    pub delivered: Vec<ReceivedMessage>,
    /// Messages removed from this queue that belong in its dead-letter queue.
    pub dead_letters: Vec<DeadLetterMessage>,
}

/// A message on its way to a dead-letter queue.
pub struct DeadLetterMessage(Message);

#[derive(Debug)]
pub struct Queue {
    kind: QueueKind,
    settings: Settings,
    /// In arrival order.
    messages: BTreeMap<SequenceNumber, Message>,
    /// `(sent_at, position)` of every message, to find the expired ones.
    by_age: BTreeSet<(Time, SequenceNumber)>,
    /// The latest handle of every delivered message.
    handles: HashMap<ReceiptHandle, SequenceNumber>,
    next_sequence: SequenceNumber,
    deduplication: HashMap<DeduplicationKey, Accepted>,
}

impl Queue {
    pub fn new(kind: QueueKind, settings: Settings) -> Self {
        Self {
            kind,
            settings,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            next_sequence: SequenceNumber::FIRST,
            deduplication: HashMap::new(),
        }
    }

    pub fn kind(&self) -> QueueKind {
        self.kind
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.settings.dead_letter.as_ref()
    }

    pub fn drop_dead_letter(&mut self, deleted: &str) {
        if self
            .dead_letter()
            .is_some_and(|dead| dead.queue.as_str() == deleted)
        {
            self.settings.dead_letter = None;
        }
    }

    /// Changes the attributes, then removes the messages that the new
    /// retention period has already expired.
    pub fn set_attributes(
        &mut self,
        attributes: &AttributeSet,
        dead_letter: Option<DeadLetter>,
        now: Time,
    ) {
        self.settings.apply(attributes, dead_letter);
        self.expire(now);
    }

    pub fn attributes(&self) -> AttributesView {
        let dead_letter = self.dead_letter();
        AttributesView {
            visibility_timeout: self.settings.visibility_timeout,
            delay_seconds: self.settings.delay_seconds,
            retention_seconds: self.settings.retention_seconds,
            fifo: self.kind == QueueKind::Fifo,
            content_based_deduplication: self.settings.content_based_deduplication,
            dead_letter_queue: dead_letter.map(|dead| dead.queue.clone()),
            max_receive_count: dead_letter.map(|dead| dead.max_receive_count),
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
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

    /// Removes every message that is past its retention period. Every command
    /// calls this before it looks at the queue.
    pub fn expire(&mut self, now: Time) {
        while let Some(&(sent_at, position)) = self.by_age.first() {
            if now < sent_at.plus(self.settings.retention_seconds) {
                break;
            }
            self.remove(position);
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    fn push(&mut self, message: Message) -> SequenceNumber {
        let position = self.next_sequence.take();
        self.by_age.insert((message.sent_at, position));
        self.messages.insert(position, message);
        position
    }

    fn remove(&mut self, position: SequenceNumber) -> Message {
        let message = self
            .messages
            .remove(&position)
            .expect("positions in by_age and handles refer to stored messages");
        self.by_age.remove(&(message.sent_at, position));
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        message
    }

    /// Adds a message that another queue gave up on, as a fresh visible one.
    pub fn accept_dead_letter(&mut self, moved: DeadLetterMessage, now: Time) {
        let DeadLetterMessage(old) = moved;
        self.push(Message {
            id: old.id,
            body: old.body,
            sent_at: old.sent_at,
            receive_count: 0,
            first_received_at: None,
            group: old.group,
            hidden_until: Time::START,
            handle: None,
        });
        self.expire(now);
    }

    /// Accepts the message as `id`, unless the send is refused or repeats a
    /// recent one.
    pub fn send(
        &mut self,
        now: Time,
        params: SendParams,
        id: MessageId,
    ) -> Result<SendOutcome, ApiError> {
        let fifo = self.kind == QueueKind::Fifo;
        if fifo {
            if params.delay.is_some() {
                return Err(unsupported("DelaySeconds is not supported by a FIFO queue"));
            }
        } else if params.group.is_some() || params.deduplication_id.is_some() {
            return Err(unsupported(
                "GroupId and DeduplicationId are only supported by a FIFO queue",
            ));
        }

        let mut deduplication_key = None;
        if fifo {
            if params.group.is_none() {
                return Err(ApiError::missing("GroupId"));
            }
            let key = match &params.deduplication_id {
                Some(dedup_id) => dedup_id.as_str(),
                None if self.settings.content_based_deduplication => &params.body,
                None => return Err(ApiError::missing("DeduplicationId")),
            };
            let recent = self
                .deduplication
                .get(key)
                .filter(|accepted| now < accepted.at.plus(DEDUPLICATION_WINDOW));
            if let Some(original) = recent {
                return Ok(SendOutcome::Duplicate(SendOutput {
                    message_id: original.message_id,
                    sequence_number: Some(original.sequence),
                }));
            }
            deduplication_key = Some(DeduplicationKey(key.to_owned()));
        }

        if self.messages.len() >= MAX_MESSAGES {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let delay = params.delay.unwrap_or(self.settings.delay_seconds);
        let sequence = self.push(Message {
            id,
            body: params.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group: params.group,
            hidden_until: now.plus(delay),
            handle: None,
        });
        if let Some(key) = deduplication_key {
            self.deduplication.insert(
                key,
                Accepted {
                    at: now,
                    message_id: id,
                    sequence,
                },
            );
        }
        Ok(SendOutcome::Accepted(SendOutput {
            message_id: id,
            sequence_number: fifo.then_some(sequence),
        }))
    }

    /// Delivers up to `max` messages. Messages received too often leave the
    /// queue and are returned for its dead-letter queue.
    pub fn receive(
        &mut self,
        now: Time,
        max: u32,
        visibility_timeout: Option<Seconds>,
        issuer: &mut HandleIssuer,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.settings.visibility_timeout);
        let max_receive_count = self.dead_letter().map(|dead| dead.max_receive_count);
        let max = usize::try_from(max).unwrap_or(usize::MAX);
        let fifo = self.kind == QueueKind::Fifo;
        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        // Groups that still have an earlier message in the queue.
        let mut blocked_groups: HashSet<GroupId> = HashSet::new();
        let mut after = Bound::Unbounded;

        while delivered.len() < max {
            let Some((&position, message)) =
                self.messages.range_mut((after, Bound::Unbounded)).next()
            else {
                break;
            };
            after = Bound::Excluded(position);

            let group_blocked = message
                .group
                .as_ref()
                .is_some_and(|group| blocked_groups.contains(group));
            if message.state(now) == State::Visible && !group_blocked {
                if max_receive_count.is_some_and(|max| message.receive_count >= max) {
                    dead_letters.push(DeadLetterMessage(self.remove(position)));
                    continue;
                }
                let handle = issuer.issue();
                message.receive_count = message.receive_count.saturating_add(1);
                let first_received_at = *message.first_received_at.get_or_insert(now);
                message.hidden_until = now.plus(visibility_timeout);
                if let Some(previous) = message.handle.replace(handle) {
                    self.handles.remove(&previous);
                }
                self.handles.insert(handle, position);
                delivered.push(ReceivedMessage {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.group.clone(),
                    sequence_number: fifo.then_some(position),
                });
            }
            if let Some(group) = &message.group {
                blocked_groups.insert(group.clone());
            }
        }
        Received {
            delivered,
            dead_letters,
        }
    }

    fn in_flight_mut(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
    ) -> Result<(SequenceNumber, &mut Message), ApiError> {
        let position = self.handles.get(&handle).copied();
        position
            .and_then(|position| Some((position, self.messages.get_mut(&position)?)))
            .filter(|(_, message)| message.state(now) == State::InFlight)
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is not usable"),
                )
            })
    }

    pub fn delete_message(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), ApiError> {
        let (position, _) = self.in_flight_mut(handle, now)?;
        self.remove(position);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: Seconds,
        now: Time,
    ) -> Result<(), ApiError> {
        let (_, message) = self.in_flight_mut(handle, now)?;
        message.hidden_until = now.plus(timeout);
        Ok(())
    }
}

fn unsupported(message: &str) -> ApiError {
    ApiError::new(ErrorCode::UnsupportedOperation, message)
}
