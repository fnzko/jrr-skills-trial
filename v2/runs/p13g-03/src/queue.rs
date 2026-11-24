//! One queue: its messages in arrival order plus the indexes that keep
//! sends, receives and expiry from scanning every message.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::reply::Counts;
use crate::types::{
    DeadLetter, GroupId, MessageId, ReceiptHandle, Seconds, SequenceNumber, Settings,
};

/// A queue never holds more messages than this, in any state.
pub const MAX_MESSAGES: usize = 100_000;

/// A FIFO send with the same deduplication id is a duplicate for this long.
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// Position in a queue's arrival order; never reused within a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Visible,
    Delayed {
        until: Seconds,
    },
    InFlight {
        until: Seconds,
        handle: ReceiptHandle,
    },
}

impl Hold {
    fn until(self) -> Option<Seconds> {
        match self {
            Hold::Visible => None,
            Hold::Delayed { until } | Hold::InFlight { until, .. } => Some(until),
        }
    }
}

#[derive(Debug)]
struct FifoPlacement {
    group: GroupId,
    sequence: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Seconds,
    receive_count: u32,
    first_received_at: Option<Seconds>,
    hold: Hold,
    fifo: Option<FifoPlacement>,
}

/// Where a sent message goes, and what makes it a duplicate.
pub enum Placement<'a> {
    Standard,
    Fifo {
        group: GroupId,
        deduplication_key: &'a str,
    },
}

#[derive(Debug)]
struct Accepted {
    at: Seconds,
    id: MessageId,
    sequence: SequenceNumber,
}

/// What `ReceiveMessage` will do, decided before anything is changed.
#[derive(Debug, Default)]
pub struct ReceivePlan {
    pub dead_letter: Vec<Arrival>,
    pub deliver: Vec<Arrival>,
}

/// A delivered message as it appears in a `ReceiveMessage` reply.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: Seconds,
    first_received_at: Seconds,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

#[derive(Debug)]
pub struct Queue {
    settings: Settings,
    dead_letter: Option<DeadLetter>,
    messages: BTreeMap<Arrival, Message>,
    /// Visible messages.
    ready: BTreeSet<Arrival>,
    /// Delayed and in-flight messages, ordered by the time they turn visible.
    held: BTreeSet<(Seconds, Arrival)>,
    /// FIFO messages of each group, in any state.
    groups: HashMap<GroupId, BTreeSet<Arrival>>,
    /// The queue the handle was issued for knows its message.
    handles: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<String, Accepted>,
    next_arrival: u64,
    last_sequence: u64,
    /// No message can expire before this time; a lower bound, not exact.
    expiry_due: Seconds,
}

impl Queue {
    pub fn new(settings: Settings, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            settings,
            dead_letter,
            messages: BTreeMap::new(),
            ready: BTreeSet::new(),
            held: BTreeSet::new(),
            groups: HashMap::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_arrival: 0,
            last_sequence: 0,
            expiry_due: Seconds(0),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.dead_letter.as_ref()
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Replaces the given settings and, if present, the dead-letter
    /// configuration. A dead-letter configuration is never removed this way.
    pub fn reconfigure(
        &mut self,
        update: &crate::types::AttributeUpdate<'_>,
        dead_letter: Option<DeadLetter>,
    ) {
        self.settings.apply(update);
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
        // The retention period may have shrunk.
        self.expiry_due = Seconds(0);
    }

    pub fn detach_dead_letter(&mut self, deleted: &str) {
        if self
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == deleted)
        {
            self.dead_letter = None;
        }
    }

    /// Brings the queue up to `now`: drops expired messages and makes
    /// messages whose delay or visibility timeout ended visible.
    pub fn advance(&mut self, now: Seconds) {
        if now >= self.expiry_due {
            self.expire(now);
        }
        while let Some(&(until, arrival)) = self.held.first() {
            if until > now {
                break;
            }
            self.set_hold(arrival, Hold::Visible, now);
        }
    }

    fn expire(&mut self, now: Seconds) {
        let retention = self.settings.retention;
        let expired: Vec<Arrival> = self
            .messages
            .iter()
            .filter(|(_, message)| message.sent_at + retention <= now)
            .map(|(arrival, _)| *arrival)
            .collect();
        for arrival in expired {
            self.remove(arrival);
        }
        self.expiry_due = self
            .messages
            .values()
            .map(|message| message.sent_at + retention)
            .min()
            .unwrap_or(Seconds(u64::MAX));
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.ready.clear();
        self.held.clear();
        self.groups.clear();
        self.handles.clear();
    }

    pub fn counts(&self) -> Counts {
        let in_flight = self
            .held
            .iter()
            .filter(|(_, arrival)| matches!(self.messages[arrival].hold, Hold::InFlight { .. }))
            .count();
        Counts {
            visible: self.ready.len(),
            in_flight,
            delayed: self.held.len() - in_flight,
        }
    }

    pub fn find_duplicate(&self, key: &str, now: Seconds) -> Option<(MessageId, SequenceNumber)> {
        self.deduplication
            .get(key)
            .filter(|accepted| now < accepted.at + DEDUPLICATION_WINDOW)
            .map(|accepted| (accepted.id, accepted.sequence))
    }

