//! Scalar types with validity rules. Fields are private so that a value of
//! these types is always inside the range or syntax the protocol allows.

use std::borrow::Borrow;
use std::fmt;
use std::num::NonZeroU64;

use serde::{Serialize, Serializer};

/// A value rejected by a syntax rule; `expected` says what would be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidValue {
    expected: &'static str,
}

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected {}", self.expected)
    }
}

impl std::error::Error for InvalidValue {}

/// An integer outside the inclusive range of its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange {
    min: u32,
    max: u32,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected an integer from {} to {}", self.min, self.max)
    }
}

impl std::error::Error for OutOfRange {}

fn check_range(value: i64, min: u32, max: u32) -> Result<u32, OutOfRange> {
    u32::try_from(value)
        .ok()
        .filter(|v| (min..=max).contains(v))
        .ok_or(OutOfRange { min, max })
}

/// A duration in seconds within `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Seconds<const MIN: u32, const MAX: u32>(u32);

impl<const MIN: u32, const MAX: u32> Seconds<MIN, MAX> {
    pub fn new(value: i64) -> Result<Self, OutOfRange> {
        check_range(value, MIN, MAX).map(Self)
    }

    /// For compile-time constants; an out-of-range value fails the build.
    pub const fn constant(value: u32) -> Self {
        assert!(MIN <= value && value <= MAX, "constant is within the range");
        Self(value)
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
}

/// A count within `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Count<const MIN: u32, const MAX: u32>(u32);

impl<const MIN: u32, const MAX: u32> Count<MIN, MAX> {
    pub fn new(value: i64) -> Result<Self, OutOfRange> {
        check_range(value, MIN, MAX).map(Self)
    }

