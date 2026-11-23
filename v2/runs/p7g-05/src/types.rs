//! Small value types shared by the whole crate: virtual time, identifiers and validated names.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A length of virtual time in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u32);

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    /// The point `duration` after `self`; saturates instead of wrapping.
    pub fn plus(self, duration: Seconds) -> Time {
        Time(self.0.saturating_add(u64::from(duration.0)))
    }
}

/// `m-<n>`: the identity of an accepted message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

/// `r-<n>`: the identity of one delivery of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Reads the canonical text form `r-<n>` with `n >= 1`; any other spelling was never issued.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Position of a message among all messages added to one FIFO queue, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    pub fn next(self) -> SequenceNumber {
        SequenceNumber(self.0.saturating_add(1))
    }
}

/// Hands out the process-wide message ids and receipt handles, each counting from 1.
#[derive(Debug)]
pub struct IdGenerator {
    messages: u64,
    handles: u64,
}

impl IdGenerator {
    pub fn new() -> Self {
        Self {
            messages: 0,
            handles: 0,
        }
    }

    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_handle(&mut self) -> ReceiptHandle {
        self.handles += 1;
        ReceiptHandle(self.handles)
    }

    /// Whether `handle` has been handed out by `next_handle`.
    pub fn has_issued(&self, handle: ReceiptHandle) -> bool {
        handle.0 >= 1 && handle.0 <= self.handles
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

const FIFO_SUFFIX: &str = ".fifo";

/// The name of a queue that is being created; it obeys the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: String) -> Result<Self, Error> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name.as_str(), 80),
        };
        if (1..=max_len).contains(&stem.len()) && stem.chars().all(is_name_char) {
            Ok(Self(name))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ))
        }
    }

    /// Whether the name asks for a FIFO queue.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Queues are looked up by whatever string a command carries, valid or not.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_token(text: &str) -> bool {
    (1..=128).contains(&text.len()) && text.chars().all(is_name_char)
}

/// The message group of a send to a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: String) -> Result<Self, Error> {
        if is_token(&text) {
            Ok(Self(text))
        } else {
            Err(Error::invalid_value(
                "GroupId",
                "1 to 128 characters from A-Z a-z 0-9 _ -",
            ))
        }
    }
}

/// The caller-chosen deduplication id of a send to a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: String) -> Result<Self, Error> {
        if is_token(&text) {
            Ok(Self(text))
        } else {
            Err(Error::invalid_value(
                "DeduplicationId",
                "1 to 128 characters from A-Z a-z 0-9 _ -",
            ))
        }
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_stem = "a".repeat(75);
        let table: Vec<(String, bool)> = vec![
            ("jobs".into(), true),
            ("A_b-9".into(), true),
            ("jobs.fifo".into(), true),
            ("a".repeat(80), true),
            (format!("{long_stem}.fifo"), true),
            (format!("{}.fifo", "a".repeat(76)), false),
            ("a".repeat(81), false),
            (String::new(), false),
            (".fifo".into(), false),
            ("a.fifo.fifo".into(), false),
            ("has space".into(), false),
            ("dot.name".into(), false),
            ("caf\u{e9}".into(), false),
        ];
        for (name, valid) in table {
            assert_eq!(QueueName::parse(name.clone()).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn queue_name_error_has_its_own_code() {
        let error = QueueName::parse("bad name".into()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn only_canonical_receipt_handles_parse() {
        let table = [
            ("r-1", Some(ReceiptHandle(1))),
            ("r-42", Some(ReceiptHandle(42))),
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("r--1", None),
            ("r-99999999999999999999", None),
            ("m-1", None),
        ];
        for (text, expected) in table {
            assert_eq!(ReceiptHandle::parse(text), expected, "{text:?}");
        }
    }

    #[test]
    fn ids_count_from_one_and_handles_are_issued_in_order() {
        let mut ids = IdGenerator::new();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert!(!ids.has_issued(ReceiptHandle(1)));
        assert_eq!(ids.next_handle().to_string(), "r-1");
        assert!(ids.has_issued(ReceiptHandle(1)));
        assert!(!ids.has_issued(ReceiptHandle(2)));
        assert!(!ids.has_issued(ReceiptHandle(0)));
    }

    #[test]
    fn time_plus_saturates() {
        assert_eq!(Time::START.plus(Seconds(5)), Time(5));
        assert_eq!(Time(u64::MAX).plus(Seconds(1)), Time(u64::MAX));
    }

    #[test]
    fn group_and_deduplication_ids_share_the_token_rule() {
        assert_eq!(GroupId::parse("g-1_A".into()), Ok(GroupId("g-1_A".into())));
        let rejected = [
            GroupId::parse("a".repeat(129)).unwrap_err().code(),
            GroupId::parse(String::new()).unwrap_err().code(),
            DeduplicationId::parse("a b".into()).unwrap_err().code(),
        ];
        assert_eq!(rejected, [ErrorCode::InvalidParameterValue; 3]);
        assert_eq!(
            DeduplicationId::parse("d1".into()).map(DeduplicationId::into_string),
            Ok("d1".to_string())
        );
    }
}
