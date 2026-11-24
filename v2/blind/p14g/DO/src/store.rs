//! The messages of one queue, indexed for the operations the protocol needs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::error::{Error, Limit};
use crate::types::{Body, GroupId, Ids, MessageId, ReceiptHandle, Seconds, SequenceNumber, Time};

const MAX_MESSAGES: usize = 100_000;

/// Position of a message in the arrival order of its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FifoInfo {
    pub group: GroupId,
    pub sequence: SequenceNumber,
}

/// Why a message is not visible. A message that was never delivered is held until the end of its
/// delay; a delivered one until the end of its visibility timeout.
#[derive(Debug)]
pub enum Hold {
    Delayed { until: Time },
    Delivered { handle: ReceiptHandle, until: Time },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Visible,
    InFlight,
    Delayed,
}

impl Hold {
    fn state(&self, now: Time) -> State {
        match *self {
            Self::Delayed { until } if now < until => State::Delayed,
            Self::Delivered { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// Number of messages in each state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
pub struct Message {
    pub id: MessageId,
    pub body: Body,
    pub sent_at: Time,
    pub receive_count: u32,
    pub first_received_at: Option<Time>,
    pub fifo: Option<FifoInfo>,
    pub hold: Hold,
}

/// A message as handed to a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: MessageId,
    pub handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    pub fifo: Option<FifoInfo>,
}

/// What one `deliver` pass did: the deliveries, and the messages that reached the receive limit
/// and were taken out of the store for the caller to dead-letter.
#[derive(Debug)]
pub struct Scan {
    pub delivered: Vec<Delivery>,
    pub dead: Vec<Message>,
}

#[derive(Debug, Default)]
pub struct Store {
    messages: BTreeMap<Arrival, Message>,
    next_arrival: u64,
    /// `(sent_at, arrival)` of every message, so retention only looks at the oldest ones.
    by_sent_at: BTreeSet<(Time, Arrival)>,
    /// The latest handle of every delivered message. A handle stays here after its visibility
    /// timeout ends; callers check the message state.
    handles: HashMap<ReceiptHandle, Arrival>,
}

impl Store {
    /// Fails when the queue holds the maximum number of messages, whatever their state.
    pub fn ensure_room(&self) -> Result<(), Error> {
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::LimitExceeded(Limit::Messages));
        }
        Ok(())
    }

    pub fn push(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Hold::Delivered { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    /// Removes every message with `now >= sent_at + retention`.
    pub fn expire(&mut self, now: Time, retention: Seconds) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at.after(retention) {
                break;
            }
            self.remove(arrival);
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.hold.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Examines the messages once each in arrival order until `limit` are delivered.
    ///
    /// In a FIFO queue a message is skipped while an earlier message of its group is still in the
    /// queue; a message that is moved out for dead-lettering no longer blocks its group.
    pub fn deliver(
        &mut self,
        now: Time,
        limit: usize,
        visibility: Seconds,
        max_receive_count: Option<u32>,
        ids: &mut Ids,
    ) -> Scan {
        let mut delivered = Vec::new();
        let mut dead_arrivals = Vec::new();
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &mut self.messages {
            if delivered.len() == limit {
                break;
            }
            let Message {
                id,
                body,
                sent_at,
                receive_count,
                first_received_at,
                fifo,
                hold,
            } = message;
            if let Some(info) = fifo.as_ref()
                && !blocked.insert(&info.group)
            {
                continue;
            }
            if hold.state(now) != State::Visible {
                continue;
            }
            if max_receive_count.is_some_and(|max| *receive_count >= max) {
                dead_arrivals.push(arrival);
                if let Some(info) = fifo.as_ref() {
                    blocked.remove(&info.group);
                }
                continue;
            }
            *receive_count += 1;
            let first_received_at = *first_received_at.get_or_insert(now);
            let handle = ids.next_handle();
            if let Hold::Delivered { handle: old, .. } = hold {
                self.handles.remove(old);
            }
            *hold = Hold::Delivered {
                handle,
                until: now.after(visibility),
            };
            self.handles.insert(handle, arrival);
            delivered.push(Delivery {
                id: *id,
                handle,
                body: body.clone(),
                receive_count: *receive_count,
                sent_at: *sent_at,
                first_received_at,
                fifo: fifo.clone(),
            });
        }
        let dead = dead_arrivals
            .into_iter()
            .map(|arrival| {
                self.remove(arrival)
                    .expect("dead arrivals were taken from the messages just scanned")
            })
            .collect();
        Scan { delivered, dead }
    }

    fn in_flight_arrival(&self, handle: ReceiptHandle, now: Time) -> Option<Arrival> {
        let arrival = *self.handles.get(&handle)?;
        let message = self
            .messages
            .get(&arrival)
            .expect("the handle index only points at stored messages");
        (message.hold.state(now) == State::InFlight).then_some(arrival)
    }

    /// Removes the message whose latest handle is `handle`, if it is in flight.
    pub fn take_in_flight(&mut self, handle: ReceiptHandle, now: Time) -> Option<Message> {
        let arrival = self.in_flight_arrival(handle, now)?;
        self.remove(arrival)
    }

    /// Restarts the visibility timeout of the in-flight message with this handle.
    pub fn restart_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: Seconds,
    ) -> Option<()> {
        let arrival = self.in_flight_arrival(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("in_flight_arrival returned a stored message");
        message.hold = Hold::Delivered {
            handle,
            until: now.after(timeout),
        };
        Some(())
    }
}
