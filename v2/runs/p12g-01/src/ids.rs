//! Identifiers and validated text types.
//!
//! Types with a validity rule keep their field private so that a value of the
//! type is always valid.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

const FIFO_SUFFIX: &str = ".fifo";

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn parse_token(parameter: &str, text: &str, max_len: usize) -> Result<String, Error> {
    // Only ASCII passes the character check, so byte length is character length.
    if (1..=max_len).contains(&text.len()) && text.chars().all(is_token_char) {
        Ok(text.to_owned())
    } else {
        Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!("{parameter} must be 1 to {max_len} characters from A-Z a-z 0-9 _ -"),
        ))
    }
}

/// A queue name that follows the naming rules of the spec.
///
/// Lookups of existing queues use plain `&str`, because any string may be
/// looked up; only names of queues that get created are validated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (stem, max_len) = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (text, 80),
        };
        if (1..=max_len).contains(&stem.len()) && stem.chars().all(is_token_char) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue name {text:?} breaks the naming rules"),
            ))
        }
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lets a `BTreeMap<QueueName, _>` be searched with a `&str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// The message group of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_token("GroupId", text, 128).map(Self)
    }
}

/// What a FIFO queue deduplicates on: a `DeduplicationId`, or else the body.
///
/// Both share one namespace, so an id equal to some body matches that body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn from_id(text: &str) -> Result<Self, Error> {
        parse_token("DeduplicationId", text, 128).map(Self)
    }

    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

/// `m-<n>`: counts the messages accepted by the process.
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

/// Position of a message among the messages added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(pub u64);

/// `r-<n>`: counts the deliveries made by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

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

/// Issues receipt handles and recognizes the ones it issued.
#[derive(Debug, Default)]
pub struct ReceiptHandles {
    issued: u64,
}

impl ReceiptHandles {
    pub fn issue(&mut self) -> ReceiptHandle {
        self.issued += 1;
        ReceiptHandle(self.issued)
    }

    /// Returns the handle written as `text` if this process issued it.
    /// Only the canonical spelling matches: no sign, no leading zero.
    pub fn parse_issued(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (number <= self.issued).then_some(ReceiptHandle(number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_plain = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_plain = "a".repeat(81);
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let cases: [(&str, bool); 12] = [
            ("jobs", true),
            ("A_b-9", true),
            ("jobs.fifo", true),
            (&long_plain, true),
            (&long_fifo, true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            (&too_long_plain, false),
            (&too_long_fifo, false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "name {name:?}");
        }
    }

    #[test]
    fn queue_name_error_code() {
        let err = QueueName::parse("a b").unwrap_err();
        assert_eq!(err.code, ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn fifo_names_are_recognized_by_suffix() {
        assert!(QueueName::parse("jobs.fifo").unwrap().is_fifo());
        assert!(!QueueName::parse("jobs").unwrap().is_fifo());
    }

    #[test]
    fn group_id_syntax() {
        let long = "g".repeat(128);
        let too_long = "g".repeat(129);
        let cases: [(&str, bool); 6] = [
            ("g", true),
            ("A-z_0", true),
            (&long, true),
            ("", false),
            ("a.b", false),
            (&too_long, false),
        ];
        for (text, valid) in cases {
            assert_eq!(GroupId::parse(text).is_ok(), valid, "group {text:?}");
        }
    }

    #[test]
    fn dedup_id_and_body_share_a_namespace() {
        assert_eq!(
            DedupKey::from_id("abc").unwrap(),
            DedupKey::from_body("abc")
        );
    }

    #[test]
    fn only_issued_canonical_handles_parse() {
        let mut handles = ReceiptHandles::default();
        let first = handles.issue();
        let second = handles.issue();
        assert_eq!(handles.parse_issued("r-1"), Some(first));
        assert_eq!(handles.parse_issued("r-2"), Some(second));
        for text in [
            "r-3",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-+1",
            "r-",
            "1",
            "r--1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(handles.parse_issued(text), None, "handle {text:?}");
        }
    }

    #[test]
    fn ids_are_written_with_their_prefix() {
        assert_eq!(MessageId(7).to_string(), "m-7");
        let mut handles = ReceiptHandles::default();
        assert_eq!(handles.issue().to_string(), "r-1");
    }
}
