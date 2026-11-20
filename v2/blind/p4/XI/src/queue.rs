//! One queue: its attributes, its messages in arrival order and its
//! deduplication window. Everything that spans queues lives in the broker.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::error::{CommandError, ErrorCode};
use crate::request::SendMessage;
use crate::types::{Counter, GroupId, MessageId, QueueName, ReceiptHandle};

/// Most messages one queue may hold, counting every state.
pub const MESSAGE_LIMIT: usize = 100_000;

/// Seconds during which a FIFO deduplication id rejects repeats.
const DEDUPLICATION_WINDOW: u64 = 300;

/// The attributes of a queue, as `GetQueueAttributes` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueConfig {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

/// Where messages go once they were received `max_receive_count` times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The outcome of an accepted or duplicate send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    /// Set on FIFO queues only.
    pub sequence_number: Option<u64>,
}

/// A delivered message as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(flatten)]
    pub fifo: Option<FifoTag>,
}

/// What a FIFO message carries on top of a standard one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoTag {
    pub group_id: GroupId,
    pub sequence_number: u64,
}

/// What one `ReceiveMessage` did to a queue.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    /// Messages that reached the receive limit, removed from this queue and
    /// due at the end of its dead-letter queue, in examination order.
    pub dead_lettered: Vec<Message>,
}

/// A message that left one queue and has not yet joined another.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    fifo: Option<FifoTag>,
    hold: Hold,
}

/// What may keep a message from being visible. Whether it still does depends
/// on the clock, so the state is always derived from `until` and `now`.
#[derive(Debug, Clone, Copy)]
enum Hold {
    None,
    Delayed { until: u64 },
    InFlight { until: u64, receipt: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

impl Message {
    fn state(&self, now: u64) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            Hold::None | Hold::Delayed { .. } | Hold::InFlight { .. } => State::Visible,
        }
    }
}

/// The first acceptance of a FIFO deduplication id within its window.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: u64,
    sent: Sent,
}

#[derive(Debug)]
pub struct Queue {
    pub config: QueueConfig,
    /// Keyed by a per-queue arrival number, so iteration is arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, arrival)` of every message. All messages of a queue share
    /// one retention period, so the first entry always expires first, even
    /// after `RetentionSeconds` changes or a dead-letter move appends an old
    /// message.
    by_sent_at: BTreeSet<(u64, u64)>,
    arrivals: Counter,
    sequence_numbers: Counter,
    /// The latest receipt handle of each message still in the queue that was
    /// ever delivered, mapped to its arrival number.
    receipts: HashMap<ReceiptHandle, u64>,
    /// Never pruned: ids outlive their messages, and the window is checked
    /// against the stored time on lookup.
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            arrivals: Counter::default(),
            sequence_numbers: Counter::default(),
            receipts: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    /// Removes every message whose retention period has ended.
    ///
    /// Retention holds at every moment, so callers run this before anything
    /// that reads or changes the messages.
    pub fn expire(&mut self, now: u64) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at + self.config.retention_seconds {
                break;
            }
            self.remove(arrival);
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.receipts.clear();
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

    /// Applies the `SendMessage` rules that depend on the queue.
    ///
    /// `message_ids` advances only when the message is accepted.
    pub fn send(
        &mut self,
        request: SendMessage,
        now: u64,
        message_ids: &mut Counter,
    ) -> Result<Sent, CommandError> {
        if self.config.fifo {
            self.send_fifo(request, now, message_ids)
        } else {
            self.send_standard(request, now, message_ids)
        }
    }

    fn send_standard(
        &mut self,
        request: SendMessage,
        now: u64,
        message_ids: &mut Counter,
    ) -> Result<Sent, CommandError> {
        if request.group_id.is_some() || request.deduplication_id.is_some() {
            return Err(CommandError::new(
                ErrorCode::UnsupportedOperation,
                "GroupId and DeduplicationId need a FIFO queue",
            ));
        }
        self.check_capacity()?;
        let delay = request.delay_seconds.unwrap_or(self.config.delay_seconds);
        let message_id = MessageId(message_ids.issue());
        self.insert(Message {
            id: message_id,
            body: request.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo: None,
            hold: Hold::Delayed { until: now + delay },
        });
        Ok(Sent {
            message_id,
            sequence_number: None,
        })
    }

    fn send_fifo(
        &mut self,
        request: SendMessage,
        now: u64,
        message_ids: &mut Counter,
    ) -> Result<Sent, CommandError> {
        if request.delay_seconds.is_some() {
            return Err(CommandError::new(
                ErrorCode::UnsupportedOperation,
                "a FIFO queue takes no per-message DelaySeconds",
            ));
        }
        let Some(group_id) = request.group_id else {
            return Err(CommandError::new(
                ErrorCode::MissingParameter,
                "a FIFO queue needs a GroupId",
            ));
        };
        let deduplication_id = match request.deduplication_id {
            Some(id) => id,
            None if self.config.content_based_deduplication => request.body.clone(),
            None => {
                return Err(CommandError::new(
                    ErrorCode::MissingParameter,
                    "this FIFO queue needs a DeduplicationId",
                ));
            }
        };
        if let Some(accepted) = self.deduplication.get(&deduplication_id)
            && now - accepted.at < DEDUPLICATION_WINDOW
        {
            return Ok(accepted.sent);
        }
        self.check_capacity()?;
        let sent = Sent {
            message_id: MessageId(message_ids.issue()),
            sequence_number: Some(self.sequence_numbers.issue()),
        };
        self.deduplication
            .insert(deduplication_id, Accepted { at: now, sent });
        self.insert(Message {
            id: sent.message_id,
            body: request.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo: sent.sequence_number.map(|sequence_number| FifoTag {
                group_id,
                sequence_number,
            }),
            hold: Hold::Delayed {
                until: now + self.config.delay_seconds,
            },
        });
        Ok(sent)
    }

