//! One queue: its attributes and its messages, and the rules that need
//! nothing outside the queue.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::error::{ApiError, ErrorCode};
use crate::request::{AttributeUpdate, SendMessage};
use crate::types::{MessageId, MessageIds, ReceiptHandle, ReceiptHandles, Tag, Time};

pub const MAX_MESSAGES: usize = 100_000;

/// How long a FIFO queue remembers a deduplication id after accepting it.
const DEDUPLICATION_WINDOW: u32 = 300;

/// Where a queue sends messages that were received too often.
///
/// `queue` always names an existing queue: deleting that queue removes the
/// configuration from every queue that points at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: String,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

/// All attributes of a queue, as `GetQueueAttributes` reports them and
/// `CreateQueue` compares them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

/// The number of messages in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The result of an accepted or duplicate `SendMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

/// One message as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

/// What one `ReceiveMessage` did to a queue.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages taken out for the dead-letter queue, in examination order.
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    visibility_timeout: u32,
    delay_seconds: u32,
    retention_seconds: u32,
    dead_letter: Option<DeadLetter>,
    kind: Kind,
    messages: MessageStore,
}

#[derive(Debug)]
enum Kind {
    Standard,
    Fifo(Fifo),
}

#[derive(Debug)]
struct Fifo {
    content_based_deduplication: bool,
    last_sequence: u64,
    deduplication: DeduplicationWindow,
}

impl Fifo {
    fn next_sequence(&mut self) -> u64 {
        self.last_sequence += 1;
        self.last_sequence
    }
}

impl Queue {
    /// A new, empty queue. The caller has checked the attributes against the
    /// rules of `CreateQueue`.
    pub fn new(attributes: Attributes) -> Queue {
        debug_assert!(attributes.fifo || !attributes.content_based_deduplication);
        let kind = if attributes.fifo {
            Kind::Fifo(Fifo {
                content_based_deduplication: attributes.content_based_deduplication,
                last_sequence: 0,
                deduplication: DeduplicationWindow::default(),
            })
        } else {
            Kind::Standard
        };
        Queue {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay_seconds,
            retention_seconds: attributes.retention_seconds,
            dead_letter: attributes.dead_letter,
            kind,
            messages: MessageStore::default(),
        }
    }

