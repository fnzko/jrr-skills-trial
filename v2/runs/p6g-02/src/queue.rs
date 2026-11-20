use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::Config;
use crate::ids::{IdGenerator, MessageId, ReceiptHandle};
use crate::response::{Counts, Delivery};

/// Most messages one queue may hold, in any state.
pub const MAX_MESSAGES: usize = 100_000;

/// A deduplication id blocks later sends for this many seconds.
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

/// Why a message is currently hidden, if it is. An entry whose `until` has
/// passed no longer hides the message.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Free,
    Delayed { until: u64 },
    InFlight { until: u64, receipt: ReceiptHandle },
}

impl Hold {
    fn state(self, now: u64) -> State {
        match self {
            Self::Delayed { until } if now < until => State::Delayed,
            Self::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

#[derive(Debug)]
struct FifoSlot {
    group_id: String,
    sequence: u64,
}

/// A message held by a queue.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    fifo: Option<FifoSlot>,
    hold: Hold,
}

/// What `SendMessage` hands to a FIFO queue besides the body.
#[derive(Debug)]
pub struct FifoSend {
    pub group_id: String,
    pub deduplication_id: String,
}

/// A message that passed every check of `SendMessage`.
#[derive(Debug)]
pub struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub sent_at: u64,
    pub delay: u32,
    pub fifo: Option<FifoSend>,
}

/// Result of one examination pass over a queue.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug)]
struct Accepted {
    at: u64,
    message: MessageId,
    sequence: u64,
}

#[derive(Debug)]
pub struct Queue {
    config: Config,
    /// Keyed by arrival number, which only grows, so iteration is arrival order.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    last_sequence: u64,
    deduplication: HashMap<String, Accepted>,
    /// Handle to arrival key. May hold stale entries; a lookup is only
    /// trusted after the message confirms it still carries that handle.
    receipts: HashMap<ReceiptHandle, u64>,
}

impl Queue {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            next_arrival: 0,
            last_sequence: 0,
            deduplication: HashMap::new(),
            receipts: HashMap::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replaces the attributes. Messages older than a lowered retention go
    /// at once.
    pub fn set_config(&mut self, config: Config, now: u64) {
        self.config = config;
        self.expire(now);
    }

    pub fn clear_dead_letter(&mut self) {
        self.config.dead_letter = None;
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts {
            visible: 0,
            in_flight: 0,
            delayed: 0,
        };
        for message in self.messages.values() {
            match message.hold.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Removes every message whose retention has run out. Callers invoke it
    /// whenever the clock or the retention changes, which keeps the rule true
    /// at every moment.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.config.retention_seconds);
        self.messages
            .retain(|_, message| now < message.sent_at + retention);
        let messages = &self.messages;
        self.receipts.retain(|_, key| messages.contains_key(key));
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.receipts.clear();
    }

    /// The original message of a send with this deduplication id, if one was
    /// accepted less than the window ago.
    pub fn duplicate_of(&self, now: u64, deduplication_id: &str) -> Option<(MessageId, u64)> {
        let accepted = self.deduplication.get(deduplication_id)?;
        (now - accepted.at < DEDUPLICATION_WINDOW).then_some((accepted.message, accepted.sequence))
    }

    /// Appends the message. Returns its sequence number on a FIFO queue.
    pub fn enqueue(&mut self, new: NewMessage) -> Option<u64> {
        let hold = if new.delay > 0 {
            Hold::Delayed {
                until: new.sent_at + u64::from(new.delay),
            }
        } else {
            Hold::Free
        };
        let fifo = new.fifo.map(|send| {
            self.last_sequence += 1;
            self.deduplication.insert(
                send.deduplication_id,
                Accepted {
                    at: new.sent_at,
                    message: new.id,
                    sequence: self.last_sequence,
                },
            );
            FifoSlot {
                group_id: send.group_id,
                sequence: self.last_sequence,
            }
        });
        let sequence = fifo.as_ref().map(|slot| slot.sequence);
        self.push(Message {
            id: new.id,
            body: new.body,
            sent_at: new.sent_at,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold,
        });
        sequence
    }

    /// Takes in a message moved from another queue, reset as the spec says.
    /// It is dropped on arrival when it is already past this queue's retention.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::Free;
        if let Some(slot) = &mut message.fifo {
            self.last_sequence += 1;
            slot.sequence = self.last_sequence;
        }
        if now < message.sent_at + u64::from(self.config.retention_seconds) {
            self.push(message);
        }
    }

