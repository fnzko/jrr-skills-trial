//! Small domain types shared by the command parser and the queue engine.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A duration or a point of the virtual clock, in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(u64);

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Sums saturate: the clock and every duration are far below `u64::MAX`, so
    /// saturation only keeps pathological input from panicking.
    pub fn saturating_add(self, other: Seconds) -> Seconds {
        Seconds(self.0.saturating_add(other.0))
    }
}

impl From<u32> for Seconds {
    fn from(seconds: u32) -> Self {
        Self(u64::from(seconds))
    }
}

/// Identifier of an accepted message, written `m-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

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

/// Identifier of one delivery, written `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        Self(number)
    }

    /// Parses a handle that is spelled exactly as it was issued and whose number
    /// is at most `issued`, the count of handles handed out so far.
    pub fn parse_issued(text: &str, issued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        let number: u64 = digits.parse().ok().filter(|_| canonical)?;
        (number <= issued).then_some(Self(number))
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

/// Position of a message among those added to one FIFO queue, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

const FIFO_SUFFIX: &str = ".fifo";

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that follows the naming rules. Lookups of existing queues take
/// plain `&str` through `Borrow`, because any string may be looked up.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (stem, max_stem) = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (text, 80),
        };
        if (1..=max_stem).contains(&stem.len()) && stem.chars().all(is_name_char) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "queue name must be 1 to 80 characters from A-Z a-z 0-9 _ -, optionally followed by .fifo",
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

fn parse_identifier(what: &str, text: &str) -> Result<String, Error> {
    if (1..=128).contains(&text.len()) && text.chars().all(is_name_char) {
        Ok(text.to_owned())
    } else {
        Err(Error::invalid_value(format!(
            "{what} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        )))
    }
}

/// Message group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_identifier("GroupId", text).map(Self)
    }
}

/// Caller-chosen deduplication id of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_identifier("DeduplicationId", text).map(Self)
    }
}

/// The deduplication id a FIFO send is judged by: an explicit id or the body.
/// Both share one namespace, so a body equal to an id is a duplicate of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

impl From<DeduplicationId> for DedupKey {
    fn from(id: DeduplicationId) -> Self {
        Self(id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("A-b_9", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            ("caf\u{e9}", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}b.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "name {name:?}");
        }
    }

    #[test]
    fn invalid_queue_name_has_its_own_code() {
        let error = QueueName::parse("a b").unwrap_err();
        assert_eq!(error.code(), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn identifiers_are_one_to_128_name_characters() {
        let cases = [
            ("g", true),
            ("Group_1-x", true),
            (&"a".repeat(128), true),
            (&"a".repeat(129), false),
            ("", false),
            ("a.b", false),
            ("a b", false),
        ];
        for (text, valid) in cases {
            assert_eq!(GroupId::parse(text).is_ok(), valid, "group {text:?}");
            assert_eq!(
                DeduplicationId::parse(text).is_ok(),
                valid,
                "dedup {text:?}"
            );
        }
    }

    #[test]
    fn receipt_handles_must_be_spelled_as_issued() {
        let cases = [
            ("r-1", Some(1)),
            ("r-3", Some(3)),
            ("r-4", None),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r--1", None),
            ("r-", None),
            ("r-99999999999999999999999", None),
            ("1", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse_issued(text, 3);
            assert_eq!(parsed, expected.map(ReceiptHandle::new), "handle {text:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId::new(7).to_string(), "m-7");
        assert_eq!(ReceiptHandle::new(12).to_string(), "r-12");
    }
}
