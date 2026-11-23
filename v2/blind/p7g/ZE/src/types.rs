//! Value types with validity rules. Fields are private so that every value
//! was produced by a check.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// A point on the virtual clock, in seconds since start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    pub fn after(self, seconds: u32) -> Time {
        Time(self.0.saturating_add(u64::from(seconds)))
    }

    /// Seconds elapsed since `earlier`; zero if `earlier` is not earlier.
    pub fn since(self, earlier: Time) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// An integer parameter with an inclusive range.
pub trait Bounded: Sized {
    const MIN: u32;
    const MAX: u32;

    fn new(value: i64) -> Option<Self>;
}

macro_rules! bounded {
    ($(#[$meta:meta])* $name:ident, $min:expr, $max:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub struct $name(u32);

        impl $name {
            pub fn get(self) -> u32 {
                self.0
            }
        }

        impl Bounded for $name {
            const MIN: u32 = $min;
            const MAX: u32 = $max;

            fn new(value: i64) -> Option<Self> {
                u32::try_from(value)
                    .ok()
                    .filter(|v| (Self::MIN..=Self::MAX).contains(v))
                    .map(Self)
            }
        }
    };
}

bounded!(VisibilityTimeout, 0, 43_200);
bounded!(
    #[derive(Default)]
    DelaySeconds,
    0,
    900
);
bounded!(RetentionSeconds, 60, 1_209_600);
bounded!(MaxReceiveCount, 1, 1000);
bounded!(MaxMessages, 1, 10);
bounded!(TickSeconds, 0, 1_209_600);

impl Default for VisibilityTimeout {
    fn default() -> Self {
        Self(30)
    }
}

impl Default for RetentionSeconds {
    fn default() -> Self {
        Self(345_600)
    }
}

impl Default for MaxMessages {
    fn default() -> Self {
        Self(1)
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

const FIFO_SUFFIX: &str = ".fifo";

/// A queue name that follows the naming rules. Lookups of existing queues
/// take plain `&str` instead, because any string may be asked for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(candidate: &str) -> Option<Self> {
        let stem = candidate.strip_suffix(FIFO_SUFFIX);
        let (body, longest) = match stem {
            Some(stem) => (stem, 75),
            None => (candidate, 80),
        };
        let valid = (1..=longest).contains(&body.len()) && body.chars().all(is_name_char);
        valid.then(|| Self(candidate.to_owned()))
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_token(candidate: &str) -> bool {
    (1..=128).contains(&candidate.len()) && candidate.chars().all(is_name_char)
}

/// A FIFO message group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(candidate: &str) -> Option<Self> {
        is_token(candidate).then(|| Self(candidate.to_owned()))
    }
}

/// A caller-chosen deduplication id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(candidate: &str) -> Option<Self> {
        is_token(candidate).then(|| Self(candidate.to_owned()))
    }
}

/// What a FIFO send is deduplicated on: its deduplication id, else its body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationKey(String);

impl DeduplicationKey {
    pub fn from_id(id: DeduplicationId) -> Self {
        Self(id.0)
    }

    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

impl Borrow<str> for DeduplicationKey {
    fn borrow(&self) -> &str {
        &self.0
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

impl ReceiptHandle {
    /// Accepts only the canonical spelling `r-<n>`: no sign, no leading zero,
    /// no surrounding characters.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
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

/// Hands out process-wide message ids and receipt handles.
#[derive(Debug, Default)]
pub struct IdGenerator {
    messages: u64,
    receipts: u64,
}

impl IdGenerator {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    pub fn has_issued(&self, handle: ReceiptHandle) -> bool {
        (1..=self.receipts).contains(&handle.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_syntax() {
        let long_plain = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_plain = "a".repeat(81);
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let cases: [(&str, bool); 12] = [
            ("jobs", true),
            ("Jobs_1-x", true),
            ("jobs.fifo", true),
            (&long_plain, true),
            (&long_fifo, true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("jobs fifo", false),
            (&too_long_plain, false),
            (&too_long_fifo, false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "{name:?}");
        }
    }

    #[test]
    fn receipt_handles_must_be_canonical() {
        let cases = [
            ("r-1", Some(1)),
            ("r-120", Some(120)),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r--1", None),
            ("r-", None),
            ("1", None),
            ("r-99999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(
                ReceiptHandle::parse(text),
                expected.map(ReceiptHandle),
                "{text:?}"
            );
        }
    }

    #[test]
    fn bounded_values_enforce_inclusive_ranges() {
        assert_eq!(DelaySeconds::new(900).map(DelaySeconds::get), Some(900));
        assert_eq!(DelaySeconds::new(901), None);
        assert_eq!(DelaySeconds::new(-1), None);
        assert_eq!(RetentionSeconds::new(59), None);
        assert_eq!(
            RetentionSeconds::new(60).map(RetentionSeconds::get),
            Some(60)
        );
        assert_eq!(MaxMessages::new(i64::MAX), None);
    }

    #[test]
    fn tokens_are_one_to_128_name_characters() {
        assert!(GroupId::parse(&"g".repeat(128)).is_some());
        assert!(GroupId::parse(&"g".repeat(129)).is_none());
        assert!(GroupId::parse("").is_none());
        assert!(DeduplicationId::parse("a.b").is_none());
        assert!(DeduplicationId::parse("a-b_C9").is_some());
    }

    #[test]
    fn ids_count_from_one_and_handles_are_remembered() {
        let mut ids = IdGenerator::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        let first = ids.next_receipt();
        assert!(ids.has_issued(first));
        assert!(!ids.has_issued(ReceiptHandle(2)));
    }
}
