//! Queue attributes (SPEC section 4): the stored form and the form a request carries.

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::params::{as_bool, as_int, as_string};
use crate::types::{QueueName, Seconds};

const MAX_VISIBILITY_TIMEOUT: u32 = 43_200;
const MAX_DELAY: u32 = 900;
const MIN_RETENTION: u32 = 60;
const MAX_RETENTION: u32 = 1_209_600;
const MAX_RECEIVE_COUNT_LIMIT: u32 = 1000;

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, QueueKind::Fifo { .. })
    }
}

/// Where messages that were received too often are moved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    queue: QueueName,
    max_receive_count: u32,
}

impl DeadLetter {
    pub fn new(queue: QueueName, max_receive_count: u32) -> Self {
        Self {
            queue,
            max_receive_count,
        }
    }

    pub fn queue(&self) -> &QueueName {
        &self.queue
    }

    pub fn max_receive_count(&self) -> u32 {
        self.max_receive_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub(crate) visibility_timeout: Seconds,
    pub(crate) delay: Seconds,
    pub(crate) retention: Seconds,
    pub(crate) kind: QueueKind,
    pub(crate) dead_letter: Option<DeadLetter>,
}

impl Attributes {
    /// The attributes a new queue gets when `update` names some of them.
    pub fn from_update(
        update: &AttributeUpdate,
        kind: QueueKind,
        dead_letter: Option<DeadLetter>,
    ) -> Self {
        Self {
            visibility_timeout: update
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: update.delay.unwrap_or(Seconds::ZERO),
            retention: update.retention.unwrap_or(DEFAULT_RETENTION),
            kind,
            dead_letter,
        }
    }

    /// Whether a create request with these values describes this very configuration.
    pub fn matches(
        &self,
        update: &AttributeUpdate,
        kind: QueueKind,
        dead_letter: Option<(&str, u32)>,
    ) -> bool {
        let current = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue().as_str(), d.max_receive_count()));
        self.visibility_timeout
            == update
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay == update.delay.unwrap_or(Seconds::ZERO)
            && self.retention == update.retention.unwrap_or(DEFAULT_RETENTION)
            && self.kind == kind
            && current == dead_letter
    }

    /// Drops the dead-letter configuration if it points at `deleted`.
    pub fn clear_dead_letter_to(&mut self, deleted: &QueueName) {
        if self
            .dead_letter
            .as_ref()
            .is_some_and(|d| d.queue() == deleted)
        {
            self.dead_letter = None;
        }
    }

    /// Overwrites the attributes `update` names. The type of a queue never changes, so `Fifo` is
    /// not applied; callers reject it beforehand.
    pub fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay {
            self.delay = value;
        }
        if let Some(value) = update.retention {
            self.retention = value;
        }
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, update.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The attributes of a request: every one optional, each already checked for type and range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub(crate) visibility_timeout: Option<Seconds>,
    pub(crate) delay: Option<Seconds>,
    pub(crate) retention: Option<Seconds>,
    pub(crate) fifo: Option<bool>,
    pub(crate) content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    pub fn parse(value: &Value) -> Result<Self> {
        let Value::Object(members) = value else {
            return Err(Error::invalid_value("Attributes must be an object"));
        };
        Self::parse_members(members)
    }

    fn parse_members(members: &Map<String, Value>) -> Result<Self> {
        let mut update = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    update.visibility_timeout = Some(Seconds::new(as_int(
                        name,
                        value,
                        0,
                        MAX_VISIBILITY_TIMEOUT,
                    )?));
                }
                "DelaySeconds" => {
                    update.delay = Some(Seconds::new(as_int(name, value, 0, MAX_DELAY)?));
                }
                "RetentionSeconds" => {
                    update.retention = Some(Seconds::new(as_int(
                        name,
                        value,
                        MIN_RETENTION,
                        MAX_RETENTION,
                    )?));
                }
                "Fifo" => update.fifo = Some(as_bool(name, value)?),
                "ContentBasedDeduplication" => {
                    update.content_based_deduplication = Some(as_bool(name, value)?);
                }
                "DeadLetterQueue" => {
                    update.dead_letter_queue = Some(as_string(name, value)?.to_owned());
                }
                "MaxReceiveCount" => {
                    update.max_receive_count =
                        Some(as_int(name, value, 1, MAX_RECEIVE_COUNT_LIMIT)?);
                }
                _ => {
                    return Err(Error::invalid_value(format!("unknown attribute {name:?}")));
                }
            }
        }
        Ok(update)
    }

    /// The dead-letter configuration as requested: both members or neither.
    pub fn dead_letter_request(&self) -> Result<Option<(&str, u32)>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}