    pub const fn constant(value: u32) -> Self {
        assert!(MIN <= value && value <= MAX, "constant is within the range");
        Self(value)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

pub type VisibilityTimeout = Seconds<0, 43_200>;
pub type Delay = Seconds<0, 900>;
pub type Retention = Seconds<60, 1_209_600>;
pub type TickSeconds = Seconds<0, 1_209_600>;
pub type MaxReceiveCount = Count<1, 1000>;
pub type MaxMessages = Count<1, 10>;

/// A point on the virtual clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Instant(u64);

impl Instant {
    pub const START: Self = Self(0);

    pub fn plus<const MIN: u32, const MAX: u32>(self, duration: Seconds<MIN, MAX>) -> Self {
        Self(self.0.saturating_add(u64::from(duration.0)))
    }
}

/// `m-<n>`, counting accepted messages over all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    pub fn succ(self) -> Self {
        Self(self.0.saturating_add(1))
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

/// The position of a message in a FIFO queue's history, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    pub fn succ(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// `r-<n>`, counting deliveries over all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(NonZeroU64);

impl ReceiptHandle {
    /// The handle issued by the delivery at zero-based position `index`.
    pub fn at_index(index: usize) -> Self {
        let index = u64::try_from(index).expect("usize is at most 64 bits wide");
        Self(NonZeroU64::MIN.saturating_add(index))
    }

    /// The zero-based delivery position this handle names, if it fits `usize`.
    pub fn index(self) -> Option<usize> {
        usize::try_from(self.0.get() - 1).ok()
    }

    /// Accepts exactly the strings `Display` produces: `r-` and a decimal
    /// number of at least 1 without sign or leading zeros.
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

/// Identifies one incarnation of a queue; a queue recreated under an old name
/// gets a new id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueId(u64);

impl QueueId {
    pub const FIRST: Self = Self(0);

    pub fn succ(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// The position at which a message entered one queue; unique within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arrival(u64);

impl Arrival {
    pub const FIRST: Self = Self(0);

    pub fn succ(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

const FIFO_SUFFIX: &str = ".fifo";

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, InvalidValue> {
        let stem = text.strip_suffix(FIFO_SUFFIX).unwrap_or(text);
        let max_stem = 80
            - if stem.len() < text.len() {
                FIFO_SUFFIX.len()
            } else {
                0
            };
        if stem.is_empty() || stem.len() > max_stem || !stem.bytes().all(is_name_char) {
            return Err(InvalidValue {
                expected: "1 to 80 characters from A-Z a-z 0-9 _ -, optionally followed by .fifo",
            });
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo_name(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn parse_token(text: &str) -> Result<String, InvalidValue> {
    if text.is_empty() || text.len() > 128 || !text.bytes().all(is_name_char) {
        return Err(InvalidValue {
            expected: "1 to 128 characters from A-Z a-z 0-9 _ -",
        });
    }
    Ok(text.to_owned())
}

/// Identifies the ordering group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, InvalidValue> {
        parse_token(text).map(Self)
    }
}

/// The caller-chosen identity of a FIFO send, used to drop duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Result<Self, InvalidValue> {
        parse_token(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A message body of 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn parse(text: &str) -> Result<Self, InvalidValue> {
        if text.is_empty() || text.len() > 1024 {
            return Err(InvalidValue {
                expected: "a string of 1 to 1024 bytes in UTF-8",
            });
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_syntax() {
        let long = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let too_long = "a".repeat(81);
        let accepted = [
            "jobs",
            "A-z_9",
            "a",
            long.as_str(),
            "x.fifo",
            long_fifo.as_str(),
        ];
        for name in accepted {
            assert_eq!(
                QueueName::parse(name).map(|n| n.0),
                Ok(name.to_owned()),
                "{name}"
            );
        }
        let rejected = [
            "",
            ".fifo",
            "a.b",
            "a.fifo.fifo",
            "a b",
            "é",
            "jobs\n",
            "a.FIFO",
            too_long.as_str(),
            too_long_fifo.as_str(),
        ];
        for name in rejected {
            assert!(QueueName::parse(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn fifo_names_are_recognised_by_suffix() {
        assert!(QueueName::parse("a.fifo").unwrap().is_fifo_name());
        assert!(!QueueName::parse("afifo").unwrap().is_fifo_name());
    }

    #[test]
    fn ranges_are_inclusive_and_reject_out_of_range_integers() {
        assert_eq!(
            VisibilityTimeout::new(0),
            Ok(VisibilityTimeout::constant(0))
        );
        assert_eq!(
            VisibilityTimeout::new(43_200),
            Ok(VisibilityTimeout::constant(43_200))
        );
        for value in [-1, 43_201, i64::MAX, i64::MIN, i64::from(u32::MAX) + 1] {
            assert!(VisibilityTimeout::new(value).is_err(), "{value}");
        }
        assert!(Retention::new(59).is_err());
        assert!(Retention::new(60).is_ok());
        assert!(MaxMessages::new(0).is_err());
        assert!(MaxMessages::new(10).is_ok());
    }

    #[test]
    fn receipt_handles_parse_only_canonical_text() {
        assert_eq!(
            ReceiptHandle::parse("r-1"),
            Some(ReceiptHandle::at_index(0))
        );
        assert_eq!(
            ReceiptHandle::parse("r-120"),
            Some(ReceiptHandle::at_index(119))
        );
        for text in [
            "",
            "r-",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-+1",
            "r--1",
            "r-1.0",
            "m-1",
            "1",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn receipt_handle_index_round_trips() {
        assert_eq!(ReceiptHandle::at_index(41).index(), Some(41));
        assert_eq!(ReceiptHandle::at_index(41).to_string(), "r-42");
    }

    #[test]
    fn identifiers_serialize_as_protocol_text() {
        assert_eq!(
            serde_json::to_string(&MessageId::FIRST.succ()).unwrap(),
            "\"m-2\""
        );
        assert_eq!(
            serde_json::to_string(&ReceiptHandle::at_index(0)).unwrap(),
            "\"r-1\""
        );
        assert_eq!(serde_json::to_string(&SequenceNumber::FIRST).unwrap(), "1");
    }

    #[test]
    fn tokens_and_bodies_enforce_length_and_characters() {
        assert!(GroupId::parse("g-1_A").is_ok());
        assert!(GroupId::parse("").is_err());
        assert!(GroupId::parse(&"g".repeat(129)).is_err());
        assert!(DeduplicationId::parse(&"d".repeat(128)).is_ok());
        assert!(DeduplicationId::parse("a b").is_err());
        assert!(Body::parse("").is_err());
        assert!(Body::parse(&"b".repeat(1024)).is_ok());
        assert!(Body::parse(&"b".repeat(1025)).is_err());
        // 513 two-byte characters are 1026 bytes: the limit counts bytes.
        assert!(Body::parse(&"é".repeat(513)).is_err());
        assert!(Body::parse(&"é".repeat(512)).is_ok());
    }

    #[test]
    fn instants_add_durations() {
        let later = Instant::START
            .plus(Delay::constant(5))
            .plus(Delay::constant(7));
        assert_eq!(serde_json::to_string(&later).unwrap(), "12");
    }
}
