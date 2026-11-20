//! Identifiers and validated names of the miniq data model.

use std::fmt;
use std::str::FromStr;

use serde::{Serialize, Serializer};

const MAX_QUEUE_NAME_LEN: usize = 80;
const FIFO_SUFFIX: &str = ".fifo";
const MAX_ID_LEN: usize = 128;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `m-<n>`: identifies a message accepted by the process.
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

/// `r-<n>`: identifies one delivery of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        Self(number)
    }

    /// Reads a handle in its one canonical spelling, `r-` and a positive
    /// decimal number without sign or leading zeros.
    ///
    /// Returns `None` for any other text, which can therefore never have been
    /// issued. Whether a parsed handle was issued is up to the caller.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = digits.bytes().all(|b| b.is_ascii_digit()) && !digits.starts_with('0');
        if !canonical {
            return None;
        }
        digits.parse().ok().map(Self)
    }

    pub fn number(self) -> u64 {
        self.0
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

/// Position of a message in the arrival order of one queue, counting from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueName(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidQueueName;

impl fmt::Display for InvalidQueueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "QueueName must be 1 to {MAX_QUEUE_NAME_LEN} characters from A-Z a-z 0-9 _ -, \
             optionally followed by {FIFO_SUFFIX}"
        )
    }
}

impl FromStr for QueueName {
    type Err = InvalidQueueName;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
        let valid =
            !stem.is_empty() && name.len() <= MAX_QUEUE_NAME_LEN && stem.chars().all(is_name_char);
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidQueueName)
        }
    }
}

impl QueueName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name carries the `.fifo` suffix.
    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId;

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "must be 1 to {MAX_ID_LEN} characters from A-Z a-z 0-9 _ -"
        )
    }
}

macro_rules! client_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let valid = (1..=MAX_ID_LEN).contains(&text.len()) && text.chars().all(is_name_char);
                if valid {
                    Ok(Self(text.to_owned()))
                } else {
                    Err(InvalidId)
                }
            }
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

client_id!(
    /// Names the ordering group of a message in a FIFO queue.
    GroupId
);
client_id!(
    /// Client-chosen key that makes repeated sends to a FIFO queue idempotent.
    DeduplicationId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_syntax() {
        let cases = [
            ("jobs", true),
            ("a", true),
            ("A-z_0-9", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            ("jobs.FIFO", false),
            ("é", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{}.fifo", "a".repeat(75)), true),
            (&format!("{}.fifo", "a".repeat(76)), false),
        ];
        for (name, valid) in cases {
            assert_eq!(name.parse::<QueueName>().is_ok(), valid, "name {name:?}");
        }
    }

    #[test]
    fn queue_name_knows_its_suffix() {
        assert!("a.fifo".parse::<QueueName>().unwrap().is_fifo());
        assert!(!"a".parse::<QueueName>().unwrap().is_fifo());
    }

    #[test]
    fn client_ids_follow_the_syntax() {
        let cases = [
            ("g", true),
            ("Group_1-b", true),
            ("", false),
            ("a.b", false),
            ("a b", false),
            (&"x".repeat(128), true),
            (&"x".repeat(129), false),
        ];
        for (id, valid) in cases {
            assert_eq!(id.parse::<GroupId>().is_ok(), valid, "id {id:?}");
            assert_eq!(id.parse::<DeduplicationId>().is_ok(), valid, "id {id:?}");
        }
    }

    #[test]
    fn receipt_handles_parse_only_in_canonical_form() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle::new(1)));
        assert_eq!(ReceiptHandle::parse("r-120"), Some(ReceiptHandle::new(120)));
        let never_issued = [
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-",
            "r-0",
            "r-+1",
            "r--1",
            "r-1.0",
            "r-99999999999999999999999",
            "m-1",
            "1",
            "",
        ];
        for text in never_issued {
            assert_eq!(ReceiptHandle::parse(text), None, "text {text:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId::new(7).to_string(), "m-7");
        assert_eq!(ReceiptHandle::new(9).to_string(), "r-9");
    }
}
