//! Queue names, queue attributes and the rules that tie them together.

use std::borrow::Borrow;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{bounded_integer, invalid};
use crate::time::Seconds;

const FIFO_SUFFIX: &str = ".fifo";
const MAX_NAME_LEN: usize = 80;

/// A queue name that follows the naming rules of the specification.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Result<QueueName> {
        let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
        let stem_limit = MAX_NAME_LEN - (name.len() - stem.len());
        let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        if stem.is_empty() || stem.len() > stem_limit || !stem.chars().all(allowed) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ));
        }
        Ok(QueueName(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Lookups by plain `&str` must hash and order like the wrapped string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Whether a queue keeps order per group, and how it deduplicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, QueueKind::Fifo { .. })
    }
}

/// Where messages go once they have been received too often.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, with every default filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

impl Config {
    pub fn standard_defaults() -> Config {
        Config {
            visibility_timeout: Seconds::new(30),
            delay: Seconds::ZERO,
            retention: Seconds::new(345_600),
            kind: QueueKind::Standard,
            dead_letter: None,
        }
    }
}

/// The attributes named in a request, before defaults are applied.
#[derive(Debug, Default)]
pub struct AttributeSet {
    visibility_timeout: Option<Seconds>,
    delay: Option<Seconds>,
    retention: Option<Seconds>,
    fifo: Option<bool>,
    content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl AttributeSet {
    /// Checks the type and range of every attribute. An attribute whose value
    /// is `null` counts as absent.
    pub fn parse(attributes: &Value) -> Result<AttributeSet> {
        let Value::Object(members) = attributes else {
            return Err(invalid("Attributes must be an object"));
        };
        let mut set = AttributeSet::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    set.visibility_timeout = Some(seconds(name, value, 0..=43_200)?)
                }
                "DelaySeconds" => set.delay = Some(seconds(name, value, 0..=900)?),
                "RetentionSeconds" => set.retention = Some(seconds(name, value, 60..=1_209_600)?),
                "Fifo" => set.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    set.content_based_deduplication = Some(boolean(name, value)?)
                }
                "DeadLetterQueue" => {
                    let queue = value
                        .as_str()
                        .ok_or_else(|| invalid("DeadLetterQueue must be a string"))?;
                    set.dead_letter_queue = Some(queue.to_owned());
                }
                "MaxReceiveCount" => {
                    set.max_receive_count = Some(bounded_integer(name, value, 1..=1000)?)
                }
                _ => return Err(invalid(format!("unknown attribute {name}"))),
            }
        }
        Ok(set)
    }

    pub fn fifo(&self) -> Option<bool> {
        self.fifo
    }

    pub fn content_based_deduplication(&self) -> Option<bool> {
        self.content_based_deduplication
    }

    /// The dead-letter configuration of the request: both attributes or none.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(invalid(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// The configuration of a new queue: given attributes over the defaults.
    pub fn into_new_config(self, dead_letter: Option<DeadLetter>) -> Config {
        let defaults = Config::standard_defaults();
        let kind = if self.fifo == Some(true) {
            QueueKind::Fifo {
                content_based_deduplication: self.content_based_deduplication == Some(true),
            }
        } else {
            QueueKind::Standard
        };
        Config {
            visibility_timeout: self
                .visibility_timeout
                .unwrap_or(defaults.visibility_timeout),
            delay: self.delay.unwrap_or(defaults.delay),
            retention: self.retention.unwrap_or(defaults.retention),
            kind,
            dead_letter,
        }
    }

    /// Overwrites the attributes of `config` that this set names.
    pub fn apply_to(self, config: &mut Config, dead_letter: Option<DeadLetter>) {
        if let Some(value) = self.visibility_timeout {
            config.visibility_timeout = value;
        }
        if let Some(value) = self.delay {
            config.delay = value;
        }
        if let Some(value) = self.retention {
            config.retention = value;
        }
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut config.kind, self.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            config.dead_letter = dead_letter;
        }
    }
}