    fn check_capacity(&self) -> Result<(), CommandError> {
        if self.messages.len() >= MESSAGE_LIMIT {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MESSAGE_LIMIT} messages"),
            ));
        }
        Ok(())
    }

    /// Examines the messages in arrival order until `max_messages` were
    /// delivered, delivering eligible ones and taking out those that reached
    /// the receive limit.
    ///
    /// `receipt_handles` advances once per delivery, in delivery order.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: u64,
        receipt_handles: &mut Counter,
    ) -> Received {
        let (to_deliver, to_dead_letter) = self.select(now, max_messages);
        let dead_lettered = to_dead_letter
            .into_iter()
            .map(|arrival| self.remove(arrival))
            .collect();
        let delivered = to_deliver
            .into_iter()
            .map(|arrival| {
                let receipt = ReceiptHandle(receipt_handles.issue());
                self.deliver(arrival, receipt, now + visibility_timeout, now)
            })
            .collect();
        Received {
            delivered,
            dead_lettered,
        }
    }

    /// Picks the arrival numbers to deliver and to dead-letter, without
    /// changing anything.
    fn select(&self, now: u64, max_messages: usize) -> (Vec<u64>, Vec<u64>) {
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut to_deliver = Vec::new();
        let mut to_dead_letter = Vec::new();
        // Groups with a message still in the queue ahead of the current one.
        let mut blocked_groups: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &self.messages {
            if to_deliver.len() == max_messages {
                break;
            }
            let group = message.fifo.as_ref().map(|tag| &tag.group_id);
            let eligible = message.state(now) == State::Visible
                && group.is_none_or(|g| !blocked_groups.contains(g));
            if eligible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                // The message leaves the queue, so it does not block its group.
                to_dead_letter.push(arrival);
                continue;
            }
            if let Some(group) = group {
                blocked_groups.insert(group);
            }
            if eligible {
                to_deliver.push(arrival);
            }
        }
        (to_deliver, to_dead_letter)
    }

    fn deliver(&mut self, arrival: u64, receipt: ReceiptHandle, until: u64, now: u64) -> Delivery {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("select only returns arrivals of messages in the queue");
        if let Hold::InFlight { receipt: old, .. } = message.hold {
            self.receipts.remove(&old);
        }
        self.receipts.insert(receipt, arrival);
        message.hold = Hold::InFlight { until, receipt };
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        Delivery {
            message_id: message.id,
            receipt_handle: receipt,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            fifo: message.fifo.clone(),
        }
    }

    /// Appends a message taken from the queue whose dead-letter queue this is.
    ///
    /// The move ignores deduplication and the message limit, and a message
    /// already past this queue's retention is removed at once.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::None;
        message.fifo = message.fifo.map(|tag| FifoTag {
            group_id: tag.group_id,
            sequence_number: self.sequence_numbers.issue(),
        });
        self.insert(message);
        self.expire(now);
    }

    pub fn delete_message(&mut self, receipt: ReceiptHandle, now: u64) -> Result<(), CommandError> {
        let arrival = self.in_flight(receipt, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        receipt: ReceiptHandle,
        now: u64,
        visibility_timeout: u64,
    ) -> Result<(), CommandError> {
        let arrival = self.in_flight(receipt, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("in_flight only returns arrivals of messages in the queue");
        message.hold = Hold::InFlight {
            until: now + visibility_timeout,
            receipt,
        };
        Ok(())
    }

    /// The arrival number of the message whose latest handle is `receipt`, if
    /// that message is in flight now.
    fn in_flight(&self, receipt: ReceiptHandle, now: u64) -> Result<u64, CommandError> {
        let expired = || {
            CommandError::new(
                ErrorCode::ReceiptHandleExpired,
                format!("{receipt} does not belong to a message in flight in this queue"),
            )
        };
        let &arrival = self.receipts.get(&receipt).ok_or_else(expired)?;
        let message = self
            .messages
            .get(&arrival)
            .expect("receipts only map to messages in the queue");
        match message.state(now) {
            State::InFlight => Ok(arrival),
            State::Visible | State::Delayed => Err(expired()),
        }
    }

    fn insert(&mut self, message: Message) {
        let arrival = self.arrivals.issue();
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: u64) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrival numbers are only looked up for messages in the queue");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Hold::InFlight { receipt, .. } = message.hold {
            self.receipts.remove(&receipt);
        }
        message
    }
}
