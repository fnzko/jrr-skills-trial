//! Small value types shared by the queue model and the protocol layers.

use std::fmt;
use std::ops::Add;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A duration on the virtual clock, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u64);

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(pub u64);

impl Add<Seconds> for Time {
    type Output = Time;

    fn add(self, rhs: Seconds) -> Time {
        Time(self.0 + rhs.0)
    }
}

/// `m-<n>`, unique over the whole process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

/// `r-<n>`, one per delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

/// Position of a message among those ever added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(pub u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    /// Returns the current number and moves on to the next.
    pub fn take(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
    }
}

macro_rules! serialize_as_display {
    ($($ty:ty),*) => {$(
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{}", Self::PREFIX, self.0)
            }
        }

        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    )*};
}

impl MessageId {
    const PREFIX: &'static str = "m";
}

impl ReceiptHandle {
    const PREFIX: &'static str = "r";
}

serialize_as_display!(MessageId, ReceiptHandle);

/// Hands out the process-wide message ids and receipt handles.
#[derive(Debug, Default)]
pub struct IdGenerator {
    messages: u64,
    handles: u64,
}

impl IdGenerator {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_handle(&mut self) -> ReceiptHandle {
        self.handles += 1;
        ReceiptHandle(self.handles)
    }

    /// Recognizes a handle this generator has issued, by its exact text.
    pub fn parse_handle(&self, text: &str) -> Result<ReceiptHandle, Error> {
        text.strip_prefix("r-")
            .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok())
            .filter(|&n| n <= self.handles)
            .map(ReceiptHandle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }
}

/// A queue name: 1 to 80 characters of `A-Z a-z 0-9 _ -`, or 1 to 75 of them plus `.fifo`.
///
/// Ordering is by bytes, which is the order `ListQueues` reports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueName(String);

const FIFO_SUFFIX: &str = ".fifo";

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_token(text: &str, max_chars: usize) -> bool {
    // Token characters are ASCII, so the byte length is the character count.
    (1..=max_chars).contains(&text.len()) && text.chars().all(is_token_char)
}

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let stem = text.strip_suffix(FIFO_SUFFIX);
        let valid = match stem {
            Some(stem) => is_token(stem, 75),
            None => is_token(text, 80),
        };
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue name {text:?} breaks the naming rules"),
            ))
        }
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Serialize for QueueName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Names the ordering group of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_id("GroupId", text).map(Self)
    }
}

/// Client-chosen identity of a FIFO send, used to detect duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_id("DeduplicationId", text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn parse_id(parameter: &str, text: &str) -> Result<String, Error> {
    if is_token(text, 128) {
        Ok(text.to_owned())
    } else {
        Err(Error::invalid_value(
            parameter,
            "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_stem = "a".repeat(75);
        let long_plain = "a".repeat(80);
        let cases: &[(&str, bool)] = &[
            ("jobs", true),
            ("Jobs_1-x", true),
            ("jobs.fifo", true),
            (&long_plain, true),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_plain}x"), false),
            (&format!("{long_stem}a.fifo"), false),
            ("", false),
            (".fifo", false),
            ("jobs.fifo.fifo", false),
            ("a.b", false),
            ("a b", false),
            ("jöbs", false),
            ("jobs.FIFO", false),
        ];
        for &(name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn queue_names_sort_by_bytes() {
        let mut names: Vec<_> = ["a_", "a-", "aB", "a1", "ab", "a.fifo"]
            .iter()
            .map(|n| QueueName::parse(n).unwrap())
            .collect();
        names.sort();
        let sorted: Vec<_> = names.iter().map(QueueName::as_str).collect();
        assert_eq!(sorted, ["a-", "a.fifo", "a1", "aB", "a_", "ab"]);
    }

    #[test]
    fn handles_are_recognized_by_exact_text() {
        let mut ids = IdGenerator::default();
        let first = ids.next_handle();
        ids.next_handle();
        assert_eq!(ids.parse_handle("r-1").unwrap(), first);
        for text in [
            "r-3",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            "r-",
            "r--1",
            "r-+1",
            "1",
            "r-99999999999999999999",
        ] {
            let err = ids.parse_handle(text).unwrap_err();
            assert_eq!(err.code, ErrorCode::ReceiptHandleInvalid, "{text:?}");
        }
    }

    #[test]
    fn ids_serialize_with_their_prefix() {
        let mut ids = IdGenerator::default();
        assert_eq!(
            serde_json::to_string(&ids.next_message()).unwrap(),
            "\"m-1\""
        );
        assert_eq!(
            serde_json::to_string(&ids.next_handle()).unwrap(),
            "\"r-1\""
        );
    }

    #[test]
    fn group_ids_follow_the_token_rule() {
        assert!(GroupId::parse("g-1_A").is_ok());
        assert!(GroupId::parse(&"x".repeat(128)).is_ok());
        assert!(GroupId::parse(&"x".repeat(129)).is_err());
        assert!(GroupId::parse("").is_err());
        assert!(DeduplicationId::parse("a b").is_err());
    }
}
