//! One queue: its configuration, its messages in arrival order and its deduplication record.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::attributes::Config;
use crate::error::{Error, ErrorCode, Result};
use crate::request::NewMessage;

const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
struct Message {
    id: u64,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    /// Set exactly for messages of a FIFO queue.
    group_id: Option<Arc<str>>,
    sequence: Option<u64>,
    delayed_until: u64,
    invisible_until: u64,
    /// The latest receipt handle issued for this message.
    receipt: Option<u64>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.invisible_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What `ReceiveMessage` reports about one delivered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    pub id: u64,
    pub receipt_handle: u64,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<Arc<str>>,
    pub sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub id: u64,
    pub sequence: Option<u64>,
}

/// A message that was moved out of its queue because it was received too often.
#[derive(Debug)]
pub struct DeadLettered(Message);

#[derive(Debug, Clone, Copy)]
struct HandleRecord {
    queue: u64,
    arrival: u64,
}

/// Every receipt handle issued so far, indexed by its number minus one.
#[derive(Debug, Default)]
pub struct HandleLog(Vec<HandleRecord>);

impl HandleLog {
    fn issue(&mut self, queue: u64, arrival: u64) -> u64 {
        self.0.push(HandleRecord { queue, arrival });
        u64::try_from(self.0.len()).expect("handle count fits in u64")
    }

    /// The number and origin of an issued handle. Only the exact text `r-<n>` without leading
    /// zeros or signs is a handle.
    fn lookup(&self, text: &str) -> Option<(u64, HandleRecord)> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        let record = *self.0.get(usize::try_from(number - 1).ok()?)?;
        Some((number, record))
    }
}

#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: u64,
    message_id: u64,
    sequence: u64,
}

#[derive(Debug)]
pub struct Queue {
    /// Distinguishes this queue from an earlier queue of the same name.
    uid: u64,
    pub config: Config,
    /// Keyed by arrival index, which only grows, so iteration is arrival order.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    next_sequence: u64,
    deduplication: HashMap<String, Accepted>,
    /// A lower bound on the `sent_at` of every message, so retention only scans when it can
    /// remove something.
    oldest_sent_at: u64,
}

impl Queue {
    pub fn new(uid: u64, config: Config) -> Self {
        Self {
            uid,
            config,
            messages: BTreeMap::new(),
            next_arrival: 0,
            next_sequence: 1,
            deduplication: HashMap::new(),
            oldest_sent_at: u64::MAX,
        }
    }

    /// Removes the messages whose retention has run out. Every command that looks at a queue
    /// calls this first, with the retention the queue has at that moment.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.config.retention_seconds);
        if self.oldest_sent_at.saturating_add(retention) > now {
            return;
        }
        self.messages.retain(|_, m| now < m.sent_at + retention);
        self.oldest_sent_at = self
            .messages
            .values()
            .map(|m| m.sent_at)
            .min()
            .unwrap_or(u64::MAX);
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for m in self.messages.values() {
            match m.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.oldest_sent_at = u64::MAX;
    }

