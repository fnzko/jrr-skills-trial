//! Validated values and identifiers. Each type with a validity rule can only be built through
//! a constructor that checks it.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::time::Seconds;

/// An integer known to lie in `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ranged<const MIN: u32, const MAX: u32>(u32);

/// A value outside the range its type allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange {
    min: u32,
    max: u32,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "must be an integer from {} to {}", self.min, self.max)
    }
}

impl<const MIN: u32, const MAX: u32> Ranged<MIN, MAX> {
    /// For constants only: an out-of-range value fails the build, not the program.
    pub const fn constant(value: u32) -> Self {
        assert!(MIN <= value && value <= MAX, "constant outside its range");
        Self(value)
    }

    pub fn get(self) -> u32 {
        self.0
    }

    pub fn seconds(self) -> Seconds {
        Seconds::new(u64::from(self.0))
    }
}

impl<const MIN: u32, const MAX: u32> TryFrom<i64> for Ranged<MIN, MAX> {
    type Error = OutOfRange;

    fn try_from(value: i64) -> Result<Self, OutOfRange> {
        u32::try_from(value)
            .ok()
            .filter(|v| (MIN..=MAX).contains(v))
            .map(Self)
            .ok_or(OutOfRange { min: MIN, max: MAX })
    }
}

pub type VisibilityTimeout = Ranged<0, 43200>;
pub type DelaySeconds = Ranged<0, 900>;
pub type RetentionSeconds = Ranged<60, 1_209_600>;
pub type MaxReceiveCount = Ranged<1, 1000>;
pub type MaxMessages = Ranged<1, 10>;
pub type TickSeconds = Ranged<0, 1_209_600>;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A string breaking the character or length rule of the identifier it was meant to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidName;

const FIFO_SUFFIX: &str = ".fifo";

/// The name of an existing or creatable queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Result<Self, InvalidName> {
        let valid = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (1..=75).contains(&stem.len()) && stem.chars().all(is_name_char),
            None => (1..=80).contains(&name.len()) && name.chars().all(is_name_char),
        };
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

/// Lookups by `&str` must order like the owned name, which `String`'s `Borrow<str>` guarantees.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_token(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.chars().all(is_name_char)
}

/// Orders FIFO messages: messages of one group are delivered one at a time, in order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(value: &str) -> Result<Self, InvalidName> {
        is_token(value)
            .then(|| Self(value.to_owned()))
            .ok_or(InvalidName)
    }
}

/// Identifies a FIFO send for duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(value: &str) -> Result<Self, InvalidName> {
        is_token(value)
            .then(|| Self(value.to_owned()))
            .ok_or(InvalidName)
    }

    /// Content-based ids are the message body, which has no character or length rule.
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for DeduplicationId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// `m-<n>`, counting accepted messages over the whole process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: MessageId = MessageId(1);

    /// Returns the current value and moves to the one after it.
    pub fn take_next(&mut self) -> MessageId {
        let current = *self;
        self.0 = self.0.saturating_add(1);
        current
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("m-{}", self.0))
    }
}

/// `r-<n>`, counting deliveries over the whole process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: ReceiptHandle = ReceiptHandle(1);

    /// Returns the current value and moves to the one after it.
    pub fn take_next(&mut self) -> ReceiptHandle {
        let current = *self;
        self.0 = self.0.saturating_add(1);
        current
    }

    /// Accepts only the exact text `Serialize` produces: no sign, no leading zero, no padding.
    /// `None` means the text is not a handle any delivery could have produced.
    pub fn parse(text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(ReceiptHandle)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("r-{}", self.0))
    }
}

/// Position of a message among those ever added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    /// Returns the current value and moves to the one after it.
    pub fn take_next(&mut self) -> SequenceNumber {
        let current = *self;
        self.0 = self.0.saturating_add(1);
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Small = Ranged<1, 3>;

    #[test]
    fn ranged_accepts_exactly_its_bounds() {
        let results: Vec<_> = [-1, 0, 1, 3, 4, i64::MAX]
            .into_iter()
            .map(|v| Small::try_from(v).map(Small::get))
            .collect();
        let rejected = Err(OutOfRange { min: 1, max: 3 });
        assert_eq!(
            results,
            [rejected, rejected, Ok(1), Ok(3), rejected, rejected]
        );
    }

    #[test]
    fn queue_name_syntax() {
        let cases = [
            ("a", true),
            ("A-z_0", true),
            ("a.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("é", false),
            ("a.fifo.fifo", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{}.fifo", "a".repeat(75)), true),
            (&format!("{}.fifo", "a".repeat(76)), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn token_length_and_characters() {
        let cases = [
            ("g", true),
            ("A-z_9", true),
            (&"x".repeat(128), true),
            (&"x".repeat(129), false),
            ("", false),
            ("a.b", false),
        ];
        for (value, valid) in cases {
            assert_eq!(GroupId::parse(value).is_ok(), valid, "{value:?}");
            assert_eq!(DeduplicationId::parse(value).is_ok(), valid, "{value:?}");
        }
    }

    #[test]
    fn receipt_handle_parses_only_canonical_text() {
        let cases = [
            ("r-1", Some(ReceiptHandle(1))),
            ("r-120", Some(ReceiptHandle(120))),
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("r--1", None),
            ("m-1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(ReceiptHandle::parse(text), expected, "{text:?}");
        }
    }

    #[test]
    fn ids_serialize_with_their_prefix() {
        assert_eq!(serde_json::to_string(&MessageId::FIRST).unwrap(), "\"m-1\"");
        assert_eq!(
            serde_json::to_string(&{
                let mut handle = ReceiptHandle::FIRST;
                handle.take_next();
                handle
            })
            .unwrap(),
            "\"r-2\""
        );
    }
}
