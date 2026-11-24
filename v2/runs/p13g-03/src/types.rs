//! Domain types shared by parsing, queues and replies.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Add;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// Virtual time: both instants on the clock and durations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u64);

impl Add for Seconds {
    type Output = Seconds;

    fn add(self, other: Seconds) -> Seconds {
        Seconds(self.0 + other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
    /// Parses the canonical text `r-<n>` with `n >= 1`. Anything else, such
    /// as `r-01` or a number too large for `u64`, can never have been issued.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(pub u64);

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// True for 1 to `max_len` characters from `A-Z a-z 0-9 _ -`.
fn is_identifier(text: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&text.len()) && text.chars().all(is_identifier_char)
}

/// The group of a FIFO message: 1 to 128 identifier characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<Self> {
        is_identifier(text, 128).then(|| Self(text.to_owned()))
    }
}

/// Returns `text` if it is a valid `DeduplicationId`.
pub fn deduplication_id(text: &str) -> Option<&str> {
    is_identifier(text, 128).then_some(text)
}

const FIFO_SUFFIX: &str = ".fifo";

/// A valid queue name. Lookups of existing queues take any `&str` through
/// `Borrow`, because an unknown name is a lookup failure, not a syntax error.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Result<Self, Error> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name, 80),
        };
        if is_identifier(stem, max_len) {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

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

/// The queue attributes that are plain values; the dead-letter configuration
/// is kept apart because it refers to another queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
}

impl Settings {
    pub fn defaults(kind: QueueKind) -> Self {
        Self {
            visibility_timeout: Seconds(30),
            delay: Seconds(0),
            retention: Seconds(345_600),
            kind,
        }
    }

    /// Overwrites the attributes present in `update`. `Fifo` is never
    /// applied: the kind of a queue does not change.
    pub fn apply(&mut self, update: &AttributeUpdate<'_>) {
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The attributes named in a request; `None` means not given.
#[derive(Debug, Default)]
pub struct AttributeUpdate<'a> {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<&'a str>,
    pub max_receive_count: Option<u32>,
}

impl<'a> AttributeUpdate<'a> {
    /// The requested dead-letter configuration: both members or neither.
    pub fn dead_letter(&self) -> Result<Option<(&'a str, u32)>, Error> {
        match (self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handle_accepts_only_canonical_text() {
        let cases = [
            ("r-1", Some(ReceiptHandle(1))),
            ("r-120", Some(ReceiptHandle(120))),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("r-+1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(ReceiptHandle::parse(text), expected, "{text:?}");
        }
    }

    #[test]
    fn queue_name_syntax() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs".to_owned(), true),
            ("jobs.fifo".to_owned(), true),
            ("A_b-9".to_owned(), true),
            ("a".repeat(80), true),
            ("a".repeat(81), false),
            (format!("{long_stem}.fifo"), true),
            (format!("{long_stem}a.fifo"), false),
            (String::new(), false),
            (".fifo".to_owned(), false),
            ("a.b".to_owned(), false),
            ("a.fifo.fifo".to_owned(), false),
            ("jobs ".to_owned(), false),
            ("j\u{f6}bs".to_owned(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(&name).is_ok(), valid, "{name:?}");
        }
    }
}
