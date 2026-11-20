//! One queue: its settings, its messages in arrival order, and the rules a single queue applies
//! on its own (retention, message states, FIFO grouping, deduplication, dead-letter moves).
//! Rules that involve several queues, or the queue table, live in `broker`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::error::{ErrorCode, Failure};
use crate::request::{AttributeRequest, DeadLetterRequest, SendRequest};
use crate::types::{
    Body, Clock, DedupKey, GroupId, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber,
};

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_DELAY_SECONDS: Seconds = Seconds(0);
pub const DEFAULT_RETENTION_SECONDS: Seconds = Seconds(345_600);
/// How long a FIFO send's deduplication id keeps a send out (section 6).
pub const DEDUP_WINDOW: Seconds = Seconds(300);
/// Messages one queue may hold in any state (section 6).
pub const MAX_MESSAGES: usize = 100_000;

/// Position of a message in the process's history of additions to queues. Keys are unique across
/// all queues, so a stale key never finds a message that a later queue with the same name holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(u64);

impl ArrivalKey {
    pub const FIRST: ArrivalKey = ArrivalKey(0);

    pub fn next(self) -> ArrivalKey {
        ArrivalKey(
            self.0
                .checked_add(1)
                .expect("arrival keys count additions to queues and cannot reach u64::MAX"),
        )
    }
}

/// The dead-letter configuration of a queue (section 4): both members are set together.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The settings every queue has.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Settings {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes of a queue as section 4 lists them, for `GetQueueAttributes`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    #[serde(flatten)]
    pub settings: Settings,
    pub fifo: bool,
    pub content_based_deduplication: bool,
}

/// Whether a queue is a standard queue or a FIFO queue, with the state only FIFO queues need.
#[derive(Debug)]
pub enum Kind {
    Standard,
    Fifo(FifoState),
}

impl Kind {
    pub fn is_fifo(&self) -> bool {
        matches!(self, Kind::Fifo(_))
    }

    /// False for standard queues, which never deduplicate by content.
    pub fn content_based_deduplication(&self) -> bool {
        match self {
            Kind::Standard => false,
            Kind::Fifo(state) => state.content_based_deduplication,
        }
    }
}

#[derive(Debug)]
pub struct FifoState {
    pub content_based_deduplication: bool,
    /// The sequence number the next accepted message gets.
    next_sequence: SequenceNumber,
    /// Deduplication entries of accepted sends. An entry outlives its message and is only ever
    /// replaced by a send that the window allows, so it stays until the queue is deleted.
    dedup: HashMap<DedupKey, DedupEntry>,
}

#[derive(Debug, Clone, Copy)]
struct DedupEntry {
    sent_at: Clock,
    message: MessageId,
    sequence: SequenceNumber,
}

impl FifoState {
    fn new(content_based_deduplication: bool) -> FifoState {
        FifoState {
            content_based_deduplication,
            next_sequence: SequenceNumber::FIRST,
            dedup: HashMap::new(),
        }
    }

    /// The entry that makes a send with `key` a duplicate at `now`, if any.
    fn duplicate(&self, key: &str, now: Clock) -> Option<DedupEntry> {
        self.dedup
            .get(key)
            .copied()
            .filter(|entry| now < entry.sent_at.after(DEDUP_WINDOW))
    }
}

/// A message's delivery state at one moment (section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

/// The placement of a message in its queue. Only FIFO queues have groups and sequence numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Placement {
    Standard {},
    Fifo {
        #[serde(rename = "GroupId")]
        group: GroupId,
        #[serde(rename = "SequenceNumber")]
        sequence: SequenceNumber,
    },
}

impl Placement {
    fn group(&self) -> Option<&GroupId> {
        match self {
            Placement::Standard {} => None,
            Placement::Fifo { group, .. } => Some(group),
        }
    }
}

/// The latest delivery of a message. The message is in flight while `now < visible_again_at`.
#[derive(Debug, Clone, Copy)]
struct Delivery {
    receipt: ReceiptHandle,
    visible_again_at: Clock,
}

