use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::Attributes;
use crate::ids::{MessageId, ReceiptHandle};

/// Virtual seconds since start.
pub(crate) type Time = u64;

/// A FIFO-queue send with the same deduplication id is a duplicate for this long.
const DEDUPLICATION_WINDOW: Time = 300;

/// What a message has only on a FIFO queue.
#[derive(Debug, Clone)]
pub(crate) struct FifoPlacement {
    pub group: String,
    pub sequence: u64,
}

#[derive(Debug)]
pub(crate) struct Message {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Time,
    /// The message is delayed while `now` is before this.
    pub available_at: Time,
    /// The message is in flight while `now` is before this.
    pub invisible_until: Time,
    pub receive_count: u32,
    pub first_received_at: Option<Time>,
    /// The handle of the latest delivery.
    pub handle: Option<ReceiptHandle>,
    pub fifo: Option<FifoPlacement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

impl Message {
    pub fn state(&self, now: Time) -> MessageState {
        if now < self.available_at {
            MessageState::Delayed
        } else if now < self.invisible_until {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }
}

/// A message about to enter a queue; the queue completes it.
#[derive(Debug)]
pub(crate) struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Time,
    pub available_at: Time,
}

/// The receipt handle does not belong to a message that is in flight here.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HandleExpired;

/// A message as handed to a consumer.
#[derive(Debug)]
pub(crate) struct Delivery {
    pub id: MessageId,
    pub handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    pub fifo: Option<FifoPlacement>,
}

/// The outcome of one receive: what was delivered, and what has to move to
/// the dead-letter queue.
#[derive(Debug)]
pub(crate) struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A FIFO send that was accepted, kept to recognize duplicates of it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Accepted {
    pub at: Time,
    pub id: MessageId,
    pub sequence: u64,
}

#[derive(Debug)]
pub(crate) struct Queue {
    pub attributes: Attributes,
    /// Messages in arrival order, keyed by a per-queue arrival counter.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, arrival)` of every message, so that retention only looks at
    /// the oldest ones.
    by_age: BTreeSet<(Time, u64)>,
    arrivals: u64,
    sequences: u64,
    /// Deduplication id to its latest accepted send. Entries outlive their
    /// messages, and purging does not clear them.
    deduplication: HashMap<String, Accepted>,
    /// Latest handle of each message that has been delivered, to its arrival.
    handles: HashMap<ReceiptHandle, u64>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            arrivals: 0,
            sequences: 0,
            deduplication: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    /// Removes the messages whose retention has run out. Callers do this before
    /// anything else touches the queue, so that a changed `RetentionSeconds`
    /// never revives a message.
    pub fn expire(&mut self, now: Time) {
        let retention = Time::from(self.attributes.retention_seconds);
        while let Some(&(sent_at, arrival)) = self.by_age.first()
            && sent_at + retention <= now
        {
            self.take(arrival);
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                MessageState::Visible => counts.visible += 1,
                MessageState::InFlight => counts.in_flight += 1,
                MessageState::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn push_standard(&mut self, new: NewMessage) {
        self.insert(new, None);
    }

    /// Adds a message to a FIFO queue and returns its sequence number.
    pub fn push_fifo(&mut self, new: NewMessage, group: String) -> u64 {
        self.sequences += 1;
        let sequence = self.sequences;
        self.insert(new, Some(FifoPlacement { group, sequence }));
        sequence
    }

    fn insert(&mut self, new: NewMessage, fifo: Option<FifoPlacement>) {
        self.arrivals += 1;
        self.by_age.insert((new.sent_at, self.arrivals));
        self.messages.insert(
            self.arrivals,
            Message {
                id: new.id,
                body: new.body,
                sent_at: new.sent_at,
                available_at: new.available_at,
                invisible_until: 0,
                receive_count: 0,
                first_received_at: None,
                handle: None,
                fifo,
            },
        );
    }

    /// The accepted send that `deduplication_id` is a duplicate of at `now`.
    pub fn duplicate_of(&self, deduplication_id: &str, now: Time) -> Option<Accepted> {
        self.deduplication
            .get(deduplication_id)
            .filter(|accepted| now - accepted.at < DEDUPLICATION_WINDOW)
            .copied()
    }

    pub fn remember(&mut self, deduplication_id: String, accepted: Accepted) {
        self.deduplication.insert(deduplication_id, accepted);
    }

    /// Delivers up to `max_messages` messages and takes out those that reached
    /// the dead-letter limit. `deliveries` counts receipt handles over all
    /// queues.
    pub fn receive(
        &mut self,
        now: Time,
        max_messages: usize,
        visibility_timeout: u32,
        deliveries: &mut u64,
    ) -> Received {
        let (to_deliver, to_dead_letter) = self.examine(now, max_messages);

        let invisible_until = now + Time::from(visibility_timeout);
        let delivered = to_deliver
            .into_iter()
            .map(|arrival| {
                let message = self
                    .messages
                    .get_mut(&arrival)
                    .expect("examined messages are still in the queue");
                *deliveries += 1;
                let handle = ReceiptHandle(*deliveries);
                if let Some(previous) = message.handle {
                    self.handles.remove(&previous);
                }
                self.handles.insert(handle, arrival);
                message.receive_count += 1;
                message.invisible_until = invisible_until;
                message.handle = Some(handle);
                Delivery {
                    id: message.id,
                    handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at: *message.first_received_at.get_or_insert(now),
                    fifo: message.fifo.clone(),
                }
            })
            .collect();
        let dead_lettered = to_dead_letter
            .into_iter()
            .map(|arrival| self.remove(arrival))
            .collect();
        Received {
            delivered,
            dead_lettered,
        }
    }

    /// Walks the queue in arrival order and returns the arrivals to deliver and
    /// to dead-letter, without changing anything.
    fn examine(&self, now: Time, max_messages: usize) -> (Vec<u64>, Vec<u64>) {
        let receive_limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        // Groups that have a message in the queue ahead of the current one.
        let mut groups_ahead: HashSet<&str> = HashSet::new();
        let mut to_deliver = Vec::new();
        let mut to_dead_letter = Vec::new();

        for (&arrival, message) in &self.messages {
            if to_deliver.len() == max_messages {
                break;
            }
            let group = message.fifo.as_ref().map(|fifo| fifo.group.as_str());
            let eligible = message.state(now) == MessageState::Visible
                && !group.is_some_and(|group| groups_ahead.contains(group));
            let leaves_queue =
                eligible && receive_limit.is_some_and(|max| message.receive_count >= max);

            if leaves_queue {
                to_dead_letter.push(arrival);
                continue;
            }
            if eligible {
                to_deliver.push(arrival);
            }
            groups_ahead.extend(group);
        }
        (to_deliver, to_dead_letter)
    }

    /// Adds a message taken out of another queue. It starts over: no receives,
    /// no delay, and a new sequence number on a FIFO queue.
    pub fn push_dead_letter(&mut self, message: Message) {
        let new = NewMessage {
            id: message.id,
            body: message.body,
            sent_at: message.sent_at,
            available_at: message.sent_at,
        };
        match message.fifo {
            Some(fifo) => {
                self.push_fifo(new, fifo.group);
            }
            None => self.push_standard(new),
        }
    }

    /// Removes a message that is known to be in the queue.
    fn remove(&mut self, arrival: u64) -> Message {
        self.take(arrival)
            .expect("examined messages are still in the queue")
    }

    fn take(&mut self, arrival: u64) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// Deletes the message that `handle` is the latest handle of, if it is in
    /// flight.
    pub fn delete(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), HandleExpired> {
        let arrival = self.in_flight(handle, now)?;
        self.take(arrival);
        Ok(())
    }

    /// Ends the visibility timeout of the message that `handle` is the latest
    /// handle of `timeout` seconds from `now`, if it is in flight.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: u32,
    ) -> Result<(), HandleExpired> {
        let arrival = self.in_flight(handle, now)?;
        if let Some(message) = self.messages.get_mut(&arrival) {
            message.invisible_until = now + Time::from(timeout);
        }
        Ok(())
    }

    fn in_flight(&self, handle: ReceiptHandle, now: Time) -> Result<u64, HandleExpired> {
        let arrival = *self.handles.get(&handle).ok_or(HandleExpired)?;
        let message = self.messages.get(&arrival).ok_or(HandleExpired)?;
        if message.handle == Some(handle) && message.state(now) == MessageState::InFlight {
            Ok(arrival)
        } else {
            Err(HandleExpired)
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }
}
