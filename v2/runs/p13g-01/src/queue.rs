//! One queue: its attributes, its messages in arrival order, and the rules that act on
//! the queue alone. Rules that involve a second queue live in [`crate::broker`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Bound;

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::request::{AttributeSet, NewMessage};
use crate::types::{
    Body, DelaySeconds, GroupId, IdSource, MaxMessages, MaxReceiveCount, MessageId, QueueName,
    ReceiptHandle, RetentionSeconds, SequenceCounter, SequenceNumber, Timestamp, VisibilityTimeout,
};

const MESSAGE_LIMIT: usize = 100_000;
const DEDUPLICATION_WINDOW_SECONDS: u32 = 300;

const DEFAULT_VISIBILITY_TIMEOUT: VisibilityTimeout = VisibilityTimeout::constant(30);
const DEFAULT_DELAY: DelaySeconds = DelaySeconds::constant(0);
const DEFAULT_RETENTION: RetentionSeconds = RetentionSeconds::constant(345_600);

/// Position of a message in the queue; grows with every message added, so it orders
/// messages by arrival even though dead-lettered messages arrive with an old `SentAt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Why a message is not visible. A hold that has run out means the message is visible.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Delayed {
        until: Timestamp,
    },
    InFlight {
        until: Timestamp,
        handle: ReceiptHandle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

/// What only a message of a FIFO queue has.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoTag {
    group_id: GroupId,
    sequence_number: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Timestamp,
    receive_count: u64,
    first_received_at: Option<Timestamp>,
    fifo: Option<FifoTag>,
    hold: Option<Hold>,
}

impl Message {
    fn state(&self, now: Timestamp) -> State {
        match self.hold {
            Some(Hold::Delayed { until }) if now < until => State::Delayed,
            Some(Hold::InFlight { until, .. }) if now < until => State::InFlight,
            _ => State::Visible,
        }
    }

    fn is_in_flight_with(&self, handle: ReceiptHandle, now: Timestamp) -> bool {
        matches!(
            self.hold,
            Some(Hold::InFlight { until, handle: latest }) if latest == handle && now < until
        )
    }

    /// The state a message has when it arrives in a dead-letter queue.
    fn reset(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.hold = None;
    }
}

/// The messages of a queue with the indexes that keep every operation but a scan cheap.
#[derive(Debug, Default)]
struct Store {
    messages: BTreeMap<Arrival, Message>,
    next_arrival: u64,
    /// `(SentAt, arrival)` of every message, so retention only looks at the oldest ones.
    by_age: BTreeSet<(Timestamp, Arrival)>,
    /// The latest receipt handle of every message that was ever delivered and is still here.
    handles: HashMap<ReceiptHandle, Arrival>,
}

impl Store {
    fn push(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn take(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(Hold::InFlight { handle, .. }) = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    fn clear(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    fn expire(&mut self, now: Timestamp, retention: RetentionSeconds) {
        while let Some(&(sent_at, arrival)) = self.by_age.first()
            && now >= sent_at.after(retention.get())
        {
            self.take(arrival);
        }
    }

    fn ensure_room(&self) -> Result<(), Error> {
        if self.messages.len() >= MESSAGE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MESSAGE_LIMIT} messages"),
            ));
        }
        Ok(())
    }
}

/// A deduplication id: the caller's `DeduplicationId`, or else the message body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DedupKey(String);

#[derive(Debug)]
struct Original {
    accepted_at: Timestamp,
    sent: Sent,
}

#[derive(Debug)]
struct FifoState {
    content_based_deduplication: bool,
    sequence: SequenceCounter,
    recent: HashMap<DedupKey, Original>,
    /// Keys in the order their windows opened, so stale entries can be dropped from the front.
    window: VecDeque<(Timestamp, DedupKey)>,
}

impl FifoState {
    fn original(&self, key: &DedupKey, now: Timestamp) -> Option<Sent> {
        self.recent
            .get(key)
            .filter(|original| now < original.accepted_at.after(DEDUPLICATION_WINDOW_SECONDS))
            .map(|original| original.sent)
    }

    fn remember(&mut self, key: DedupKey, now: Timestamp, sent: Sent) {
        while let Some((opened_at, stale)) = self
            .window
            .pop_front_if(|(opened_at, _)| now >= opened_at.after(DEDUPLICATION_WINDOW_SECONDS))
        {
            // A newer window for the same key must survive the removal of the older entry.
            if self
                .recent
                .get(&stale)
                .is_some_and(|original| original.accepted_at == opened_at)
            {
                self.recent.remove(&stale);
            }
        }
        self.window.push_back((now, key.clone()));
        self.recent.insert(
            key,
            Original {
                accepted_at: now,
                sent,
            },
        );
    }
}

#[derive(Debug)]
enum Kind {
    Standard,
    Fifo(FifoState),
}