/// The receive history of a message that was delivered at least once.
#[derive(Debug)]
struct Reception {
    first_received_at: Clock,
    receive_count: u32,
    latest: Delivery,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Clock,
    delay: Seconds,
    placement: Placement,
    /// None until the first delivery; ReceiveCount is 0 in that state.
    reception: Option<Reception>,
}

impl Message {
    pub fn state(&self, now: Clock) -> MessageState {
        if now < self.sent_at.after(self.delay) {
            MessageState::Delayed
        } else if self.in_flight(now) {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }

    /// True when `receipt` is the latest handle of this message and its delivery is still running.
    pub fn in_flight_with(&self, receipt: ReceiptHandle, now: Clock) -> bool {
        self.reception.as_ref().is_some_and(|reception| {
            reception.latest.receipt == receipt && now < reception.latest.visible_again_at
        })
    }

    fn in_flight(&self, now: Clock) -> bool {
        self.reception
            .as_ref()
            .is_some_and(|reception| now < reception.latest.visible_again_at)
    }

    fn receive_count(&self) -> u32 {
        self.reception
            .as_ref()
            .map_or(0, |reception| reception.receive_count)
    }

    fn deliver(&mut self, now: Clock, receipt: ReceiptHandle, visibility: Seconds) -> Delivered {
        let latest = Delivery {
            receipt,
            visible_again_at: now.after(visibility),
        };
        let reception = match self.reception.take() {
            None => Reception {
                first_received_at: now,
                receive_count: 1,
                latest,
            },
            Some(previous) => Reception {
                receive_count: previous.receive_count + 1,
                latest,
                ..previous
            },
        };
        let delivered = Delivered {
            message_id: self.id,
            receipt_handle: receipt,
            body: self.body.clone(),
            receive_count: reception.receive_count,
            sent_at: self.sent_at,
            first_received_at: reception.first_received_at,
            placement: self.placement.clone(),
        };
        self.reception = Some(reception);
        delivered
    }

    /// Ends the delivery's visibility timeout at `now + visibility`. The caller has checked that
    /// the handle is in flight, which means the message has been received.
    pub fn change_visibility(&mut self, now: Clock, visibility: Seconds) {
        let reception = self
            .reception
            .as_mut()
            .expect("change_visibility is called only for a message whose handle is in flight");
        reception.latest.visible_again_at = now.after(visibility);
    }
}

/// The messages of one queue: keyed by arrival, plus an index by send time for retention.
#[derive(Debug, Default)]
struct Store {
    by_arrival: BTreeMap<ArrivalKey, Message>,
    /// Ordered by SentAt. Dead-lettered messages keep their SentAt, so arrival order is not
    /// send-time order and retention needs this index.
    by_sent_at: BTreeSet<(Clock, ArrivalKey)>,
}

impl Store {
    fn len(&self) -> usize {
        self.by_arrival.len()
    }

    fn insert(&mut self, arrival: ArrivalKey, message: Message) {
        self.by_sent_at.insert((message.sent_at, arrival));
        self.by_arrival.insert(arrival, message);
    }

    fn get(&self, arrival: ArrivalKey) -> Option<&Message> {
        self.by_arrival.get(&arrival)
    }

    fn get_mut(&mut self, arrival: ArrivalKey) -> &mut Message {
        self.by_arrival
            .get_mut(&arrival)
            .expect("arrival keys handed out by plan_receive name messages in this store")
    }

    /// Removes a message whose arrival key the caller has just read from this store.
    fn take(&mut self, arrival: ArrivalKey) -> Message {
        let message = self
            .by_arrival
            .remove(&arrival)
            .expect("arrival keys in the send-time index name messages in this store");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        message
    }