fn seconds(name: &str, value: &Value, range: std::ops::RangeInclusive<u32>) -> Result<Seconds> {
    bounded_integer(name, value, range).map(Seconds::new)
}

fn boolean(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{name} must be a boolean")))
}

/// Builds the `Attributes` object of `GetQueueAttributes`.
pub fn attributes_json(config: &Config) -> Map<String, Value> {
    let mut attributes = Map::new();
    attributes.insert(
        "VisibilityTimeout".into(),
        config.visibility_timeout.as_secs().into(),
    );
    attributes.insert("DelaySeconds".into(), config.delay.as_secs().into());
    attributes.insert("RetentionSeconds".into(), config.retention.as_secs().into());
    let (fifo, deduplication) = match config.kind {
        QueueKind::Standard => (false, false),
        QueueKind::Fifo {
            content_based_deduplication,
        } => (true, content_based_deduplication),
    };
    attributes.insert("Fifo".into(), fifo.into());
    attributes.insert("ContentBasedDeduplication".into(), deduplication.into());
    if let Some(dead_letter) = &config.dead_letter {
        attributes.insert("DeadLetterQueue".into(), dead_letter.queue.clone().into());
        attributes.insert(
            "MaxReceiveCount".into(),
            dead_letter.max_receive_count.into(),
        );
    }
    attributes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let longest_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let valid = [
            "a",
            "A_z-9",
            long.as_str(),
            "q.fifo",
            longest_fifo.as_str(),
            "-.fifo",
        ];
        for name in valid {
            assert_eq!(QueueName::parse(name).unwrap().as_str(), name);
        }
        let invalid = [
            "",
            too_long.as_str(),
            too_long_fifo.as_str(),
            ".fifo",
            "a.b",
            "a.fifo.fifo",
            "a b",
            "é",
            "a.FIFO",
            "a.fifo ",
            "a/b",
        ];
        for name in invalid {
            assert_eq!(
                QueueName::parse(name).unwrap_err().code,
                ErrorCode::QueueNameInvalid,
                "{name:?}"
            );
        }
    }

    #[test]
    fn parse_rejects_unknown_names_and_bad_types() {
        for bad in [
            r#"{"Bogus":1}"#,
            r#"{"Fifo":"yes"}"#,
            r#"{"VisibilityTimeout":43201}"#,
            r#"{"RetentionSeconds":59}"#,
            r#"{"MaxReceiveCount":0}"#,
            r#"{"DeadLetterQueue":1}"#,
            "[]",
        ] {
            let value: Value = serde_json::from_str(bad).unwrap();
            assert_eq!(
                AttributeSet::parse(&value).unwrap_err().code,
                ErrorCode::InvalidParameterValue,
                "{bad}"
            );
        }
    }

    #[test]
    fn null_attributes_count_as_absent() {
        let value: Value = serde_json::from_str(r#"{"Bogus":null,"Fifo":null}"#).unwrap();
        let set = AttributeSet::parse(&value).unwrap();
        assert_eq!(set.fifo(), None);
        assert_eq!(set.into_new_config(None), Config::standard_defaults());
    }

    #[test]
    fn dead_letter_attributes_come_in_pairs() {
        let one: Value = serde_json::from_str(r#"{"MaxReceiveCount":3}"#).unwrap();
        assert_eq!(
            AttributeSet::parse(&one)
                .unwrap()
                .dead_letter()
                .unwrap_err()
                .code,
            ErrorCode::InvalidParameterValue
        );
        let both: Value =
            serde_json::from_str(r#"{"MaxReceiveCount":3,"DeadLetterQueue":"d"}"#).unwrap();
        assert_eq!(
            AttributeSet::parse(&both).unwrap().dead_letter().unwrap(),
            Some(DeadLetter {
                queue: "d".into(),
                max_receive_count: 3
            })
        );
    }
}
