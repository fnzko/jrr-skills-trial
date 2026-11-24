//! Validated value types shared by the request parser and the queue state.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// An integer known to lie in `MIN..=MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Bounded<const MIN: u32, const MAX: u32>(u32);

impl<const MIN: u32, const MAX: u32> Bounded<MIN, MAX> {
    pub fn new(value: i64) -> Option<Self> {
        u32::try_from(value)
            .ok()
            .filter(|value| (MIN..=MAX).contains(value))
            .map(Self)
    }

    /// Builds a value that is checked at compile time when used in a `const`.
    pub const fn constant(value: u32) -> Self {
        assert!(MIN <= value && value <= MAX, "constant out of range");
        Self(value)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

pub type VisibilityTimeout = Bounded<0, 43_200>;
pub type DelaySeconds = Bounded<0, 900>;
pub type RetentionSeconds = Bounded<60, 1_209_600>;
pub type MaxReceiveCount = Bounded<1, 1000>;
pub type MaxMessages = Bounded<1, 10>;
pub type TickSeconds = Bounded<0, 1_209_600>;

/// A point on the virtual clock, in seconds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Saturates: behavior past the clock range of the spec is unspecified.
    pub fn after(self, seconds: u32) -> Self {
        Self(self.0.saturating_add(u64::from(seconds)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

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

/// Issues the process-wide message ids and receipt handles.
#[derive(Debug, Default)]
pub struct IdSource {
    messages: u64,
    receipts: u64,
}

impl IdSource {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    /// Returns the handle written as `text` if this source has issued it. Only the
    /// canonical spelling `r-<n>` counts: no sign, no leading zeros, no padding.
    pub fn issued_receipt(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        let number: u64 = digits.parse().ok().filter(|_| canonical)?;
        (1..=self.receipts)
            .contains(&number)
            .then_some(ReceiptHandle(number))
    }
}

/// Hands out the per-queue sequence numbers of a FIFO queue.
#[derive(Debug, Default)]
pub struct SequenceCounter(u64);

impl SequenceCounter {
    pub fn issue(&mut self) -> SequenceNumber {
        self.0 += 1;
        SequenceNumber(self.0)
    }
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

macro_rules! token_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 1 to 128 characters from `A-Z a-z 0-9 _ -`.
            pub fn new(text: &str) -> Option<Self> {
                ((1..=128).contains(&text.len()) && text.bytes().all(is_token_byte))
                    .then(|| Self(text.to_owned()))
            }
        }
    };
}

token_type!(
    /// Messages of one group in a FIFO queue are delivered one at a time, in order.
    GroupId
);
token_type!(
    /// Caller-chosen key that makes repeated FIFO sends collapse into one message.
    DeduplicationId
);

impl DeduplicationId {
    pub fn into_string(self) -> String {
        self.0
    }
}

/// A message body of 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn new(text: &str) -> Option<Self> {
        (1..=1024)
            .contains(&text.len())
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const FIFO_SUFFIX: &str = ".fifo";

/// The name of an existing or new queue, following the naming rules of CreateQueue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(text: &str) -> Option<Self> {
        let (stem, limit) = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (text, 80),
        };
        ((1..=limit).contains(&stem.len()) && stem.bytes().all(is_token_byte))
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Consistent with the derived `Ord` and `Hash`, which both delegate to the inner string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
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
            ("A-z_0", true),
            ("", false),
            ("a b", false),
            ("a.b", false),
            ("é", false),
            (".fifo", false),
            ("x.fifo", true),
            ("x.fifo.fifo", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_some(), valid, "{name}");
        }
    }

    #[test]
    fn bounded_checks_both_ends_and_negatives() {
        assert_eq!(
            VisibilityTimeout::new(43_200).map(Bounded::get),
            Some(43_200)
        );
        assert_eq!(VisibilityTimeout::new(43_201), None);
        assert_eq!(VisibilityTimeout::new(-1), None);
        assert_eq!(RetentionSeconds::new(59), None);
        assert_eq!(RetentionSeconds::new(60).map(Bounded::get), Some(60));
    }

    #[test]
    fn only_canonical_issued_receipts_are_accepted() {
        let mut ids = IdSource::default();
        let first = ids.next_receipt();
        ids.next_receipt();
        assert_eq!(ids.issued_receipt("r-1"), Some(first));
        for text in [
            "r-3", "r-0", "r-01", "R-1", "r-1 ", "r-+1", "r-", "1", "r--1",
        ] {
            assert_eq!(ids.issued_receipt(text), None, "{text:?}");
        }
    }

    #[test]
    fn tokens_reject_empty_and_long_text() {
        assert_eq!(GroupId::new("g-1"), Some(GroupId("g-1".to_owned())));
        assert_eq!(GroupId::new(""), None);
        assert_eq!(GroupId::new(&"g".repeat(129)), None);
        assert_eq!(DeduplicationId::new("a b"), None);
        assert_eq!(Body::new(""), None);
        assert_eq!(Body::new(&"é".repeat(513)), None);
        assert_eq!(Body::new(&"é".repeat(512)), Some(Body("é".repeat(512))));
    }
}
