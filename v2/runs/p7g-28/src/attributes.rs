//! Queue attributes (SPEC section 4): the full set a queue holds, and the partial
//! set a request may carry.

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{as_bool, as_int, as_string, member};

pub const DEFAULT_VISIBILITY_TIMEOUT: i64 = 30;
pub const DEFAULT_RETENTION_SECONDS: i64 = 345_600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue: String,
    pub max_receive_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: i64,
    pub delay_seconds: i64,
    pub retention_seconds: i64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterConfig>,
}

/// The attributes present in a request, each validated on its own.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<i64>,
    pub delay_seconds: Option<i64>,
    pub retention_seconds: Option<i64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<i64>,
}

const NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

impl AttributeSet {
    /// Validates the members of an `Attributes` object. Unknown names are rejected.
    pub fn parse(map: &Map<String, Value>) -> Result<Self> {
        if let Some(unknown) = map
            .keys()
            .find(|k| !NAMES.contains(&k.as_str()) && member(map, k).is_some())
        {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("unknown attribute {unknown}"),
            ));
        }
        let get = |name| member(map, name);
        Ok(Self {
            visibility_timeout: get("VisibilityTimeout")
                .map(|v| as_int("VisibilityTimeout", v, &(0..=43_200)))
                .transpose()?,
            delay_seconds: get("DelaySeconds")
                .map(|v| as_int("DelaySeconds", v, &(0..=900)))
                .transpose()?,
            retention_seconds: get("RetentionSeconds")
                .map(|v| as_int("RetentionSeconds", v, &(60..=1_209_600)))
                .transpose()?,
            fifo: get("Fifo").map(|v| as_bool("Fifo", v)).transpose()?,
            content_based_deduplication: get("ContentBasedDeduplication")
                .map(|v| as_bool("ContentBasedDeduplication", v))
                .transpose()?,
            dead_letter_queue: get("DeadLetterQueue")
                .map(|v| as_string("DeadLetterQueue", v).map(str::to_owned))
                .transpose()?,
            max_receive_count: get("MaxReceiveCount")
                .map(|v| as_int("MaxReceiveCount", v, &(1..=1000)))
                .transpose()?,
        })
    }

    /// True if exactly one half of the dead-letter configuration is present.
    pub fn has_partial_dead_letter(&self) -> bool {
        self.dead_letter_queue.is_some() != self.max_receive_count.is_some()
    }

    fn dead_letter(&self) -> Option<DeadLetterConfig> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Some(DeadLetterConfig {
                queue: queue.clone(),
                max_receive_count,
            }),
            _ => None,
        }
    }
}

impl Attributes {
    /// The attributes of a new queue: requested values over defaults.
    pub fn from_set(set: &AttributeSet) -> Self {
        Self {
            visibility_timeout: set.visibility_timeout.unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: set.delay_seconds.unwrap_or(0),
            retention_seconds: set.retention_seconds.unwrap_or(DEFAULT_RETENTION_SECONDS),
            fifo: set.fifo.unwrap_or(false),
            content_based_deduplication: set.content_based_deduplication.unwrap_or(false),
            dead_letter: set.dead_letter(),
        }
    }

    /// Overwrites the attributes present in `set`; the others keep their values.
    pub fn apply(&mut self, set: &AttributeSet) {
        self.visibility_timeout = set.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay_seconds = set.delay_seconds.unwrap_or(self.delay_seconds);
        self.retention_seconds = set.retention_seconds.unwrap_or(self.retention_seconds);
        self.content_based_deduplication = set
            .content_based_deduplication
            .unwrap_or(self.content_based_deduplication);
        if let Some(config) = set.dead_letter() {
            self.dead_letter = Some(config);
        }
    }

    /// The `Attributes` object of `GetQueueAttributes`.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("VisibilityTimeout".into(), json!(self.visibility_timeout));
        map.insert("DelaySeconds".into(), json!(self.delay_seconds));
        map.insert("RetentionSeconds".into(), json!(self.retention_seconds));
        map.insert("Fifo".into(), json!(self.fifo));
        map.insert(
            "ContentBasedDeduplication".into(),
            json!(self.content_based_deduplication),
        );
        if let Some(dl) = &self.dead_letter {
            map.insert("DeadLetterQueue".into(), json!(dl.queue));
            map.insert("MaxReceiveCount".into(), json!(dl.max_receive_count));
        }
        Value::Object(map)
    }
}