    /// Adds a message sent now. Returns its sequence number on a FIFO queue.
    pub fn send(
        &mut self,
        id: MessageId,
        body: &str,
        placement: Placement<'_>,
        delay: Seconds,
        now: Seconds,
    ) -> Option<SequenceNumber> {
        let hold = Hold::Delayed { until: now + delay };
        match placement {
            Placement::Standard => {
                self.push(id, body.to_owned(), now, None, hold, now);
                None
            }
            Placement::Fifo {
                group,
                deduplication_key,
            } => {
                let placement = self.place_in_group(group);
                let sequence = placement.sequence;
                self.push(id, body.to_owned(), now, Some(placement), hold, now);
                self.deduplication.insert(
                    deduplication_key.to_owned(),
                    Accepted {
                        at: now,
                        id,
                        sequence,
                    },
                );
                Some(sequence)
            }
        }
    }

    /// Adds a message moved from another queue. It is visible at once, takes
    /// the next sequence number here and records no deduplication id.
    pub fn accept_moved(&mut self, message: Message, now: Seconds) {
        let fifo = message.fifo.map(|fifo| self.place_in_group(fifo.group));
        self.push(
            message.id,
            message.body,
            message.sent_at,
            fifo,
            Hold::Visible,
            now,
        );
    }

    fn place_in_group(&mut self, group: GroupId) -> FifoPlacement {
        self.last_sequence += 1;
        FifoPlacement {
            group,
            sequence: SequenceNumber(self.last_sequence),
        }
    }

    fn push(
        &mut self,
        id: MessageId,
        body: String,
        sent_at: Seconds,
        fifo: Option<FifoPlacement>,
        hold: Hold,
        now: Seconds,
    ) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        if let Some(fifo) = &fifo {
            self.groups
                .entry(fifo.group.clone())
                .or_default()
                .insert(arrival);
        }
        let hold = self.index(arrival, hold, now);
        self.expiry_due = self.expiry_due.min(sent_at + self.settings.retention);
        self.messages.insert(
            arrival,
            Message {
                id,
                body,
                sent_at,
                receive_count: 0,
                first_received_at: None,
                hold,
                fifo,
            },
        );
    }

    /// Files `arrival` under the index that matches `hold`, and returns the
    /// hold to store: one that has already ended is `Visible`.
    fn index(&mut self, arrival: Arrival, hold: Hold, now: Seconds) -> Hold {
        match hold.until() {
            Some(until) if until > now => {
                self.held.insert((until, arrival));
                hold
            }
            _ => {
                self.ready.insert(arrival);
                Hold::Visible
            }
        }
    }

    fn message_mut(&mut self, arrival: Arrival) -> &mut Message {
        self.messages
            .get_mut(&arrival)
            .expect("arrival numbers held by the indexes belong to stored messages")
    }

    fn set_hold(&mut self, arrival: Arrival, hold: Hold, now: Seconds) {
        match self.message_mut(arrival).hold.until() {
            Some(until) => self.held.remove(&(until, arrival)),
            None => self.ready.remove(&arrival),
        };
        let hold = self.index(arrival, hold, now);
        self.message_mut(arrival).hold = hold;
    }

    pub fn remove(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("callers pass arrival numbers of stored messages");
        match message.hold.until() {
            Some(until) => self.held.remove(&(until, arrival)),
            None => self.ready.remove(&arrival),
        };
        if let Some(fifo) = &message.fifo
            && let Some(members) = self.groups.get_mut(&fifo.group)
        {
            members.remove(&arrival);
            if members.is_empty() {
                self.groups.remove(&fifo.group);
            }
        }
        message
    }

    /// Chooses what a `ReceiveMessage` does, scanning visible messages in
    /// arrival order until `max_messages` are delivered.
    pub fn plan_receive(&self, max_messages: usize) -> ReceivePlan {
        let mut plan = ReceivePlan::default();
        let mut moved = HashSet::new();
        for &arrival in &self.ready {
            if plan.deliver.len() == max_messages {
                break;
            }
            let message = &self.messages[&arrival];
            if let Some(fifo) = &message.fifo {
                // A message that is moved away this very call no longer blocks.
                let blocked = self.groups[&fifo.group]
                    .range(..arrival)
                    .any(|earlier| !moved.contains(earlier));
                if blocked {
                    continue;
                }
            }
            match &self.dead_letter {
                Some(dead_letter) if message.receive_count >= dead_letter.max_receive_count => {
                    plan.dead_letter.push(arrival);
                    moved.insert(arrival);
                }
                _ => plan.deliver.push(arrival),
            }
        }
        plan
    }

    pub fn deliver(
        &mut self,
        arrival: Arrival,
        visibility_timeout: Seconds,
        handle: ReceiptHandle,
        now: Seconds,
    ) -> Delivery {
        self.handles.insert(handle, arrival);
        let hold = Hold::InFlight {
            until: now + visibility_timeout,
            handle,
        };
        self.set_hold(arrival, hold, now);
        let message = self.message_mut(arrival);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        Delivery {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.fifo.as_ref().map(|fifo| fifo.group.clone()),
            sequence_number: message.fifo.as_ref().map(|fifo| fifo.sequence),
        }
    }

    /// The message that `handle` is the latest handle of, if it is in flight.
    /// Call `advance` first so ended timeouts no longer count.
    pub fn in_flight(&self, handle: ReceiptHandle) -> Option<Arrival> {
        let arrival = *self.handles.get(&handle)?;
        let message = self.messages.get(&arrival)?;
        matches!(message.hold, Hold::InFlight { handle: latest, .. } if latest == handle)
            .then_some(arrival)
    }

    /// Ends the visibility timeout of an in-flight message at `now + timeout`.
    pub fn change_visibility(
        &mut self,
        arrival: Arrival,
        handle: ReceiptHandle,
        timeout: Seconds,
        now: Seconds,
    ) {
        let hold = Hold::InFlight {
            until: now + timeout,
            handle,
        };
        self.set_hold(arrival, hold, now);
    }
}
