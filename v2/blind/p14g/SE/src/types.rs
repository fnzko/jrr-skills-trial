//! Value types with a validity rule, and the identifiers and clock values of the protocol.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// An integer number of seconds or items that is known to lie in `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Bounded<const MIN: u32, const MAX: u32>(u32);

impl<const MIN: u32, const MAX: u32> Bounded<MIN, MAX> {
    /// For constants; a value out of range fails at compile time.
    pub const fn new_const(value: u32) -> Self {
        assert!(MIN <= value && value <= MAX);
        Self(value)
    }

    /// `None` when `value` is outside the range.
    pub fn new(value: i128) -> Option<Self> {
        u32::try_from(value)
            .ok()
            .filter(|v| (MIN..=MAX).contains(v))
            .map(Self)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl<const MIN: u32, const MAX: u32> From<Bounded<MIN, MAX>> for u32 {
    fn from(value: Bounded<MIN, MAX>) -> Self {
        value.0
    }
}

pub type VisibilityTimeout = Bounded<0, 43_200>;
pub type DelaySeconds = Bounded<0, 900>;
pub type RetentionSeconds = Bounded<60, 1_209_600>;
pub type MaxReceiveCount = Bounded<1, 1000>;
pub type MaxMessages = Bounded<1, 10>;
pub type TickSeconds = Bounded<0, 1_209_600>;

pub const DEFAULT_VISIBILITY_TIMEOUT: VisibilityTimeout = VisibilityTimeout::new_const(30);
pub const DEFAULT_DELAY: DelaySeconds = DelaySeconds::new_const(0);
pub const DEFAULT_RETENTION: RetentionSeconds = RetentionSeconds::new_const(345_600);

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    /// Saturates: the clock is unspecified beyond 4_000_000_000 anyway.
    pub fn plus(self, seconds: impl Into<u32>) -> Time {
        Time(self.0.saturating_add(u64::from(seconds.into())))
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// The name of an existing or creatable queue.
///
/// Ordering and hashing are those of the underlying string, which is what lets maps keyed by
/// `QueueName` be searched with a `&str`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    const FIFO_SUFFIX: &'static str = ".fifo";
    const MAX_LEN: usize = 80;

    pub fn parse(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(Self::FIFO_SUFFIX).unwrap_or(name);
        let valid =
            name.len() <= Self::MAX_LEN && !stem.is_empty() && stem.bytes().all(is_name_byte);
        valid.then(|| Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(Self::FIFO_SUFFIX)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_fifo_id(text: &str) -> bool {
    (1..=128).contains(&text.len()) && text.bytes().all(is_name_byte)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<Self> {
        is_fifo_id(text).then(|| Self(text.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Option<Self> {
        is_fifo_id(text).then(|| Self(text.to_owned()))
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
    pub fn parse(text: &str) -> Option<Self> {
        (1..=1024)
            .contains(&text.len())
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// `r-<n>` with `n` counting deliveries from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        Self(number)
    }

    /// The delivery number when `text` is exactly the canonical spelling of a handle: no sign,
    /// no leading zero, no surrounding space.
    pub fn parse_number(text: &str) -> Option<u64> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if canonical { digits.parse().ok() } else { None }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_syntax() {
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
            ("jobs.FIFO", false),
            ("é", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "{name:?}");
        }
    }

    #[test]
    fn receipt_handles_are_only_the_canonical_spelling() {
        let cases = [
            ("r-1", Some(1)),
            ("r-10", Some(10)),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("m-1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, number) in cases {
            assert_eq!(ReceiptHandle::parse_number(text), number, "{text:?}");
        }
    }

    #[test]
    fn bounded_checks_both_ends() {
        type Small = Bounded<2, 4>;
        assert_eq!(Small::new(1), None);
        assert_eq!(Small::new(2), Some(Small::new_const(2)));
        assert_eq!(Small::new(4), Some(Small::new_const(4)));
        assert_eq!(Small::new(5), None);
        assert_eq!(Small::new(-3), None);
    }

    #[test]
    fn bodies_are_limited_in_bytes_not_characters() {
        assert_eq!(Body::parse(""), None);
        assert_eq!(Body::parse(&"x".repeat(1024)), Some(Body("x".repeat(1024))));
        assert_eq!(Body::parse(&"é".repeat(512)), Some(Body("é".repeat(512))));
        assert_eq!(Body::parse(&"é".repeat(513)), None);
    }
}