    fn expire(&mut self, now: Clock, retention: Seconds) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at.after(retention) {
                break;
            }
            self.take(arrival);
        }
    }

    fn ensure_room(&self) -> Result<(), Failure> {
        if self.len() >= MAX_MESSAGES {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                "the queue already holds 100000 messages",
            ));
        }
        Ok(())
    }

    fn iter(&self) -> impl Iterator<Item = (&ArrivalKey, &Message)> {
        self.by_arrival.iter()
    }

    fn clear(&mut self) {
        self.by_arrival.clear();
        self.by_sent_at.clear();
    }
}

#[derive(Debug)]
pub struct Queue {
    settings: Settings,
    kind: Kind,
    store: Store,
}

/// The counts of `GetQueueAttributes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A message a send put in the queue, or the earlier message that a duplicate send matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Sent {
    Standard {
        message_id: MessageId,
    },
    Fifo {
        message_id: MessageId,
        #[serde(rename = "SequenceNumber")]
        sequence: SequenceNumber,
    },
}

/// What a send did. A duplicate changes nothing, so the caller must not spend an id on it.
#[derive(Debug, PartialEq)]
pub enum SendOutcome {
    Accepted(Sent),
    Duplicate(Sent),
}

/// A message in a `ReceiveMessage` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Clock,
    pub first_received_at: Clock,
    #[serde(flatten)]
    pub placement: Placement,
}

/// The outcome of one receive: the deliveries in order, and the messages to move to the
/// dead-letter queue, which the caller moves.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<(ArrivalKey, Delivered)>,
    pub dead_lettered: Vec<Message>,
}

/// What a receive does to one examined message.
enum Step {
    Deliver(ArrivalKey),
    DeadLetter(ArrivalKey),
}

impl Queue {
    pub fn new(settings: Settings, kind: Kind) -> Queue {
        Queue {
            settings,
            kind,
            store: Store::default(),
        }
    }