    pub fn attributes(&self) -> Attributes {
        let content_based_deduplication = match &self.kind {
            Kind::Standard => false,
            Kind::Fifo(fifo) => fifo.content_based_deduplication,
        };
        Attributes {
            visibility_timeout: self.visibility_timeout,
            delay_seconds: self.delay_seconds,
            retention_seconds: self.retention_seconds,
            fifo: self.is_fifo(),
            content_based_deduplication,
            dead_letter: self.dead_letter.clone(),
        }
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo(_))
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.dead_letter.as_ref()
    }

    /// Applies the attributes present in `update`, and `dead_letter` if it
    /// is set. The caller has checked them against `SetQueueAttributes`.
    ///
    /// A lower retention removes messages at the next [`Queue::expire`],
    /// which every lookup of the queue runs before using it.
    pub fn update(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        debug_assert!(update.fifo.is_none());
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = update.retention_seconds {
            self.retention_seconds = value;
        }
        if let (Some(value), Kind::Fifo(fifo)) =
            (update.content_based_deduplication, &mut self.kind)
        {
            fifo.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// Removes the dead-letter configuration if it points at `queue`.
    pub fn forget_dead_letter_queue(&mut self, queue: &str) {
        if self
            .dead_letter
            .as_ref()
            .is_some_and(|dl| dl.queue == queue)
        {
            self.dead_letter = None;
        }
    }

    /// Removes the messages whose retention has ended.
    ///
    /// Retention holds at every moment, so callers run this on each lookup,
    /// before the clock or the retention period can change again.
    pub fn expire(&mut self, now: Time) {
        self.messages.expire(now, self.retention_seconds);
    }

    pub fn counts(&self, now: Time) -> Counts {
        self.messages.counts(now)
    }

    pub fn purge(&mut self) {
        self.messages.clear();
    }

    pub fn send(
        &mut self,
        request: SendMessage,
        now: Time,
        ids: &mut MessageIds,
    ) -> Result<Sent, ApiError> {
        let unsupported = |message| ApiError::new(ErrorCode::UnsupportedOperation, message);
        let missing = |message| ApiError::new(ErrorCode::MissingParameter, message);
        let limit_reached = || {
            ApiError::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            )
        };

        let (id, delay, fifo) = match &mut self.kind {
            Kind::Standard => {
                if request.group_id.is_some() || request.deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                if self.messages.len() >= MAX_MESSAGES {
                    return Err(limit_reached());
                }
                let delay = request.delay_seconds.unwrap_or(self.delay_seconds);
                (ids.next(), delay, None)
            }
            Kind::Fifo(fifo) => {
                if request.delay_seconds.is_some() {
                    return Err(unsupported(
                        "a FIFO queue takes no per-message DelaySeconds",
                    ));
                }
                let group = request
                    .group_id
                    .ok_or_else(|| missing("a FIFO queue needs GroupId"))?;
                let deduplication_id = match request.deduplication_id {
                    Some(id) => id.as_str().to_owned(),
                    None if fifo.content_based_deduplication => request.body.clone(),
                    None => {
                        return Err(missing(
                            "DeduplicationId is required without ContentBasedDeduplication",
                        ));
                    }
                };
                if let Some(original) = fifo.deduplication.find(&deduplication_id, now) {
                    return Ok(original.clone());
                }
                if self.messages.len() >= MAX_MESSAGES {
                    return Err(limit_reached());
                }
                let id = ids.next();
                let sequence = fifo.next_sequence();
                let sent = Sent {
                    message_id: id,
                    sequence_number: Some(sequence),
                };
                fifo.deduplication.record(deduplication_id, now, sent);
                (id, self.delay_seconds, Some(FifoPlace { group, sequence }))
            }
        };

        let message = Message {
            id,
            body: request.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            hold: Hold::Delayed {
                until: now.plus(delay),
            },
            fifo,
        };
        let sent = Sent {
            message_id: message.id,
            sequence_number: message.fifo.as_ref().map(|place| place.sequence),
        };
        self.messages.push(message);
        Ok(sent)
    }

    /// Delivers up to `max` messages, taking out on the way those that have
    /// been received `MaxReceiveCount` times already.
    pub fn receive(
        &mut self,
        now: Time,
        max: usize,
        visibility_timeout: Option<u32>,
        handles: &mut ReceiptHandles,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        let max_receive_count = self.dead_letter.as_ref().map(|dl| dl.max_receive_count);
        self.messages
            .receive(now, max, visibility_timeout, max_receive_count, handles)
    }

    /// Appends a message taken from the queue that names this one as its
    /// dead-letter queue.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Time) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::Delayed { until: now };
        match (&mut self.kind, &mut message.fifo) {
            (Kind::Fifo(fifo), Some(place)) => place.sequence = fifo.next_sequence(),
            (Kind::Standard, None) => {}
            (Kind::Fifo(_), None) | (Kind::Standard, Some(_)) => {
                panic!("a dead-letter queue always has the same type as its source queue")
            }
        }
        self.messages.push(message);
        // The message keeps its SentAt and may already be too old here.
        self.expire(now);
    }

    pub fn delete_message(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), ApiError> {
        let arrival = self
            .messages
            .leased(handle, now)
            .ok_or_else(|| expired(handle))?;
        self.messages.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: u32,
    ) -> Result<(), ApiError> {
        let arrival = self
            .messages
            .leased(handle, now)
            .ok_or_else(|| expired(handle))?;
        let message = self.messages.get_mut(arrival);
        message.hold = Hold::InFlight {
            handle,
            until: now.plus(timeout),
        };
        Ok(())
    }
}

fn expired(handle: ReceiptHandle) -> ApiError {
    ApiError::new(
        ErrorCode::ReceiptHandleExpired,
        format!("{handle} is not the handle of a message in flight in this queue"),
    )
}

/// A message in a queue.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    hold: Hold,
    /// Set exactly when the message is in a FIFO queue.
    fifo: Option<FifoPlace>,
}

#[derive(Debug)]
struct FifoPlace {
    group: Tag,
    sequence: u64,
}

/// Why a message may be hidden. Whether it still is depends on the clock.
#[derive(Debug, Clone, Copy)]
enum Hold {
    /// Sent (or dead-lettered) with a delay; hidden while `now < until`.
    Delayed { until: Time },
    /// Delivered with `handle`; hidden while `now < until`.
    InFlight { handle: ReceiptHandle, until: Time },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

impl Message {
    fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            Hold::Delayed { .. } | Hold::InFlight { .. } => State::Visible,
        }
    }

    fn lease_handle(&self) -> Option<ReceiptHandle> {
        match self.hold {
            Hold::InFlight { handle, .. } => Some(handle),
            Hold::Delayed { .. } => None,
        }
    }
}

