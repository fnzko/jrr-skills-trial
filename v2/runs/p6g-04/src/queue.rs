use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use serde::Serialize;

use crate::reply::{Attributes, Counts, ReceivedMessage, SequenceNumber};
use crate::types::{GroupId, Ids, MessageId, QueueName, ReceiptHandle};

pub(crate) const MAX_MESSAGES: usize = 100_000;

/// A deduplication id blocks later sends for this long after the send that
/// opened its window.
const DEDUPLICATION_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub(crate) fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

/// The attributes of a queue other than its dead-letter configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) visibility_timeout: Duration,
    pub(crate) delay: Duration,
    pub(crate) retention: Duration,
    pub(crate) kind: QueueKind,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visibility_timeout: Duration::from_secs(30),
            delay: Duration::ZERO,
            retention: Duration::from_secs(345_600),
            kind: QueueKind::Standard,
        }
    }
}

/// Where a queue sends messages that were received too often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub(crate) queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub(crate) max_receive_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
enum Phase {
    /// Hidden until `until`, from the delay of the send.
    Delayed {
        until: Duration,
    },
    /// Hidden until `until`, from the delivery that issued `receipt`.
    Delivered {
        until: Duration,
        receipt: ReceiptHandle,
    },
    Visible,
}

#[derive(Debug)]
pub(crate) struct Message {
    id: MessageId,
    body: String,
    sent_at: Duration,
    receive_count: u32,
    first_received_at: Option<Duration>,
    /// Set exactly for messages of FIFO queues.
    group: Option<GroupId>,
    phase: Phase,
}

impl Message {
    pub(crate) fn sent(
        id: MessageId,
        body: String,
        now: Duration,
        delay: Duration,
        group: Option<GroupId>,
    ) -> Self {
        Self {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            phase: Phase::Delayed { until: now + delay },
        }
    }

    fn state(&self, now: Duration) -> State {
        match self.phase {
            Phase::Delayed { until } if now < until => State::Delayed,
            Phase::Delivered { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }

    fn reset_for_dead_letter(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.phase = Phase::Visible;
    }
}

/// The outcome of a send that an earlier send with the same deduplication id
/// already settled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Accepted {
    pub(crate) message_id: MessageId,
    pub(crate) sequence_number: SequenceNumber,
    at: Duration,
}

/// What one `ReceiveMessage` did to a queue.
#[derive(Debug, Default)]
pub(crate) struct Received {
    pub(crate) delivered: Vec<ReceivedMessage>,
    /// Messages taken out of the queue, ready to be added to the dead-letter
    /// queue.
    pub(crate) dead: Vec<Message>,
}

enum Step {
    Deliver(SequenceNumber),
    DeadLetter(SequenceNumber),
}

#[derive(Debug)]
pub(crate) struct Queue {
    settings: Settings,
    dead_letter: Option<DeadLetter>,
    /// In arrival order; the key is the sequence number the message was
    /// added under.
    messages: BTreeMap<SequenceNumber, Message>,
    /// Every message by send time, so retention only looks at the oldest.
    by_age: BTreeSet<(Duration, SequenceNumber)>,
    /// The latest receipt handle of each delivered message.
    receipts: HashMap<ReceiptHandle, SequenceNumber>,
    /// Deduplication ids outlive their messages, so they are kept apart.
    accepted: HashMap<String, Accepted>,
    added: u64,
}

impl Queue {
    pub(crate) fn new(settings: Settings, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            settings,
            dead_letter,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            receipts: HashMap::new(),
            accepted: HashMap::new(),
            added: 0,
        }
    }

    pub(crate) fn settings(&self) -> Settings {
        self.settings
    }

    pub(crate) fn dead_letter(&self) -> Option<&DeadLetter> {
        self.dead_letter.as_ref()
    }

    pub(crate) fn configure(&mut self, settings: Settings, dead_letter: Option<DeadLetter>) {
        self.settings = settings;
        self.dead_letter = dead_letter;
    }

