//! Validated value types shared by the parser and the queue engine.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::NameProblem;

macro_rules! bounded {
    ($(#[$doc:meta])* $name:ident, $min:expr, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// `None` when `value` is outside the inclusive range of this type.
            pub fn new(value: i64) -> Option<Self> {
                u32::try_from(value)
                    .ok()
                    .filter(|v| ($min..=$max).contains(v))
                    .map(Self)
            }

            pub fn get(self) -> u32 {
                self.0
            }
        }
    };
    ($(#[$doc:meta])* $name:ident, $min:expr, $max:expr, default $default:expr) => {
        bounded!($(#[$doc])* $name, $min, $max);

        impl Default for $name {
            fn default() -> Self {
                Self($default)
            }
        }
    };
}

bounded!(
    /// Seconds a delivered message stays invisible.
    VisibilityTimeout, 0, 43_200, default 30
);
bounded!(
    /// Seconds a sent message stays invisible.
    DelaySeconds, 0, 900, default 0
);
bounded!(
    /// Seconds after which a message is removed.
    RetentionSeconds, 60, 1_209_600, default 345_600
);
bounded!(MaxReceiveCount, 1, 1000);
bounded!(MaxMessages, 1, 10, default 1);
bounded!(TickSeconds, 0, 1_209_600);

/// A point on the virtual clock, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    pub fn plus(self, seconds: u32) -> Time {
        Time(self.0 + u64::from(seconds))
    }

    /// Seconds elapsed since `earlier`, which must not be later than `self`.
    pub fn since(self, earlier: Time) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Counts identifiers handed out so far.
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn advance(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    pub fn issued(&self) -> u64 {
        self.0
    }
}

macro_rules! prefixed_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(u64);

        impl $name {
            pub fn from_counter(n: u64) -> Self {
                Self(n)
            }

            /// Parses the canonical text form only: no sign, no leading zeros.
            pub fn parse(text: &str) -> Option<Self> {
                let digits = text.strip_prefix($prefix)?;
                if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                digits.parse().ok().map(Self)
            }

            pub fn number(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    };
}

prefixed_id!(MessageId, "m-");
prefixed_id!(ReceiptHandle, "r-");

/// Per-FIFO-queue position of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub fn from_counter(n: u64) -> Self {
        Self(n)
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that follows the naming rules of `CreateQueue`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    const SUFFIX: &'static str = ".fifo";

    pub fn new(name: &str) -> Result<Self, NameProblem> {
        let stem = name.strip_suffix(Self::SUFFIX).unwrap_or(name);
        let stem_max = if stem.len() == name.len() { 80 } else { 75 };
        let valid = (1..=stem_max).contains(&stem.len()) && stem.chars().all(is_name_char);
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(NameProblem::Syntax)
        }
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(Self::SUFFIX)
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

macro_rules! token {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(text: &str) -> Option<Self> {
                ((1..=128).contains(&text.len()) && text.chars().all(is_name_char))
                    .then(|| Self(text.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

token!(
    /// Messages of one group are delivered one at a time, in order.
    GroupId
);
token!(DeduplicationId);

/// What a FIFO send is deduplicated on: its explicit id or else its body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn of(id: Option<&DeduplicationId>, body: &Body) -> Self {
        Self(id.map_or(body.as_str(), DeduplicationId::as_str).to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let fifo_max = format!("{}.fifo", "a".repeat(75));
        let fifo_over = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("Jobs_1-x", true),
            ("jobs.fifo", true),
            (long.as_str(), true),
            (fifo_max.as_str(), true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            ("caf\u{e9}", false),
            (too_long.as_str(), false),
            (fifo_over.as_str(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn bounded_values_include_both_ends() {
        assert_eq!(DelaySeconds::new(900).map(DelaySeconds::get), Some(900));
        assert_eq!(DelaySeconds::new(901), None);
        assert_eq!(DelaySeconds::new(-1), None);
        assert_eq!(RetentionSeconds::new(59), None);
        assert_eq!(
            RetentionSeconds::new(60).map(RetentionSeconds::get),
            Some(60)
        );
    }

    #[test]
    fn handles_parse_only_canonical_text() {
        let cases = [
            ("r-1", Some(1)),
            ("r-120", Some(120)),
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("r-+1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(
                ReceiptHandle::parse(text).map(ReceiptHandle::number),
                expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn body_limit_counts_bytes() {
        assert!(Body::new(&"\u{e9}".repeat(512)).is_some());
        assert!(Body::new(&"\u{e9}".repeat(513)).is_none());
        assert!(Body::new("").is_none());
    }
}
