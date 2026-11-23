//! Domain value types. Types with a validity rule are built only through
//! their checked constructors.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// Whole seconds as a span of virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u32);

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    /// Saturates: behavior past the supported clock range is unspecified.
    pub fn plus(self, span: impl Into<Seconds>) -> Time {
        Time(self.0.saturating_add(u64::from(span.into().0)))
    }

    /// Whole seconds from `earlier` to `self`, zero if `earlier` is later.
    pub fn since(self, earlier: Time) -> Seconds {
        Seconds(u32::try_from(self.0.saturating_sub(earlier.0)).unwrap_or(u32::MAX))
    }
}

/// An integer parameter confined to an inclusive range.
pub trait Ranged: Sized {
    const MIN: u32;
    const MAX: u32;
    const NAME: &'static str;

    fn from_checked(value: u32) -> Self;

    fn new(value: i64) -> Option<Self> {
        u32::try_from(value)
            .ok()
            .filter(|v| (Self::MIN..=Self::MAX).contains(v))
            .map(Self::from_checked)
    }
}

macro_rules! ranged {
    ($name:ident, $min:expr, $max:expr, $default:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub struct $name(u32);

        impl $name {
            pub const DEFAULT: $name = $name($default);

            pub fn get(self) -> u32 {
                self.0
            }
        }

        impl Ranged for $name {
            const MIN: u32 = $min;
            const MAX: u32 = $max;
            const NAME: &'static str = stringify!($name);

            fn from_checked(value: u32) -> Self {
                Self(value)
            }
        }
    };
}

macro_rules! as_span {
    ($($name:ident),+) => {
        $(impl From<$name> for Seconds {
            fn from(value: $name) -> Seconds {
                Seconds(value.0)
            }
        })+
    };
}

ranged!(VisibilityTimeout, 0, 43_200, 30);
ranged!(DelaySeconds, 0, 900, 0);
ranged!(RetentionSeconds, 60, 1_209_600, 345_600);
ranged!(MaxReceiveCount, 1, 1000, 1);
ranged!(MaxMessages, 1, 10, 1);
ranged!(TickSeconds, 0, 1_209_600, 0);
as_span!(
    VisibilityTimeout,
    DelaySeconds,
    RetentionSeconds,
    TickSeconds
);

/// How long a duplicate send is suppressed after the original was accepted.
pub const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// `m-<n>`, unique per accepted message in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: MessageId = MessageId(1);

    pub fn next(self) -> MessageId {
        MessageId(self.0 + 1)
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

/// `r-<n>`, unique per delivery in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: ReceiptHandle = ReceiptHandle(1);

    pub fn next(self) -> ReceiptHandle {
        ReceiptHandle(self.0 + 1)
    }

    /// Accepts only the exact text miniq itself produces: `r-` and a decimal
    /// number without sign, leading zeros or surrounding characters.
    pub fn parse(text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return None;
        }
        digits.parse().ok().map(ReceiptHandle)
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

/// Position of a message among all messages added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    pub fn next(self) -> SequenceNumber {
        SequenceNumber(self.0 + 1)
    }
}

const FIFO_SUFFIX: &str = ".fifo";

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that obeys the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub const MAX_LEN: usize = 80;

    pub fn parse(text: &str) -> Option<QueueName> {
        let stem = text.strip_suffix(FIFO_SUFFIX).unwrap_or(text);
        let valid =
            !stem.is_empty() && text.len() <= Self::MAX_LEN && stem.chars().all(is_name_char);
        valid.then(|| QueueName(text.to_owned()))
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

fn is_token(text: &str) -> bool {
    (1..=128).contains(&text.chars().count()) && text.chars().all(is_name_char)
}

/// Identifies the ordering group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<GroupId> {
        is_token(text).then(|| GroupId(text.to_owned()))
    }
}

/// A caller-chosen deduplication id of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Option<DeduplicationId> {
        is_token(text).then(|| DeduplicationId(text.to_owned()))
    }
}

/// A message body of 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body(String);

impl Body {
    pub const MAX_BYTES: usize = 1024;

    pub fn parse(text: &str) -> Option<Body> {
        (1..=Self::MAX_BYTES)
            .contains(&text.len())
            .then(|| Body(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Key of the deduplication table: an explicit id or, failing that, the body.
/// Both share one namespace, so an id equal to a body collides with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl From<DeduplicationId> for DedupKey {
    fn from(id: DeduplicationId) -> Self {
        DedupKey(id.0)
    }
}

impl From<&Body> for DedupKey {
    fn from(body: &Body) -> Self {
        DedupKey(body.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let cases = [
            ("jobs", true),
            ("A-z_9", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            ("é", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{}.fifo", "a".repeat(75)), true),
            (&format!("{}.fifo", "a".repeat(76)), false),
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
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r--1", None),
            ("r-+1", None),
            ("r-", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse(text).map(|h| h.to_string());
            assert_eq!(parsed, expected.map(|n| format!("r-{n}")), "{text:?}");
        }
    }

    #[test]
    fn ranged_values_reject_everything_outside_their_bounds() {
        assert_eq!(
            VisibilityTimeout::new(43_200).map(VisibilityTimeout::get),
            Some(43_200)
        );
        assert_eq!(VisibilityTimeout::new(43_201), None);
        assert_eq!(VisibilityTimeout::new(-1), None);
        assert_eq!(RetentionSeconds::new(59), None);
        assert_eq!(
            RetentionSeconds::new(60).map(RetentionSeconds::get),
            Some(60)
        );
        assert_eq!(MaxReceiveCount::new(0), None);
        assert_eq!(MaxMessages::new(i64::MAX), None);
    }

    #[test]
    fn tokens_and_bodies_are_length_checked() {
        assert!(GroupId::parse("g-1_A").is_some());
        assert!(GroupId::parse("").is_none());
        assert!(GroupId::parse("a b").is_none());
        assert!(GroupId::parse(&"g".repeat(128)).is_some());
        assert!(GroupId::parse(&"g".repeat(129)).is_none());
        assert!(Body::parse("").is_none());
        assert!(Body::parse(&"x".repeat(1024)).is_some());
        assert!(Body::parse(&"x".repeat(1025)).is_none());
        assert!(Body::parse(&"é".repeat(512)).is_some());
        assert!(Body::parse(&"é".repeat(513)).is_none());
    }
}
