//! One queue: its attributes, its type-specific state and its messages.

use std::collections::HashMap;

use crate::error::{Error, Unsupported};
use crate::request::{Attributes, DeadLetterRequest, SendParams};
use crate::store::{Counts, Delivery, FifoInfo, Hold, Message, Store};
use crate::types::{
    Body, DeduplicationId, Ids, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber, Time,
};

const DEDUPLICATION_WINDOW: Seconds = Seconds::new(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub dead_letter: Option<DeadLetter>,
}

impl Settings {
    const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
    const DEFAULT_DELAY: Seconds = Seconds::new(0);
    const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

    pub fn from_request(attributes: &Attributes, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(Self::DEFAULT_VISIBILITY_TIMEOUT),
            delay: attributes.delay.unwrap_or(Self::DEFAULT_DELAY),
            retention: attributes.retention.unwrap_or(Self::DEFAULT_RETENTION),
            dead_letter,
        }
    }
}

#[derive(Debug)]
struct Accepted {
    at: Time,
    id: MessageId,
    sequence: SequenceNumber,
}

/// The state only a FIFO queue has. Deduplication entries outlive their messages.
#[derive(Debug)]
pub struct Fifo {
    content_based_deduplication: bool,
    next_sequence: SequenceNumber,
    accepted: HashMap<String, Accepted>,
}

impl Fifo {
    fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next();
        sequence
    }

    fn duplicate_of(&self, key: &str, now: Time) -> Option<Sent> {
        self.accepted
            .get(key)
            .filter(|original| now < original.at.after(DEDUPLICATION_WINDOW))
            .map(|original| Sent {
                id: original.id,
                sequence: Some(original.sequence),
            })
    }
}

#[derive(Debug)]
pub enum Kind {
    Standard,
    Fifo(Fifo),
}

impl Kind {
    pub fn new(fifo: bool, content_based_deduplication: bool) -> Self {
        if fifo {
            Self::Fifo(Fifo {
                content_based_deduplication,
                next_sequence: SequenceNumber::FIRST,
                accepted: HashMap::new(),
            })
        } else {
            Self::Standard
        }
    }

    fn is_fifo(&self) -> bool {
        matches!(self, Self::Fifo(_))
    }

    fn content_based_deduplication(&self) -> bool {
        matches!(
            self,
            Self::Fifo(Fifo {
                content_based_deduplication: true,
                ..
            })
        )
    }
}

/// Result of an accepted (or duplicate) send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

/// Messages that reached the receive limit, to be added to `to`.
#[derive(Debug)]
pub struct DeadLettered {
    pub to: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_lettered: Option<DeadLettered>,
}

#[derive(Debug)]
pub struct Queue {
    pub settings: Settings,
    kind: Kind,
    store: Store,
}

impl Queue {
    pub fn new(settings: Settings, kind: Kind) -> Self {
        Self {
            settings,
            kind,
            store: Store::default(),
        }
    }

    pub fn is_fifo(&self) -> bool {
        self.kind.is_fifo()
    }

    pub fn content_based_deduplication(&self) -> bool {
        self.kind.content_based_deduplication()
    }

    /// Whether a `CreateQueue` request asks for exactly this queue's current attributes.
    pub fn matches(
        &self,
        requested: &Attributes,
        dead_letter: Option<&DeadLetterRequest<'_>>,
    ) -> bool {
        let asked_dead_letter = dead_letter.map(|d| (d.queue, d.max_receive_count));
        let own_dead_letter = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        let asked = Settings::from_request(requested, None);
        self.settings.visibility_timeout == asked.visibility_timeout
            && self.settings.delay == asked.delay
            && self.settings.retention == asked.retention
            && own_dead_letter == asked_dead_letter
            && self.is_fifo() == requested.fifo.unwrap_or(false)
            && self.content_based_deduplication()
                == requested.content_based_deduplication.unwrap_or(false)
    }

    /// Applies `SetQueueAttributes`; every rule has been checked by the caller.
    pub fn apply(&mut self, attributes: &Attributes, dead_letter: Option<DeadLetter>, now: Time) {
        let settings = &mut self.settings;
        if let Some(value) = attributes.visibility_timeout {
            settings.visibility_timeout = value;
        }
        if let Some(value) = attributes.delay {
            settings.delay = value;
        }
        if let Some(value) = attributes.retention {
            settings.retention = value;
        }
        if dead_letter.is_some() {
            settings.dead_letter = dead_letter;
        }
        if let (Kind::Fifo(fifo), Some(value)) =
            (&mut self.kind, attributes.content_based_deduplication)
        {
            fifo.content_based_deduplication = value;
        }
        self.expire(now);
    }

