//! Small domain types shared by the whole crate.

use std::borrow::Borrow;
use std::fmt;
use std::sync::Arc;

use serde::{Serialize, Serializer};

/// A length of virtual time in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u32);

/// A value of the virtual clock, in seconds since the process started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    pub fn plus(self, seconds: Seconds) -> Time {
        Time(self.0.saturating_add(u64::from(seconds.0)))
    }
}

/// `m-<n>`: the identifier of an accepted message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: MessageId = MessageId(1);

    /// Returns the current id and moves to the next one.
    pub fn take(&mut self) -> MessageId {
        let id = *self;
        self.0 += 1;
        id
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

/// `r-<n>`: the identifier of one delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// Hands out receipt handles and recognizes the ones it handed out.
#[derive(Debug, Default)]
pub struct HandleIssuer {
    issued: u64,
}

impl HandleIssuer {
    pub fn issue(&mut self) -> ReceiptHandle {
        self.issued += 1;
        ReceiptHandle(self.issued)
    }

    /// Returns the handle written as `text`, if that exact string was issued.
    pub fn recognize(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (number <= self.issued).then_some(ReceiptHandle(number))
    }
}

/// Position of a message in its queue's arrival order; also the FIFO
/// `SequenceNumber`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    /// Returns the current number and moves to the next one.
    pub fn take(&mut self) -> SequenceNumber {
        let number = *self;
        self.0 += 1;
        number
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_token(text: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&text.len()) && text.chars().all(is_token_char)
}

/// A name taken from a request that is only used to look a queue up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueRef(String);

impl From<&str> for QueueRef {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl QueueRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const FIFO_SUFFIX: &str = ".fifo";

/// The name of an existing queue; it follows the naming rules.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueName(Arc<str>);

impl QueueName {
    /// Returns `None` if `text` breaks the naming rules.
    pub fn parse(text: &str) -> Option<Self> {
        let valid = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => is_token(stem, 75),
            None => is_token(text, 80),
        };
        valid.then(|| Self(Arc::from(text)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

impl Serialize for QueueName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A FIFO message group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(Arc<str>);

impl GroupId {
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text, 128).then(|| Self(Arc::from(text)))
    }
}

impl Serialize for GroupId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// An explicit deduplication id of a FIFO send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text, 128).then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("A_z-9", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("a.fifo.fifo", false),
            ("é", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "{name}");
        }
    }

    #[test]
    fn handles_are_recognized_only_in_canonical_form() {
        let mut issuer = HandleIssuer::default();
        let first = issuer.issue();
        issuer.issue();
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
            ("1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(issuer.recognize(text), expected, "{text}");
        }
    }

    #[test]
    fn identifiers_display_as_on_the_wire() {
        let mut id = MessageId::FIRST;
        id.take();
        assert_eq!(id.to_string(), "m-2");
        assert_eq!(ReceiptHandle(7).to_string(), "r-7");
    }
}
