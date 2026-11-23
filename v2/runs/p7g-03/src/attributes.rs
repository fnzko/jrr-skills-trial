//! Queue attributes (SPEC section 4): parsing, defaults and the name syntax.

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::params::{as_bool, as_integer, as_string, is_identifier};

pub const DEFAULT_VISIBILITY_TIMEOUT: u64 = 30;
pub const DEFAULT_RETENTION_SECONDS: u64 = 345_600;
const FIFO_SUFFIX: &str = ".fifo";

/// True if `name` follows the queue naming rules.
pub fn is_valid_queue_name(name: &str) -> bool {
    match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) if is_identifier(stem, 75) => true,
        _ => is_identifier(name, 80),
    }
}

pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The complete attribute set of an existing queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes named in a request; everything else is left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeRequest {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl AttributeRequest {
    /// Validates every attribute on its own: unknown names, types and ranges.
    /// `null` values count as absent.
    pub fn parse(attributes: &Map<String, Value>) -> Result<Self> {
        let mut request = Self::default();
        for (name, value) in attributes.iter().filter(|(_, v)| !v.is_null()) {
            let int = |range| as_integer(name, value, range).map(|n| n as u64);
            match name.as_str() {
                "VisibilityTimeout" => request.visibility_timeout = Some(int(0..=43_200)?),
                "DelaySeconds" => request.delay_seconds = Some(int(0..=900)?),
                "RetentionSeconds" => request.retention_seconds = Some(int(60..=1_209_600)?),
                "Fifo" => request.fifo = Some(as_bool(name, value)?),
                "ContentBasedDeduplication" => {
                    request.content_based_deduplication = Some(as_bool(name, value)?)
                }
                "DeadLetterQueue" => {
                    request.dead_letter_queue = Some(as_string(name, value)?.into())
                }
                "MaxReceiveCount" => request.max_receive_count = Some(int(1..=1000)?),
                _ => return Err(Error::invalid_value(format!("unknown attribute {name}"))),
            }
        }
        Ok(request)
    }

    /// True if exactly one half of the dead-letter configuration is present.
    pub fn has_partial_dead_letter(&self) -> bool {
        self.dead_letter_queue.is_some() != self.max_receive_count.is_some()
    }

    /// The dead-letter configuration, if both halves are present.
    pub fn dead_letter(&self) -> Option<DeadLetter> {
        Some(DeadLetter {
            queue: self.dead_letter_queue.clone()?,
            max_receive_count: self.max_receive_count?,
        })
    }

    /// The attributes of a new queue: given values over the defaults.
    pub fn with_defaults(&self) -> Attributes {
        Attributes {
            visibility_timeout: self
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: self.delay_seconds.unwrap_or(0),
            retention_seconds: self.retention_seconds.unwrap_or(DEFAULT_RETENTION_SECONDS),
            fifo: self.fifo.unwrap_or(false),
            content_based_deduplication: self.content_based_deduplication.unwrap_or(false),
            dead_letter: self.dead_letter(),
        }
    }
}

impl Attributes {
    /// Applies the attributes present in `request`, keeping the others.
    pub fn apply(&mut self, request: &AttributeRequest) {
        if let Some(v) = request.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = request.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = request.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = request.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if let Some(dead_letter) = request.dead_letter() {
            self.dead_letter = Some(dead_letter);
        }
    }

    /// The `Attributes` member of `GetQueueAttributes`.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("VisibilityTimeout".into(), self.visibility_timeout.into());
        m.insert("DelaySeconds".into(), self.delay_seconds.into());
        m.insert("RetentionSeconds".into(), self.retention_seconds.into());
        m.insert("Fifo".into(), self.fifo.into());
        m.insert(
            "ContentBasedDeduplication".into(),
            self.content_based_deduplication.into(),
        );
        if let Some(dl) = &self.dead_letter {
            m.insert("DeadLetterQueue".into(), dl.queue.clone().into());
            m.insert("MaxReceiveCount".into(), dl.max_receive_count.into());
        }
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_names() {
        assert!(is_valid_queue_name("jobs"));
        assert!(is_valid_queue_name("jobs.fifo"));
        assert!(is_valid_queue_name(&"a".repeat(80)));
        assert!(!is_valid_queue_name(&"a".repeat(81)));
        assert!(is_valid_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_valid_queue_name(".fifo"));
        assert!(!is_valid_queue_name(""));
        assert!(!is_valid_queue_name("a.b"));
        assert!(!is_valid_queue_name("a.fifo.fifo"));
        assert!(!is_valid_queue_name("a b"));
    }

    #[test]
    fn parse_rejects_unknown_names_and_bad_values() {
        let parse = |v: Value| AttributeRequest::parse(v.as_object().unwrap());
        assert!(parse(json!({"Nope": 1})).is_err());
        assert!(parse(json!({"DelaySeconds": 901})).is_err());
        assert!(parse(json!({"RetentionSeconds": 59})).is_err());
        assert!(parse(json!({"Fifo": "true"})).is_err());
        assert_eq!(
            parse(json!({"Nope": null, "DelaySeconds": null})),
            Ok(AttributeRequest::default())
        );
    }
}