    /// The empty queue a `CreateQueue` makes. Absent attributes take the defaults of section 4;
    /// the dead-letter target is already resolved by the caller.
    pub fn create(attributes: &AttributeRequest, dead_letter: Option<DeadLetter>) -> Queue {
        let settings = Settings {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: attributes.delay_seconds.unwrap_or(DEFAULT_DELAY_SECONDS),
            retention_seconds: attributes
                .retention_seconds
                .unwrap_or(DEFAULT_RETENTION_SECONDS),
            dead_letter,
        };
        let kind = if attributes.fifo.unwrap_or(false) {
            Kind::Fifo(FifoState::new(
                attributes.content_based_deduplication.unwrap_or(false),
            ))
        } else {
            Kind::Standard
        };
        Queue::new(settings, kind)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn is_fifo(&self) -> bool {
        self.kind.is_fifo()
    }

    pub fn attributes(&self) -> QueueAttributes {
        QueueAttributes {
            settings: self.settings.clone(),
            fifo: self.kind.is_fifo(),
            content_based_deduplication: self.kind.content_based_deduplication(),
        }
    }

    pub fn counts(&self, now: Clock) -> Counts {
        let states = || self.store.iter().map(|(_, message)| message.state(now));
        Counts {
            visible: states()
                .filter(|state| *state == MessageState::Visible)
                .count(),
            in_flight: states()
                .filter(|state| *state == MessageState::InFlight)
                .count(),
            delayed: states()
                .filter(|state| *state == MessageState::Delayed)
                .count(),
        }
    }

    /// Removes every message whose retention has ended. Called before a queue is read, so the
    /// queue never reports a message that section 4 says is gone.
    pub fn expire(&mut self, now: Clock) {
        self.store.expire(now, self.settings.retention_seconds);
    }

    /// True when a `CreateQueue` with `request` would find this queue unchanged (section 5, rule 4).
    pub fn matches(
        &self,
        request: &AttributeRequest,
        dead_letter: Option<&DeadLetterRequest>,
    ) -> bool {
        let dead_letter_matches = match (&self.settings.dead_letter, dead_letter) {
            (None, None) => true,
            (Some(current), Some(requested)) => {
                current.queue.as_str() == requested.queue.as_str()
                    && current.max_receive_count == requested.max_receive_count
            }
            _ => false,
        };
        self.settings.visibility_timeout
            == request
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.settings.delay_seconds == request.delay_seconds.unwrap_or(DEFAULT_DELAY_SECONDS)
            && self.settings.retention_seconds
                == request
                    .retention_seconds
                    .unwrap_or(DEFAULT_RETENTION_SECONDS)
            && self.is_fifo() == request.fifo.unwrap_or(false)
            && self.kind.content_based_deduplication()
                == request.content_based_deduplication.unwrap_or(false)
            && dead_letter_matches
    }

    /// Applies the attributes a `SetQueueAttributes` gave. The caller has checked the rules and
    /// resolved the dead-letter target; `dead_letter` is the new configuration when the request
    /// gave one.
    pub fn update(&mut self, request: &AttributeRequest, dead_letter: Option<DeadLetter>) {
        if let Some(visibility) = request.visibility_timeout {
            self.settings.visibility_timeout = visibility;
        }
        if let Some(delay) = request.delay_seconds {
            self.settings.delay_seconds = delay;
        }
        if let Some(retention) = request.retention_seconds {
            self.settings.retention_seconds = retention;
        }
        if let Some(configuration) = dead_letter {
            self.settings.dead_letter = Some(configuration);
        }
        if let (Kind::Fifo(state), Some(content_based)) =
            (&mut self.kind, request.content_based_deduplication)
        {
            state.content_based_deduplication = content_based;
        }
    }

    pub fn dead_letter_target(&self) -> Option<&QueueName> {
        self.settings
            .dead_letter
            .as_ref()
            .map(|dead_letter| &dead_letter.queue)
    }

    /// Drops the dead-letter configuration if it names `target`; used when that queue is deleted.
    pub fn forget_dead_letter_target(&mut self, target: &QueueName) {
        if self.dead_letter_target() == Some(target) {
            self.settings.dead_letter = None;
        }
    }

    /// Empties the queue. Sequence numbers and deduplication entries carry on (section 5).
    pub fn purge(&mut self) {
        self.store.clear();
    }

    /// Section 6: the rules of a send, in order. Mutates the queue only when the send is accepted.
    pub fn send(
        &mut self,
        now: Clock,
        request: SendRequest,
        id: MessageId,
        arrival: ArrivalKey,
    ) -> Result<SendOutcome, Failure> {
        let SendRequest {
            body,
            delay,
            group,
            deduplication,
        } = request;
        match &mut self.kind {
            Kind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(unsupported(
                        "GroupId and DeduplicationId apply only to FIFO queues",
                    ));
                }
                self.store.ensure_room()?;
                let delay = delay.unwrap_or(self.settings.delay_seconds);
                self.store.insert(
                    arrival,
                    Message {
                        id,
                        body,
                        sent_at: now,
                        delay,
                        placement: Placement::Standard {},
                        reception: None,
                    },
                );
                Ok(SendOutcome::Accepted(Sent::Standard { message_id: id }))
            }
            Kind::Fifo(state) => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds does not apply to FIFO queues"));
                }
                let group = group.ok_or_else(|| {
                    Failure::new(
                        ErrorCode::MissingParameter,
                        "GroupId is required in a FIFO queue",
                    )
                })?;
                let dedup_text = match (&deduplication, state.content_based_deduplication) {
                    (Some(explicit), _) => explicit.as_str(),
                    (None, true) => body.as_str(),
                    (None, false) => {
                        return Err(Failure::new(
                            ErrorCode::MissingParameter,
                            "DeduplicationId is required in a FIFO queue without content-based deduplication",
                        ))
                    }
                };
                if let Some(entry) = state.duplicate(dedup_text, now) {
                    return Ok(SendOutcome::Duplicate(Sent::Fifo {
                        message_id: entry.message,
                        sequence: entry.sequence,
                    }));
                }
                self.store.ensure_room()?;
                let sequence = state.next_sequence;
                state.next_sequence = sequence.next();
                state.dedup.insert(
                    DedupKey::new(dedup_text),
                    DedupEntry {
                        sent_at: now,
                        message: id,
                        sequence,
                    },
                );
                self.store.insert(
                    arrival,
                    Message {
                        id,
                        body,
                        sent_at: now,
                        delay: self.settings.delay_seconds,
                        placement: Placement::Fifo { group, sequence },
                        reception: None,
                    },
                );
                Ok(SendOutcome::Accepted(Sent::Fifo {
                    message_id: id,
                    sequence,
                }))
            }
        }
    }

    /// The message stored under `arrival`, if it is still in this queue.
    pub fn message(&self, arrival: ArrivalKey) -> Option<&Message> {
        self.store.get(arrival)
    }

    /// Changes the visibility of a message the caller has checked is in flight.
    pub fn change_visibility(&mut self, arrival: ArrivalKey, now: Clock, visibility: Seconds) {
        self.store
            .get_mut(arrival)
            .change_visibility(now, visibility);
    }

    /// Removes a message the caller has checked is present.
    pub fn remove(&mut self, arrival: ArrivalKey) {
        self.store.take(arrival);
    }

    /// Section 6, ReceiveMessage: examines messages once each in arrival order and delivers up to
    /// `max_messages` of them. `next_receipt` is advanced by one for each delivery.
    pub fn receive(
        &mut self,
        now: Clock,
        max_messages: u32,
        visibility: Seconds,
        next_receipt: &mut ReceiptHandle,
    ) -> Received {
        let steps = self.plan_receive(now, max_messages);
        let mut deliveries = Vec::new();
        let mut dead_lettered = Vec::new();
        for step in steps {
            match step {
                Step::Deliver(arrival) => {
                    let receipt = *next_receipt;
                    *next_receipt = receipt.next();
                    let delivered = self
                        .store
                        .get_mut(arrival)
                        .deliver(now, receipt, visibility);
                    deliveries.push((arrival, delivered));
                }
                Step::DeadLetter(arrival) => dead_lettered.push(self.store.take(arrival)),
            }
        }
        Received {
            deliveries,
            dead_lettered,
        }
    }

    /// Decides what a receive does, without changing anything. A FIFO group is blocked by its
    /// earliest message that is still in the queue; a message moved to the dead-letter queue
    /// therefore does not block the messages behind it.
    fn plan_receive(&self, now: Clock, max_messages: u32) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut delivered = 0_u32;
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in self.store.iter() {
            if delivered == max_messages {
                break;
            }
            let group = message.placement.group();
            if group.is_some_and(|group| blocked.contains(group)) {
                continue;
            }
            if message.state(now) != MessageState::Visible {
                blocked.extend(group);
                continue;
            }
            if self.dead_letter_due(message) {
                steps.push(Step::DeadLetter(arrival));
                continue;
            }
            steps.push(Step::Deliver(arrival));
            delivered += 1;
            blocked.extend(group);
        }
        steps
    }

    fn dead_letter_due(&self, message: &Message) -> bool {
        self.settings
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| message.receive_count() >= dead_letter.max_receive_count)
    }

    /// Takes a message moved from the source queue. It is dropped at once if it is already past
    /// this queue's retention, and it takes the next sequence number only if it is kept.
    pub fn accept_dead_letter(&mut self, arrival: ArrivalKey, mut message: Message, now: Clock) {
        if now >= message.sent_at.after(self.settings.retention_seconds) {
            return;
        }
        message.delay = DEFAULT_DELAY_SECONDS;
        message.reception = None;
        message.placement = match (message.placement, &mut self.kind) {
            (Placement::Fifo { group, .. }, Kind::Fifo(state)) => {
                let sequence = state.next_sequence;
                state.next_sequence = sequence.next();
                Placement::Fifo { group, sequence }
            }
            (Placement::Standard {}, Kind::Standard) => Placement::Standard {},
            _ => unreachable!("a dead-letter queue has the same type as the queue that feeds it"),
        };
        self.store.insert(arrival, message);
    }
}

fn unsupported(message: &str) -> Failure {
    Failure::new(ErrorCode::UnsupportedOperation, message)
}