    pub fn expire(&mut self, now: Time) {
        self.store.expire(now, self.settings.retention);
    }

    pub fn counts(&self, now: Time) -> Counts {
        self.store.counts(now)
    }

    pub fn purge(&mut self) {
        self.store.purge();
    }

    pub fn send(&mut self, now: Time, params: SendParams, ids: &mut Ids) -> Result<Sent, Error> {
        let SendParams {
            body,
            delay,
            group,
            deduplication_id,
        } = params;
        match &mut self.kind {
            Kind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(Error::UnsupportedOperation(Unsupported::FifoOnlyParameter));
                }
                self.store.ensure_room()?;
                let id = ids.next_message();
                let delay = delay.unwrap_or(self.settings.delay);
                self.store.push(new_message(id, body, now, delay, None));
                Ok(Sent { id, sequence: None })
            }
            Kind::Fifo(fifo) => {
                if delay.is_some() {
                    return Err(Error::UnsupportedOperation(Unsupported::DelayOnFifo));
                }
                let group = group.ok_or(Error::MissingParameter { name: "GroupId" })?;
                let key = deduplication_key(deduplication_id, &body, fifo)?;
                if let Some(original) = fifo.duplicate_of(&key, now) {
                    return Ok(original);
                }
                self.store.ensure_room()?;
                let id = ids.next_message();
                let sequence = fifo.take_sequence();
                fifo.accepted.insert(
                    key,
                    Accepted {
                        at: now,
                        id,
                        sequence,
                    },
                );
                let info = FifoInfo { group, sequence };
                self.store
                    .push(new_message(id, body, now, self.settings.delay, Some(info)));
                Ok(Sent {
                    id,
                    sequence: Some(sequence),
                })
            }
        }
    }

    pub fn receive(
        &mut self,
        now: Time,
        limit: usize,
        visibility: Option<Seconds>,
        ids: &mut Ids,
    ) -> Received {
        let visibility = visibility.unwrap_or(self.settings.visibility_timeout);
        let max_receive_count = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let scan = self
            .store
            .deliver(now, limit, visibility, max_receive_count, ids);
        let dead_lettered = self
            .settings
            .dead_letter
            .as_ref()
            .filter(|_| !scan.dead.is_empty())
            .map(|d| DeadLettered {
                to: d.queue.clone(),
                messages: scan.dead,
            });
        Received {
            delivered: scan.delivered,
            dead_lettered,
        }
    }

    /// Adds a message moved from another queue: receive state is reset, the delay is gone and a
    /// FIFO queue assigns a new sequence number. Neither the message limit nor deduplication
    /// applies.
    pub fn accept_dead_letter(&mut self, moved: Message, now: Time) {
        let fifo = match (&mut self.kind, moved.fifo) {
            (Kind::Fifo(fifo), Some(info)) => Some(FifoInfo {
                group: info.group,
                sequence: fifo.take_sequence(),
            }),
            (Kind::Standard, None) => None,
            _ => {
                unreachable!("a dead-letter queue has the same Fifo setting as the queue it serves")
            }
        };
        self.store.push(Message {
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Hold::Delayed { until: now },
            ..moved
        });
        self.expire(now);
    }

    pub fn delete_message(
        &mut self,
        handle: ReceiptHandle,
        raw: &str,
        now: Time,
    ) -> Result<(), Error> {
        self.store
            .take_in_flight(handle, now)
            .map(drop)
            .ok_or_else(|| expired(raw))
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        raw: &str,
        timeout: Seconds,
        now: Time,
    ) -> Result<(), Error> {
        self.store
            .restart_visibility(handle, now, timeout)
            .ok_or_else(|| expired(raw))
    }
}

fn expired(raw: &str) -> Error {
    Error::ReceiptHandleExpired {
        handle: raw.to_owned(),
    }
}

fn new_message(
    id: MessageId,
    body: Body,
    now: Time,
    delay: Seconds,
    fifo: Option<FifoInfo>,
) -> Message {
    Message {
        id,
        body,
        sent_at: now,
        receive_count: 0,
        first_received_at: None,
        fifo,
        hold: Hold::Delayed {
            until: now.after(delay),
        },
    }
}

fn deduplication_key(
    explicit: Option<DeduplicationId>,
    body: &Body,
    fifo: &Fifo,
) -> Result<String, Error> {
    match explicit {
        Some(id) => Ok(id.into_string()),
        None if fifo.content_based_deduplication => Ok(body.as_str().to_owned()),
        None => Err(Error::MissingParameter {
            name: "DeduplicationId",
        }),
    }
}