    pub fn send(&mut self, now: u64, new: NewMessage, next_id: &mut u64) -> Result<Sent> {
        let fifo = self.config.fifo;
        if (!fifo && (new.group_id.is_some() || new.deduplication_id.is_some()))
            || (fifo && new.delay_seconds.is_some())
        {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "GroupId and DeduplicationId need a FIFO queue, DelaySeconds a standard queue",
            ));
        }
        let group_id = if fifo {
            Some(Arc::<str>::from(new.group_id.ok_or_else(|| {
                Error::new(ErrorCode::MissingParameter, "GroupId is required")
            })?))
        } else {
            None
        };
        let mut deduplication_key = None;
        if fifo {
            if new.deduplication_id.is_none() && !self.config.content_based_deduplication {
                return Err(Error::new(
                    ErrorCode::MissingParameter,
                    "DeduplicationId is required",
                ));
            }
            let key = new.deduplication_id.as_deref().unwrap_or(&new.body);
            if let Some(original) = self.deduplication.get(key)
                && now - original.at < DEDUPLICATION_WINDOW
            {
                return Ok(Sent {
                    id: original.message_id,
                    sequence: Some(original.sequence),
                });
            }
            deduplication_key = Some(key.to_owned());
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds 100000 messages",
            ));
        }

        let id = *next_id;
        *next_id += 1;
        let sequence = self.take_sequence();
        if let (Some(key), Some(sequence)) = (deduplication_key, sequence) {
            let accepted = Accepted {
                at: now,
                message_id: id,
                sequence,
            };
            self.deduplication.insert(key, accepted);
        }
        let delay = new.delay_seconds.unwrap_or(self.config.delay_seconds);
        self.push(Message {
            id,
            body: new.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id,
            sequence,
            delayed_until: now + u64::from(delay),
            invisible_until: 0,
            receipt: None,
        });
        Ok(Sent { id, sequence })
    }

    pub fn receive(
        &mut self,
        now: u64,
        max_messages: u32,
        visibility_timeout: Option<u32>,
        handles: &mut HandleLog,
    ) -> (Vec<Delivered>, Vec<DeadLettered>) {
        let visibility = u64::from(visibility_timeout.unwrap_or(self.config.visibility_timeout));
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);

        // Decide first, on shared borrows, then apply: the decisions never depend on the effects
        // of earlier ones except through `seen_groups`.
        let mut plan = Vec::new();
        let mut deliveries = 0;
        let mut seen_groups: HashSet<&str> = HashSet::new();
        for (&arrival, m) in &self.messages {
            if deliveries == max_messages {
                break;
            }
            if let Some(group) = m.group_id.as_deref()
                && !seen_groups.insert(group)
            {
                continue;
            }
            if m.state(now) != State::Visible {
                continue;
            }
            let dead = max_receive_count.is_some_and(|max| m.receive_count >= max);
            if dead {
                // The message leaves the queue, so it no longer holds its group back.
                if let Some(group) = m.group_id.as_deref() {
                    seen_groups.remove(group);
                }
            } else {
                deliveries += 1;
            }
            plan.push((arrival, dead));
        }

        let mut delivered = Vec::new();
        let mut dead_lettered = Vec::new();
        for (arrival, dead) in plan {
            if dead {
                let m = self
                    .messages
                    .remove(&arrival)
                    .expect("planned message is in the queue");
                dead_lettered.push(DeadLettered(m));
                continue;
            }
            let handle = handles.issue(self.uid, arrival);
            let m = self
                .messages
                .get_mut(&arrival)
                .expect("planned message is in the queue");
            m.receive_count += 1;
            let first_received_at = *m.first_received_at.get_or_insert(now);
            m.receipt = Some(handle);
            m.invisible_until = now + visibility;
            delivered.push(Delivered {
                id: m.id,
                receipt_handle: handle,
                body: m.body.clone(),
                receive_count: m.receive_count,
                sent_at: m.sent_at,
                first_received_at,
                group_id: m.group_id.clone(),
                sequence: m.sequence,
            });
        }
        (delivered, dead_lettered)
    }

    /// Appends a message that another queue gave up on, as a fresh and immediately visible one.
    pub fn add_dead_letter(&mut self, now: u64, DeadLettered(mut m): DeadLettered) {
        m.sequence = self.take_sequence();
        if now >= m.sent_at + u64::from(self.config.retention_seconds) {
            return;
        }
        m.receive_count = 0;
        m.first_received_at = None;
        m.delayed_until = 0;
        m.invisible_until = 0;
        m.receipt = None;
        self.push(m);
    }

    pub fn dead_letter_queue(&self) -> Option<&str> {
        self.config.dead_letter.as_ref().map(|d| d.queue.as_str())
    }

    /// Finds the message that `handle` is the latest, still-running delivery of.
    fn in_flight(
        &mut self,
        now: u64,
        handles: &HandleLog,
        handle: &str,
    ) -> Result<(u64, &mut Message)> {
        let (number, record) = handles.lookup(handle).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{handle:?} was never issued"),
            )
        })?;
        let expired = || {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("{handle:?} no longer identifies a message in flight"),
            )
        };
        if record.queue != self.uid {
            return Err(expired());
        }
        self.messages
            .get_mut(&record.arrival)
            .filter(|m| m.receipt == Some(number) && now < m.invisible_until)
            .map(|m| (record.arrival, m))
            .ok_or_else(expired)
    }

    pub fn delete_message(&mut self, now: u64, handles: &HandleLog, handle: &str) -> Result<()> {
        let (arrival, _) = self.in_flight(now, handles, handle)?;
        self.messages.remove(&arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        now: u64,
        handles: &HandleLog,
        handle: &str,
        visibility_timeout: u32,
    ) -> Result<()> {
        let (_, m) = self.in_flight(now, handles, handle)?;
        m.invisible_until = now + u64::from(visibility_timeout);
        Ok(())
    }

    fn take_sequence(&mut self) -> Option<u64> {
        self.config.fifo.then(|| {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            sequence
        })
    }

    fn push(&mut self, m: Message) {
        self.oldest_sent_at = self.oldest_sent_at.min(m.sent_at);
        self.messages.insert(self.next_arrival, m);
        self.next_arrival += 1;
    }
}