    pub(crate) fn forget_dead_letter(&mut self, queue: &str) {
        if self
            .dead_letter
            .as_ref()
            .is_some_and(|d| d.queue.as_str() == queue)
        {
            self.dead_letter = None;
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.messages.len()
    }

    pub(crate) fn attributes(&self) -> Attributes {
        let (fifo, content_based_deduplication) = match self.settings.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Attributes {
            visibility_timeout: self.settings.visibility_timeout,
            delay: self.settings.delay,
            retention: self.settings.retention,
            fifo,
            content_based_deduplication,
            dead_letter: self.dead_letter.clone(),
        }
    }

    pub(crate) fn counts(&self, now: Duration) -> Counts {
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

    /// Removes the messages whose retention has run out at `now`. Every
    /// command that looks at a queue calls this first, which is what keeps
    /// retention exact without a timer.
    pub(crate) fn expire(&mut self, now: Duration) {
        while let Some(&(sent_at, sequence)) = self.by_age.first() {
            if now < sent_at + self.settings.retention {
                break;
            }
            self.remove(sequence);
        }
    }

    /// Adds `message` at the end of the queue.
    pub(crate) fn add(&mut self, message: Message) -> SequenceNumber {
        self.added += 1;
        let sequence = SequenceNumber(self.added);
        self.by_age.insert((message.sent_at, sequence));
        self.messages.insert(sequence, message);
        sequence
    }

    fn remove(&mut self, sequence: SequenceNumber) -> Option<Message> {
        let message = self.messages.remove(&sequence)?;
        self.by_age.remove(&(message.sent_at, sequence));
        if let Phase::Delivered { receipt, .. } = message.phase {
            self.receipts.remove(&receipt);
        }
        Some(message)
    }

    /// Removes every message; sequence numbers and deduplication carry on.
    pub(crate) fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.receipts.clear();
    }

    /// The earlier send with this deduplication id, if its window is open.
    pub(crate) fn duplicate_of(&self, id: &str, now: Duration) -> Option<Accepted> {
        self.accepted
            .get(id)
            .filter(|accepted| now < accepted.at + DEDUPLICATION_WINDOW)
            .copied()
    }

    pub(crate) fn record_accepted(
        &mut self,
        id: String,
        message_id: MessageId,
        sequence_number: SequenceNumber,
        now: Duration,
    ) {
        self.accepted.insert(
            id,
            Accepted {
                message_id,
                sequence_number,
                at: now,
            },
        );
    }

    /// The message whose latest handle is `receipt`, if it is in flight.
    pub(crate) fn in_flight(
        &self,
        receipt: ReceiptHandle,
        now: Duration,
    ) -> Option<SequenceNumber> {
        let sequence = *self.receipts.get(&receipt)?;
        let message = self.messages.get(&sequence)?;
        (message.state(now) == State::InFlight).then_some(sequence)
    }

    pub(crate) fn delete(&mut self, sequence: SequenceNumber) {
        self.remove(sequence);
    }

    /// Moves the end of the visibility timeout of an in-flight message.
    pub(crate) fn hide_until(&mut self, sequence: SequenceNumber, deadline: Duration) {
        let message = self
            .messages
            .get_mut(&sequence)
            .expect("the sequence number was just found by in_flight");
        let Phase::Delivered { until, .. } = &mut message.phase else {
            unreachable!("in_flight only returns delivered messages");
        };
        *until = deadline;
    }

    /// Delivers up to `max` messages and takes out those that were received
    /// too often.
    pub(crate) fn receive(
        &mut self,
        now: Duration,
        max: usize,
        visibility_timeout: Option<Duration>,
        ids: &mut Ids,
    ) -> Received {
        let timeout = visibility_timeout.unwrap_or(self.settings.visibility_timeout);
        let mut received = Received::default();
        for step in self.plan_receive(now, max) {
            match step {
                Step::DeadLetter(sequence) => {
                    let mut message = self
                        .remove(sequence)
                        .expect("planned sequence numbers are in the queue");
                    message.reset_for_dead_letter();
                    received.dead.push(message);
                }
                Step::Deliver(sequence) => {
                    received
                        .delivered
                        .push(self.deliver(sequence, now + timeout, now, ids));
                }
            }
        }
        received
    }

    fn plan_receive(&self, now: Duration, max: usize) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut delivering = 0;
        // Groups that already have an earlier message in the queue.
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        for (&sequence, message) in &self.messages {
            if delivering == max {
                break;
            }
            let first_of_group = message
                .group
                .as_ref()
                .is_none_or(|group| blocked.insert(group));
            if !first_of_group || message.state(now) != State::Visible {
                continue;
            }
            match &self.dead_letter {
                Some(dead_letter) if message.receive_count >= dead_letter.max_receive_count => {
                    // The message leaves the queue, so it blocks nobody.
                    if let Some(group) = &message.group {
                        blocked.remove(group);
                    }
                    steps.push(Step::DeadLetter(sequence));
                }
                _ => {
                    steps.push(Step::Deliver(sequence));
                    delivering += 1;
                }
            }
        }
        steps
    }

    fn deliver(
        &mut self,
        sequence: SequenceNumber,
        until: Duration,
        now: Duration,
        ids: &mut Ids,
    ) -> ReceivedMessage {
        let message = self
            .messages
            .get_mut(&sequence)
            .expect("planned sequence numbers are in the queue");
        if let Phase::Delivered { receipt, .. } = message.phase {
            self.receipts.remove(&receipt);
        }
        let receipt = ids.next_receipt();
        message.phase = Phase::Delivered { until, receipt };
        self.receipts.insert(receipt, sequence);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        ReceivedMessage {
            message_id: message.id,
            receipt_handle: receipt,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group.clone(),
            sequence_number: message.group.as_ref().map(|_| sequence),
        }
    }
}
