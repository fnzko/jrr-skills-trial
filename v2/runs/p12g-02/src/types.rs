//! Small domain types shared by the parser, the broker and the responses.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// A length of virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u32);

/// A value of the virtual clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Instant(u64);

impl Instant {
    pub const START: Instant = Instant(0);

    /// Saturates instead of wrapping: the spec leaves behavior past 4000000000 open,
    /// and a wrapped clock would make old messages look new.
    pub fn plus(self, duration: Seconds) -> Instant {
        Instant(self.0.saturating_add(u64::from(duration.0)))
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidQueueName;

impl fmt::Display for InvalidQueueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "queue name must be 1 to 80 characters of A-Z a-z 0-9 _ -, \
             or 1 to 75 such characters followed by .fifo",
        )
    }
}

const FIFO_SUFFIX: &str = ".fifo";

/// A name that follows the naming rules of a queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: &str) -> Result<Self, InvalidQueueName> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name, 80),
        };
        let valid = (1..=max_len).contains(&stem.len()) && stem.bytes().all(is_identifier_byte);
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidQueueName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name carries the suffix that FIFO queues must have.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Lookups by `&str` are consistent with `Ord` and `Hash` because the wrapper derives them
// from the inner string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidIdentifier;

impl fmt::Display for InvalidIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("must be 1 to 128 characters of A-Z a-z 0-9 _ -")
    }
}

fn check_identifier(value: String) -> Result<String, InvalidIdentifier> {
    if (1..=128).contains(&value.len()) && value.bytes().all(is_identifier_byte) {
        Ok(value)
    } else {
        Err(InvalidIdentifier)
    }
}

/// The message group of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(value: String) -> Result<Self, InvalidIdentifier> {
        check_identifier(value).map(Self)
    }
}

/// The key a FIFO queue uses to recognize repeated sends.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn new(value: String) -> Result<Self, InvalidIdentifier> {
        check_identifier(value).map(Self)
    }

    /// The id used when the sender gives none: the body, which may be of any length.
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

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

/// The per-queue sequence number of a FIFO message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(pub u64);

/// The process-wide counters behind message ids and receipt handles.
#[derive(Debug)]
pub struct Ids {
    last_message: u64,
    last_receipt: u64,
}

impl Ids {
    pub fn new() -> Self {
        Self {
            last_message: 0,
            last_receipt: 0,
        }
    }

    pub fn next_message(&mut self) -> MessageId {
        self.last_message += 1;
        MessageId(self.last_message)
    }

    pub fn next_receipt(&mut self) -> ReceiptHandle {
        self.last_receipt += 1;
        ReceiptHandle(self.last_receipt)
    }

    /// The handle written as `text`, if this process has issued exactly that string.
    pub fn issued_receipt(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // A number too large for u64 cannot have been issued.
        let number: u64 = digits.parse().ok()?;
        (1..=self.last_receipt)
            .contains(&number)
            .then_some(ReceiptHandle(number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let fifo_max = format!("{}.fifo", "a".repeat(75));
        let fifo_too_long = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("Jobs_1-x", true),
            ("a.fifo", true),
            (long.as_str(), true),
            (fifo_max.as_str(), true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("jobs.fifo.fifo", false),
            ("caf\u{e9}", false),
            (too_long.as_str(), false),
            (fifo_too_long.as_str(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_ok(), valid, "name {name:?}");
        }
    }

    #[test]
    fn identifiers_are_limited_to_128_name_characters() {
        let longest = "x".repeat(128);
        let too_long = "x".repeat(129);
        assert_eq!(GroupId::new(longest.clone()), Ok(GroupId(longest)));
        assert_eq!(GroupId::new(too_long), Err(InvalidIdentifier));
        assert_eq!(GroupId::new(String::new()), Err(InvalidIdentifier));
        assert_eq!(DeduplicationId::new("a.b".into()), Err(InvalidIdentifier));
    }

    #[test]
    fn receipt_handles_must_match_an_issued_string_exactly() {
        let mut ids = Ids::new();
        let first = ids.next_receipt();
        ids.next_receipt();
        let cases = [
            ("r-1", Some(first)),
            ("r-2", Some(ReceiptHandle(2))),
            ("r-3", None),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(ids.issued_receipt(text), expected, "handle {text:?}");
        }
    }
}
