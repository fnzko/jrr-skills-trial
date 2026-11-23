//! Value types with validity rules, and the identifiers handed out by the service.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// An integer parameter with an inclusive range.
pub trait Bounded: Sized {
    /// `None` when `value` is outside the range.
    fn from_i64(value: i64) -> Option<Self>;
}

macro_rules! bounded_int {
    ($(#[$meta:meta])* $name:ident, $min:expr, $max:expr, default $default:expr) => {
        bounded_int!($(#[$meta])* $name, $min, $max);

        impl Default for $name {
            fn default() -> Self {
                Self($default)
            }
        }
    };
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
            fn from_i64(value: i64) -> Option<Self> {
                u32::try_from(value)
                    .ok()
                    .filter(|v| ($min..=$max).contains(v))
                    .map(Self)
            }
        }
    };
}

bounded_int!(
    /// Seconds a delivered message stays invisible.
    VisibilityTimeout, 0, 43_200, default 30
);
bounded_int!(
    /// Seconds before a sent message becomes visible.
    DelaySeconds, 0, 900, default 0
);
bounded_int!(
    /// Seconds a message lives, counted from `SentAt`.
    RetentionSeconds, 60, 1_209_600, default 345_600
);
bounded_int!(
    /// Deliveries after which a message moves to the dead-letter queue.
    MaxReceiveCount, 1, 1000
);
bounded_int!(
    /// Upper bound of one `ReceiveMessage`.
    MaxMessages, 1, 10, default 1
);
bounded_int!(
    /// Amount of virtual time one `Tick` advances.
    TickSeconds, 0, 1_209_600
);

/// A point on the virtual clock, in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const ZERO: Self = Self(0);

    /// The instant `seconds` later. Saturates: behavior that far out is unspecified.
    pub fn after(self, seconds: u32) -> Self {
        Self(self.0.saturating_add(u64::from(seconds)))
    }

    pub fn advance(self, ticks: TickSeconds) -> Self {
        self.after(ticks.get())
    }
}

/// `m-<n>`, unique over all queues of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

/// `r-<n>`, one per delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

/// Position of a message among the additions to one FIFO queue, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
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

/// Hands out message ids and receipt handles in the order the spec numbers them.
#[derive(Debug, Default)]
pub struct IdAllocator {
    messages: u64,
    handles: u64,
}

impl IdAllocator {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_handle(&mut self) -> ReceiptHandle {
        self.handles += 1;
        ReceiptHandle(self.handles)
    }