/// Where messages go after too many deliveries.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug)]
struct Settings {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: Body,
    receive_count: u64,
    sent_at: Timestamp,
    first_received_at: Timestamp,
    #[serde(flatten)]
    fifo: Option<FifoTag>,
}

/// The outcome of one scan of the queue by `ReceiveMessage`.
#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<Delivered>,
    /// Messages that left the queue for its dead-letter queue, in the order they left.
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    visible: u64,
    in_flight: u64,
    delayed: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<DeadLetter>,
}

#[derive(Debug)]
pub struct Queue {
    settings: Settings,
    kind: Kind,
    store: Store,
}

fn missing(name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("{name} is required on a FIFO queue"),
    )
}

fn expired_handle() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle does not belong to a message that is in flight in this queue",
    )
}

impl Queue {
    /// Builds an empty queue from request attributes that the broker has validated.
    pub fn new(attributes: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if attributes.fifo.unwrap_or(false) {
            Kind::Fifo(FifoState {
                content_based_deduplication: attributes
                    .content_based_deduplication
                    .unwrap_or(false),
                sequence: SequenceCounter::default(),
                recent: HashMap::new(),
                window: VecDeque::new(),
            })
        } else {
            Kind::Standard
        };
        Self {
            settings: Settings {
                visibility_timeout: attributes
                    .visibility_timeout
                    .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
                delay_seconds: attributes.delay_seconds.unwrap_or(DEFAULT_DELAY),
                retention_seconds: attributes.retention_seconds.unwrap_or(DEFAULT_RETENTION),
                dead_letter,
            },
            kind,
            store: Store::default(),
        }
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo(_))
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.settings.dead_letter.as_ref()
    }

    /// Whether creating a queue with `requested` would describe this very queue.
    pub fn matches(&self, requested: &AttributeSet) -> bool {
        let content_based = matches!(
            &self.kind,
            Kind::Fifo(state) if state.content_based_deduplication
        );
        let dead_letter = self.dead_letter();
        requested
            .visibility_timeout
            .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            == self.settings.visibility_timeout
            && requested.delay_seconds.unwrap_or(DEFAULT_DELAY) == self.settings.delay_seconds
            && requested.retention_seconds.unwrap_or(DEFAULT_RETENTION)
                == self.settings.retention_seconds
            && requested.fifo.unwrap_or(false) == self.is_fifo()
            && requested.content_based_deduplication.unwrap_or(false) == content_based
            && requested.dead_letter_queue.as_deref()
                == dead_letter.map(|dead_letter| dead_letter.queue.as_str())
            && requested.max_receive_count == dead_letter.map(|d| d.max_receive_count)
    }

    /// Takes over the attributes `changes` names. The caller has checked them against the
    /// rules of SetQueueAttributes and resolved the dead-letter configuration.
    pub fn apply(&mut self, changes: &AttributeSet, dead_letter: Option<DeadLetter>) {
        let settings = &mut self.settings;
        settings.visibility_timeout = changes
            .visibility_timeout
            .unwrap_or(settings.visibility_timeout);
        settings.delay_seconds = changes.delay_seconds.unwrap_or(settings.delay_seconds);
        settings.retention_seconds = changes
            .retention_seconds
            .unwrap_or(settings.retention_seconds);
        if let (Kind::Fifo(state), Some(enabled)) =
            (&mut self.kind, changes.content_based_deduplication)
        {
            state.content_based_deduplication = enabled;
        }
        if dead_letter.is_some() {
            settings.dead_letter = dead_letter;
        }
    }

    pub fn clear_dead_letter_to(&mut self, deleted: &str) {
        if self
            .dead_letter()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == deleted)
        {
            self.settings.dead_letter = None;
        }
    }

    pub fn attributes(&self) -> AttributesView {
        AttributesView {
            visibility_timeout: self.settings.visibility_timeout,
            delay_seconds: self.settings.delay_seconds,
            retention_seconds: self.settings.retention_seconds,
            fifo: self.is_fifo(),
            content_based_deduplication: matches!(
                &self.kind,
                Kind::Fifo(state) if state.content_based_deduplication
            ),
            dead_letter: self.settings.dead_letter.clone(),
        }
    }

    pub fn counts(&self, now: Timestamp) -> Counts {
        let mut counts = Counts::default();
        for message in self.store.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Removes the messages whose retention has run out. Every command that looks at the
    /// queue calls this first, and a retention change is applied only after it.
    pub fn expire(&mut self, now: Timestamp) {
        self.store.expire(now, self.settings.retention_seconds);
    }

    pub fn purge(&mut self) {
        self.store.clear();
    }

    pub fn send(
        &mut self,
        now: Timestamp,
        ids: &mut IdSource,
        new: NewMessage,
    ) -> Result<Sent, Error> {
        let unsupported = |what: &str| {
            Err(Error::new(
                ErrorCode::UnsupportedOperation,
                format!("{what} is not supported by this type of queue"),
            ))
        };
        let delay = new.delay_seconds.unwrap_or(self.settings.delay_seconds);
        let (sent, fifo) = match &mut self.kind {
            Kind::Standard => {
                if new.group_id.is_some() || new.deduplication_id.is_some() {
                    return unsupported("GroupId and DeduplicationId");
                }
                self.store.ensure_room()?;
                let sent = Sent {
                    message_id: ids.next_message(),
                    sequence_number: None,
                };
                (sent, None)
            }
            Kind::Fifo(state) => {
                if new.delay_seconds.is_some() {
                    return unsupported("DelaySeconds on a message");
                }
                let group_id = new.group_id.ok_or_else(|| missing("GroupId"))?;
                let key = match new.deduplication_id {
                    Some(id) => DedupKey(id.into_string()),
                    None if state.content_based_deduplication => {
                        DedupKey(new.body.as_str().to_owned())
                    }
                    None => return Err(missing("DeduplicationId")),
                };
                if let Some(original) = state.original(&key, now) {
                    return Ok(original);
                }
                self.store.ensure_room()?;
                let sequence_number = state.sequence.issue();
                let sent = Sent {
                    message_id: ids.next_message(),
                    sequence_number: Some(sequence_number),
                };
                state.remember(key, now, sent);
                let tag = FifoTag {
                    group_id,
                    sequence_number,
                };
                (sent, Some(tag))
            }
        };
        self.store.push(Message {
            id: sent.message_id,
            body: new.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Some(Hold::Delayed {
                until: now.after(delay.get()),
            }),
        });
        Ok(sent)
    }

    /// Scans the queue once in arrival order, delivering up to `max_messages` messages and
    /// moving out the ones that exhausted their deliveries.
    pub fn receive(
        &mut self,
        now: Timestamp,
        ids: &mut IdSource,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.settings.visibility_timeout);
        let mut received = Received::default();
        let mut remaining = max_messages.get();
        // Groups that already have an earlier message in the queue. A message moved out to
        // the dead-letter queue is no longer in the queue and does not block its group.
        let mut occupied_groups = HashSet::new();
        let mut cursor = Bound::Unbounded;
        while remaining > 0 {
            let Some((&arrival, message)) = self
                .store
                .messages
                .range_mut((cursor, Bound::Unbounded))
                .next()
            else {
                break;
            };
            cursor = Bound::Excluded(arrival);
            let eligible = message.state(now) == State::Visible
                && message
                    .fifo
                    .as_ref()
                    .is_none_or(|tag| !occupied_groups.contains(&tag.group_id));
            let exhausted = self
                .settings
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| {
                    message.receive_count >= u64::from(dead_letter.max_receive_count.get())
                });
            if eligible && exhausted {
                received.dead_lettered.extend(self.store.take(arrival));
                continue;
            }
            if let Some(tag) = &message.fifo
                && !occupied_groups.contains(&tag.group_id)
            {
                occupied_groups.insert(tag.group_id.clone());
            }
            if !eligible {
                continue;
            }
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            let handle = ids.next_receipt();
            let latest = Hold::InFlight {
                until: now.after(visibility_timeout.get()),
                handle,
            };
            if let Some(Hold::InFlight {
                handle: superseded, ..
            }) = message.hold.replace(latest)
            {
                self.store.handles.remove(&superseded);
            }
            self.store.handles.insert(handle, arrival);
            received.delivered.push(Delivered {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                fifo: message.fifo.clone(),
            });
            remaining -= 1;
        }
        received
    }

    /// Adds a message that was dead-lettered from a queue of the same type.
    pub fn adopt(&mut self, mut message: Message) {
        message.reset();
        if let (Kind::Fifo(state), Some(tag)) = (&mut self.kind, message.fifo.as_mut()) {
            tag.sequence_number = state.sequence.issue();
        }
        self.store.push(message);
    }

    pub fn delete(&mut self, now: Timestamp, handle: ReceiptHandle) -> Result<(), Error> {
        let (arrival, _) = self.locate(now, handle)?;
        self.store.take(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        now: Timestamp,
        handle: ReceiptHandle,
        visibility_timeout: VisibilityTimeout,
    ) -> Result<(), Error> {
        let (_, message) = self.locate(now, handle)?;
        message.hold = Some(Hold::InFlight {
            until: now.after(visibility_timeout.get()),
            handle,
        });
        Ok(())
    }

    /// Finds the message that `handle` is the latest handle of, if it is in flight now.
    fn locate(
        &mut self,
        now: Timestamp,
        handle: ReceiptHandle,
    ) -> Result<(Arrival, &mut Message), Error> {
        let arrival = *self.store.handles.get(&handle).ok_or_else(expired_handle)?;
        self.store
            .messages
            .get_mut(&arrival)
            .filter(|message| message.is_in_flight_with(handle, now))
            .map(|message| (arrival, message))
            .ok_or_else(expired_handle)
    }
}
