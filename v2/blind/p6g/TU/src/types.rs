//! Value types whose constructors enforce the rules of the specification.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// An integer known to lie in `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Bounded<const MIN: u64, const MAX: u64>(u64);

impl<const MIN: u64, const MAX: u64> Bounded<MIN, MAX> {
    pub const fn new(value: u64) -> Option<Self> {
        if value >= MIN && value <= MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// For defaults, so that a default outside the range fails to compile.
    pub const fn constant<const VALUE: u64>() -> Self {
        const { assert!(MIN <= VALUE && VALUE <= MAX) };
        Self(VALUE)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

pub type VisibilityTimeout = Bounded<0, 43_200>;
pub type DelaySeconds = Bounded<0, 900>;
pub type RetentionSeconds = Bounded<60, 1_209_600>;
pub type MaxReceiveCount = Bounded<1, 1000>;
pub type MaxMessages = Bounded<1, 10>;
pub type TickSeconds = Bounded<0, 1_209_600>;

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_token(s: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&s.len()) && s.chars().all(is_token_char)
}

const FIFO_SUFFIX: &str = ".fifo";

/// A queue name that follows the naming rules. Lookups use plain `&str`,
/// because a name that was never valid simply names no queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: &str) -> Result<Self, Error> {
        let valid = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => is_token(stem, 75),
            None => is_token(name, 80),
        };
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "QueueName must be 1 to 80 characters of A-Z a-z 0-9 _ -, optionally followed by .fifo (75 characters at most before the suffix)",
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(value: &str) -> Option<Self> {
        is_token(value, 128).then(|| Self(value.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn new(value: &str) -> Option<Self> {
        is_token(value, 128).then(|| Self(value.to_owned()))
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// Position of a message in the arrival order of its queue. It is also the
/// `SequenceNumber` that FIFO queues report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Sequence(u64);

impl Sequence {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Hands out the process-wide `MessageId`s and `ReceiptHandle`s.
#[derive(Debug, Default)]
pub struct Ids {
    messages: u64,
    receipts: u64,
}

impl Ids {
    pub fn next_message_id(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_receipt_handle(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    /// The handle `text` spells, if this process issued it. Only the
    /// canonical spelling `r-<n>` without padding or sign counts.
    pub fn issued_receipt_handle(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (1..=self.receipts)
            .contains(&number)
            .then_some(ReceiptHandle(number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_accepts_both_ends_and_nothing_beyond() {
        type B = Bounded<2, 4>;
        let accepted: Vec<bool> = (1..=5).map(|v| B::new(v).is_some()).collect();
        assert_eq!(accepted, [false, true, true, true, false]);
    }

    #[test]
    fn queue_names_follow_the_syntax() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("jobs.fifo", true),
            ("A_b-9", true),
            ("", false),
            (".fifo", false),
            ("a b", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("jobs.FIFO", false),
            ("é", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn invalid_queue_name_has_its_own_error_code() {
        let error = QueueName::new("a b").unwrap_err();
        assert_eq!(error.code, ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn group_ids_are_limited_to_128_token_characters() {
        assert!(GroupId::new(&"g".repeat(128)).is_some());
        assert!(GroupId::new(&"g".repeat(129)).is_none());
        assert!(GroupId::new("").is_none());
        assert!(GroupId::new("a.b").is_none());
    }

    #[test]
    fn only_canonical_issued_handles_are_recognised() {
        let mut ids = Ids::default();
        let first = ids.next_receipt_handle();
        ids.next_receipt_handle();
        let recognised = |text| ids.issued_receipt_handle(text).map(|h| h.to_string());
        assert_eq!(recognised("r-1"), Some("r-1".to_owned()));
        assert_eq!(recognised("r-2"), Some("r-2".to_owned()));
        for text in [
            "r-3", "r-0", "r-01", "R-1", "r-1 ", "r-+1", "r-", "1", "", "r--1",
        ] {
            assert_eq!(recognised(text), None, "{text:?}");
        }
        assert_eq!(first.to_string(), "r-1");
    }

    #[test]
    fn message_ids_count_from_one() {
        let mut ids = Ids::default();
        assert_eq!(ids.next_message_id().to_string(), "m-1");
        assert_eq!(ids.next_message_id().to_string(), "m-2");
    }
}
