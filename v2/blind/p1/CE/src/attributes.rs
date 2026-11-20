use serde_json::Value;

use crate::error::Error;
use crate::params::{Params, invalid};

pub(crate) const VISIBILITY_TIMEOUT: &str = "VisibilityTimeout";
pub(crate) const DELAY_SECONDS: &str = "DelaySeconds";
pub(crate) const RETENTION_SECONDS: &str = "RetentionSeconds";
pub(crate) const FIFO: &str = "Fifo";
pub(crate) const CONTENT_BASED_DEDUPLICATION: &str = "ContentBasedDeduplication";
pub(crate) const DEAD_LETTER_QUEUE: &str = "DeadLetterQueue";
pub(crate) const MAX_RECEIVE_COUNT: &str = "MaxReceiveCount";

const KNOWN: [&str; 7] = [
    VISIBILITY_TIMEOUT,
    DELAY_SECONDS,
    RETENTION_SECONDS,
    FIFO,
    CONTENT_BASED_DEDUPLICATION,
    DEAD_LETTER_QUEUE,
    MAX_RECEIVE_COUNT,
];

/// Where messages that were received too often are moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl Attributes {
    /// The attributes as `GetQueueAttributes` reports them: the dead-letter
    /// configuration only when it is set.
    pub fn to_json(&self) -> Value {
        let mut members = serde_json::Map::new();
        members.insert(VISIBILITY_TIMEOUT.into(), self.visibility_timeout.into());
        members.insert(DELAY_SECONDS.into(), self.delay_seconds.into());
        members.insert(RETENTION_SECONDS.into(), self.retention_seconds.into());
        members.insert(FIFO.into(), self.fifo.into());
        members.insert(
            CONTENT_BASED_DEDUPLICATION.into(),
            self.content_based_deduplication.into(),
        );
        if let Some(dead_letter) = &self.dead_letter {
            members.insert(DEAD_LETTER_QUEUE.into(), dead_letter.queue.as_str().into());
            members.insert(
                MAX_RECEIVE_COUNT.into(),
                dead_letter.max_receive_count.into(),
            );
        }
        Value::Object(members)
    }

    /// Overwrites the attributes that `update` names. `dead_letter` is the
    /// update's checked dead-letter configuration, if it has one.
    pub fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = update.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = update.fifo {
            self.fifo = value;
        }
        if let Some(value) = update.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The attributes present in a request, each checked on its own.
#[derive(Debug, Default)]
pub(crate) struct AttributeUpdate {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(invalid("Attributes", "an object"));
        };
        if let Some(name) = members
            .iter()
            .find(|(name, value)| !value.is_null() && !KNOWN.contains(&name.as_str()))
            .map(|(name, _)| name)
        {
            return Err(Error::new(
                crate::error::ErrorCode::InvalidParameterValue,
                format!("unknown attribute `{name}`"),
            ));
        }
        let p = Params::new(members);
        Ok(Self {
            visibility_timeout: p.integer(VISIBILITY_TIMEOUT, 0..=43_200)?,
            delay_seconds: p.integer(DELAY_SECONDS, 0..=900)?,
            retention_seconds: p.integer(RETENTION_SECONDS, 60..=1_209_600)?,
            fifo: p.boolean(FIFO)?,
            content_based_deduplication: p.boolean(CONTENT_BASED_DEDUPLICATION)?,
            dead_letter_queue: p.string(DEAD_LETTER_QUEUE)?.map(str::to_owned),
            max_receive_count: p.integer(MAX_RECEIVE_COUNT, 1..=1000)?,
        })
    }

    /// The dead-letter configuration, which must be given whole or not at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::new(
                crate::error::ErrorCode::InvalidParameterValue,
                format!("`{DEAD_LETTER_QUEUE}` and `{MAX_RECEIVE_COUNT}` must be given together"),
            )),
        }
    }
}