/// The messages of one queue in arrival order, with the indexes that keep
/// sends, expiry and handle lookups from scanning the whole queue.
#[derive(Debug, Default)]
struct MessageStore {
    last_arrival: u64,
    by_arrival: BTreeMap<u64, Message>,
    /// `(SentAt, arrival)` of every message. Dead-lettered messages arrive
    /// with an old `SentAt`, so arrival order alone does not sort by age.
    by_sent_at: BTreeSet<(Time, u64)>,
    /// The latest handle of every message that has been delivered.
    handles: HashMap<ReceiptHandle, u64>,
}

impl MessageStore {
    fn len(&self) -> usize {
        self.by_arrival.len()
    }

    fn push(&mut self, message: Message) {
        self.last_arrival += 1;
        let arrival = self.last_arrival;
        self.by_sent_at.insert((message.sent_at, arrival));
        if let Some(handle) = message.lease_handle() {
            self.handles.insert(handle, arrival);
        }
        self.by_arrival.insert(arrival, message);
    }

    fn get_mut(&mut self, arrival: u64) -> &mut Message {
        self.by_arrival
            .get_mut(&arrival)
            .expect("arrival keys come from this store's own indexes")
    }

    fn remove(&mut self, arrival: u64) -> Message {
        let message = self
            .by_arrival
            .remove(&arrival)
            .expect("arrival keys come from this store's own indexes");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.lease_handle() {
            self.handles.remove(&handle);
        }
        message
    }

    fn clear(&mut self) {
        self.by_arrival.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    fn expire(&mut self, now: Time, retention_seconds: u32) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at.plus(retention_seconds) {
                break;
            }
            self.remove(arrival);
        }
    }

    fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.by_arrival.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// The arrival key of the message in flight under `handle` right now.
    fn leased(&self, handle: ReceiptHandle, now: Time) -> Option<u64> {
        let arrival = *self.handles.get(&handle)?;
        let message = &self.by_arrival[&arrival];
        let in_flight = matches!(
            message.hold,
            Hold::InFlight { handle: current, until } if current == handle && now < until
        );
        in_flight.then_some(arrival)
    }

    fn receive(
        &mut self,
        now: Time,
        max: usize,
        visibility_timeout: u32,
        max_receive_count: Option<u32>,
        handles: &mut ReceiptHandles,
    ) -> Received {
        let mut deliveries = Vec::new();
        let mut to_dead_letter = Vec::new();
        // Groups with a message that stays in the queue ahead of the current
        // one. The scan starts at the head, so this sees every such message.
        let mut blocked_groups: HashSet<Tag> = HashSet::new();

        for (&arrival, message) in &mut self.by_arrival {
            if deliveries.len() == max {
                break;
            }
            let blocked = message
                .fifo
                .as_ref()
                .is_some_and(|place| blocked_groups.contains(&place.group));
            let eligible = !blocked && message.state(now) == State::Visible;
            if eligible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                to_dead_letter.push(arrival);
                continue;
            }
            if let Some(place) = &message.fifo
                && !blocked_groups.contains(&place.group)
            {
                blocked_groups.insert(place.group.clone());
            }
            if !eligible {
                continue;
            }

            let handle = handles.next();
            if let Some(previous) = message.lease_handle() {
                self.handles.remove(&previous);
            }
            self.handles.insert(handle, arrival);
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.hold = Hold::InFlight {
                handle,
                until: now.plus(visibility_timeout),
            };
            deliveries.push(Delivery {
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

        let dead_lettered = to_dead_letter
            .into_iter()
            .map(|arrival| self.remove(arrival))
            .collect();
        Received {
            deliveries,
            dead_lettered,
        }
    }
}

/// The deduplication ids a FIFO queue accepted in the last 300 seconds.
///
/// Ids are accepted at non-decreasing times, so `order` is sorted by time
/// and old entries leave from its front.
#[derive(Debug, Default)]
struct DeduplicationWindow {
    accepted: HashMap<String, (Time, Sent)>,
    order: VecDeque<(Time, String)>,
}

impl DeduplicationWindow {
    /// The original send if `id` was accepted less than 300 seconds ago.
    fn find(&mut self, id: &str, now: Time) -> Option<&Sent> {
        self.forget_before(now);
        self.accepted.get(id).map(|(_, sent)| sent)
    }

    fn record(&mut self, id: String, now: Time, sent: Sent) {
        self.order.push_back((now, id.clone()));
        self.accepted.insert(id, (now, sent));
    }

    fn forget_before(&mut self, now: Time) {
        // An id is only accepted again after its entry has left `order`
        // here, so `order` never holds two entries for one id.
        while self
            .order
            .front()
            .is_some_and(|(accepted_at, _)| now >= accepted_at.plus(DEDUPLICATION_WINDOW))
        {
            let (_, id) = self.order.pop_front().expect("front() was just Some");
            self.accepted.remove(&id);
        }
    }
}
