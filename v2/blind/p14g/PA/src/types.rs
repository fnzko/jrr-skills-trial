//! Validated value types shared by the whole crate.
//!
//! Types with a validity rule keep their field private and are built through
//! a constructor that returns `None` when the rule is broken. They implement
//! `Serialize` only: nothing in the protocol deserializes them directly.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// A value of the virtual clock, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    /// The instant `seconds` after `self`. The result saturates: behavior
    /// past 4000000000 is unspecified and must merely not panic.
    pub fn after(self, seconds: u64) -> Self {
        Self(self.0.saturating_add(seconds))
    }

    /// Whole seconds from `earlier` to `self`, 0 if `earlier` is later.
    pub fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

macro_rules! bounded {
    ($(#[$meta:meta])* $name:ident, $min:expr, $max:expr, default $default:expr) => {
        bounded!($(#[$meta])* $name, $min, $max);

        impl Default for $name {
            fn default() -> Self {
                Self($default)
            }
        }
    };
    ($(#[$meta:meta])* $name:ident, $min:expr, $max:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// `Some` when `value` lies in the inclusive range of this type.
            pub fn new(value: u64) -> Option<Self> {
                u32::try_from(value)
                    .ok()
                    .filter(|v| ($min..=$max).contains(v))
                    .map(Self)
            }

            pub fn get(self) -> u64 {
                u64::from(self.0)
            }
        }
    };
}

bounded!(
    /// Seconds a delivered message stays invisible.
    VisibilityTimeout,
    0,
    43_200,
    default 30
);
bounded!(
    /// Seconds a sent message stays invisible before its first delivery.
    DelaySeconds,
    0,
    900,
    default 0
);
bounded!(
    /// Seconds a message lives in a queue, counted from `SentAt`.
    RetentionSeconds,
    60,
    1_209_600,
    default 345_600
);
bounded!(
    /// Deliveries after which a message is moved to the dead-letter queue.
    MaxReceiveCount,
    1,
    1000
);
bounded!(
    /// Number of messages one `ReceiveMessage` may deliver.
    MaxMessages,
    1,
    10,
    default 1
);
bounded!(
    /// Seconds one `Tick` advances the clock.
    TickSeconds,
    0,
    1_209_600
);

/// The name of a queue: 1 to 80 characters of `A-Z a-z 0-9 _ -`, optionally
/// followed by the suffix `.fifo` (then 1 to 75 such characters precede it).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueueName(String);

const FIFO_SUFFIX: &str = ".fifo";

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

impl QueueName {
    pub fn new(name: &str) -> Option<Self> {
        let valid = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (1..=75).contains(&stem.len()) && stem.chars().all(is_name_char),
            None => (1..=80).contains(&name.len()) && name.chars().all(is_name_char),
        };
        valid.then(|| Self(name.to_owned()))
    }

    /// Whether the name ends in `.fifo`, which only FIFO queues may carry.
    pub fn has_fifo_suffix(name: &str) -> bool {
        name.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// Lets maps keyed by `QueueName` be searched with a plain `&str`. Hash and
// ordering of the newtype equal those of its string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_token(text: &str) -> bool {
    (1..=128).contains(&text.len()) && text.chars().all(is_name_char)
}

/// The group of a FIFO message: 1 to 128 characters of `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(text: &str) -> Option<Self> {
        is_token(text).then(|| Self(text.to_owned()))
    }
}

/// An explicit deduplication id: same character rules as [`GroupId`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn new(text: &str) -> Option<Self> {
        is_token(text).then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A message body: 1 to 1024 bytes of UTF-8.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

/// `m-<n>`, counted over all messages the process has accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// `r-<n>`, counted over all deliveries the process has made.
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
pub struct ReceiptHandleIssuer {
    issued: u64,
}

impl ReceiptHandleIssuer {
    pub fn issue(&mut self) -> ReceiptHandle {
        self.issued += 1;
        ReceiptHandle(self.issued)
    }

    /// The handle written as `text`, if this issuer issued it. Only the
    /// canonical spelling matches: `r-01`, `R-1` and `r-1 ` were never issued.
    pub fn recognize(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (number <= self.issued).then_some(ReceiptHandle(number))
    }
}

/// Position of a message in the arrival order of its queue, starting at 1.
/// In a FIFO queue it is also the `SequenceNumber` the client sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let stem_75 = format!("{}.fifo", "a".repeat(75));
        let stem_76 = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A_z-9", true),
            (long.as_str(), true),
            (too_long.as_str(), false),
            ("", false),
            ("jobs.fifo", true),
            (stem_75.as_str(), true),
            (stem_76.as_str(), false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("jobs.fifo.fifo", false),
            ("j\u{f6}bs", false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_some(), valid, "{name:?}");
        }
    }

    #[test]
    fn bounded_ranges_are_inclusive() {
        let visibility = |v| VisibilityTimeout::new(v).map(VisibilityTimeout::get);
        assert_eq!(visibility(0), Some(0));
        assert_eq!(visibility(43_200), Some(43_200));
        assert_eq!(visibility(43_201), None);
        let retention = |v| RetentionSeconds::new(v).map(RetentionSeconds::get);
        assert_eq!(retention(59), None);
        assert_eq!(retention(60), Some(60));
        let max = |v| MaxMessages::new(v).map(MaxMessages::get);
        assert_eq!(max(0), None);
        assert_eq!(max(u64::MAX), None);
    }

    #[test]
    fn body_limit_counts_bytes() {
        let accepted_len = |text: &str| Body::new(text).map(|body| body.as_str().len());
        assert_eq!(accepted_len(""), None);
        assert_eq!(accepted_len(&"a".repeat(1024)), Some(1024));
        assert_eq!(accepted_len(&"a".repeat(1025)), None);
        // 512 two-byte characters fit, 513 do not.
        assert_eq!(accepted_len(&"\u{e9}".repeat(512)), Some(1024));
        assert_eq!(accepted_len(&"\u{e9}".repeat(513)), None);
    }

    #[test]
    fn handles_must_be_canonical_and_issued() {
        let mut issuer = ReceiptHandleIssuer::default();
        let first = issuer.issue();
        issuer.issue();
        assert_eq!(issuer.recognize("r-1"), Some(first));
        for text in [
            "r-3",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            "r-+1",
            "r-",
            "1",
            "r-99999999999999999999",
        ] {
            assert_eq!(issuer.recognize(text), None, "{text:?}");
        }
    }
}