    fn push(&mut self, message: Message) {
        self.messages.insert(self.next_arrival, message);
        self.next_arrival += 1;
    }

    /// Examines the messages once each in arrival order and delivers up to
    /// `max` of them. Messages past their receive limit are returned for the
    /// caller to move to the dead-letter queue.
    pub fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility_timeout: Option<u32>,
        ids: &mut IdGenerator,
    ) -> Received {
        let visibility = u64::from(visibility_timeout.unwrap_or(self.config.visibility_timeout));
        let mut delivered = Vec::new();
        let mut dead_lettered = Vec::new();
        // Groups that still have an earlier message in the queue.
        let mut occupied_groups: HashSet<String> = HashSet::new();
        let mut cursor = 0;
        while delivered.len() < max {
            let Some((&key, message)) = self.messages.range_mut(cursor..).next() else {
                break;
            };
            cursor = key + 1;
            let group = message.fifo.as_ref().map(|slot| slot.group_id.as_str());
            let eligible = message.hold.state(now) == State::Visible
                && group.is_none_or(|group| !occupied_groups.contains(group));
            if eligible
                && let Some(dead_letter) = &self.config.dead_letter
                && message.receive_count >= dead_letter.max_receive_count
            {
                let dead = self
                    .messages
                    .remove(&key)
                    .expect("the key was returned by the range just above");
                dead_lettered.push(dead);
                continue;
            }
            if let Some(group) = group
                && !occupied_groups.contains(group)
            {
                occupied_groups.insert(group.to_owned());
            }
            if !eligible {
                continue;
            }
            if let Hold::InFlight { receipt, .. } = message.hold {
                self.receipts.remove(&receipt);
            }
            let receipt = ids.next_receipt();
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.hold = Hold::InFlight {
                until: now + visibility,
                receipt,
            };
            self.receipts.insert(receipt, key);
            delivered.push(Delivery {
                message_id: message.id,
                receipt_handle: receipt,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.fifo.as_ref().map(|slot| slot.group_id.clone()),
                sequence_number: message.fifo.as_ref().map(|slot| slot.sequence),
            });
        }
        Received {
            delivered,
            dead_lettered,
        }
    }

    /// Arrival key of the message that is in flight under exactly this handle.
    fn in_flight_key(&self, now: u64, handle: ReceiptHandle) -> Option<u64> {
        let key = *self.receipts.get(&handle)?;
        let message = self.messages.get(&key)?;
        matches!(
            message.hold,
            Hold::InFlight { until, receipt } if receipt == handle && now < until
        )
        .then_some(key)
    }

    /// Removes the message in flight under `handle`; false if there is none.
    pub fn delete(&mut self, now: u64, handle: ReceiptHandle) -> bool {
        let Some(key) = self.in_flight_key(now, handle) else {
            return false;
        };
        self.messages.remove(&key);
        self.receipts.remove(&handle);
        true
    }

    /// Makes the message in flight under `handle` stay hidden for `timeout`
    /// seconds from now; false if there is none.
    pub fn change_visibility(&mut self, now: u64, handle: ReceiptHandle, timeout: u32) -> bool {
        let Some(key) = self.in_flight_key(now, handle) else {
            return false;
        };
        let message = self
            .messages
            .get_mut(&key)
            .expect("in_flight_key only returns keys of existing messages");
        message.hold = Hold::InFlight {
            until: now + u64::from(timeout),
            receipt: handle,
        };
        true
    }
}