    /// The handle `text` spells, if this allocator issued it. Only the canonical
    /// spelling `r-<n>` (no sign, no leading zero) counts.
    pub fn issued_handle(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (1..=self.handles)
            .contains(&number)
            .then_some(ReceiptHandle(number))
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

const FIFO_SUFFIX: &str = ".fifo";

/// The name of a queue. Any string can be looked up; only creation requires
/// [`QueueName::has_valid_syntax`]. Orders by byte value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: String) -> Self {
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn has_valid_syntax(&self) -> bool {
        let (stem, longest) = match self.0.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (self.0.as_str(), 80),
        };
        (1..=longest).contains(&stem.len()) && stem.bytes().all(is_name_byte)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

macro_rules! message_token {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        pub struct $name(String);

        impl $name {
            /// `None` unless `value` is 1 to 128 characters of `A-Z a-z 0-9 _ -`.
            pub fn new(value: String) -> Option<Self> {
                let valid = (1..=128).contains(&value.len()) && value.bytes().all(is_name_byte);
                valid.then_some(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

message_token!(
    /// Messages of one group of a FIFO queue are delivered one at a time, in order.
    GroupId
);
message_token!(
    /// Caller-chosen identity of a FIFO send, for deduplication.
    DeduplicationId
);

/// The text of a message: 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body(String);

impl Body {
    pub fn new(text: String) -> Option<Self> {
        (1..=1024).contains(&text.len()).then_some(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_ranges_are_inclusive() {
        assert_eq!(VisibilityTimeout::from_i64(0), Some(VisibilityTimeout(0)));
        assert_eq!(
            VisibilityTimeout::from_i64(43_200).map(VisibilityTimeout::get),
            Some(43_200)
        );
        assert_eq!(VisibilityTimeout::from_i64(43_201), None);
        assert_eq!(VisibilityTimeout::from_i64(-1), None);
        assert_eq!(RetentionSeconds::from_i64(59), None);
        assert_eq!(
            RetentionSeconds::from_i64(60).map(RetentionSeconds::get),
            Some(60)
        );
        assert_eq!(MaxReceiveCount::from_i64(0), None);
        assert_eq!(
            MaxReceiveCount::from_i64(1000).map(MaxReceiveCount::get),
            Some(1000)
        );
        assert_eq!(MaxReceiveCount::from_i64(i64::MAX), None);
    }

    #[test]
    fn queue_name_syntax() {
        let valid = |s: &str| QueueName::new(s.to_owned()).has_valid_syntax();
        assert!(valid("jobs"));
        assert!(valid("A_b-9"));
        assert!(valid("jobs.fifo"));
        assert!(valid(&"a".repeat(80)));
        assert!(valid(&format!("{}.fifo", "a".repeat(75))));
        assert!(!valid(""));
        assert!(!valid(".fifo"));
        assert!(!valid(&"a".repeat(81)));
        assert!(!valid(&format!("{}.fifo", "a".repeat(76))));
        assert!(!valid("a.b"));
        assert!(!valid("a.fifo.fifo"));
        assert!(!valid("jobs fifo"));
        assert!(!valid("caf\u{e9}"));
    }

    #[test]
    fn tokens_and_bodies_check_length_and_alphabet() {
        assert!(GroupId::new("g-1_A".to_owned()).is_some());
        assert!(GroupId::new(String::new()).is_none());
        assert!(GroupId::new("a".repeat(128)).is_some());
        assert!(GroupId::new("a".repeat(129)).is_none());
        assert!(DeduplicationId::new("a b".to_owned()).is_none());
        assert!(Body::new("x".repeat(1024)).is_some());
        assert!(Body::new("x".repeat(1025)).is_none());
        assert!(Body::new("\u{e9}".repeat(512)).is_some());
        assert!(Body::new("\u{e9}".repeat(513)).is_none());
        assert!(Body::new(String::new()).is_none());
    }

    #[test]
    fn only_canonical_issued_handles_parse() {
        let mut ids = IdAllocator::default();
        assert_eq!(ids.issued_handle("r-1"), None);
        let first = ids.next_handle();
        let second = ids.next_handle();
        assert_eq!(ids.issued_handle("r-1"), Some(first));
        assert_eq!(ids.issued_handle("r-2"), Some(second));
        for text in [
            "r-3", "r-01", "R-1", "r-1 ", " r-1", "r-", "r-+1", "r-0", "r--1", "1", "m-1",
        ] {
            assert_eq!(ids.issued_handle(text), None, "{text:?}");
        }
        assert_eq!(ids.issued_handle("r-99999999999999999999999"), None);
    }

    #[test]
    fn ids_serialize_with_their_prefix() {
        let mut ids = IdAllocator::default();
        assert_eq!(
            serde_json::to_string(&ids.next_message()).unwrap(),
            "\"m-1\""
        );
        assert_eq!(
            serde_json::to_string(&ids.next_handle()).unwrap(),
            "\"r-1\""
        );
        assert_eq!(
            serde_json::to_string(&SequenceNumber::FIRST.next()).unwrap(),
            "2"
        );
    }

    #[test]
    fn timestamps_add_without_overflow() {
        assert_eq!(Timestamp::ZERO.after(5).after(7), Timestamp::ZERO.after(12));
        assert!(Timestamp::ZERO.after(u32::MAX).after(u32::MAX) > Timestamp::ZERO);
    }
}
