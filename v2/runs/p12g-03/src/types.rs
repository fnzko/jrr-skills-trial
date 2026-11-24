use std::fmt;
use std::rc::Rc;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

const FIFO_SUFFIX: &str = ".fifo";
const MAX_QUEUE_NAME_LEN: usize = 80;

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    /// Saturates: behavior past 4000000000 seconds is unspecified, and a clock
    /// that cannot go backwards is the only property callers rely on.
    pub fn plus(self, duration: Seconds) -> Time {
        Time(self.0.saturating_add(u64::from(duration.0)))
    }
}

/// A span of virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Seconds(pub u32);

/// The name of an existing or creatable queue, checked against the naming
/// rules. Lookups by arbitrary strings go through `Borrow<str>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Result<Self, Error> {
        let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
        let valid = name.len() <= MAX_QUEUE_NAME_LEN
            && !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
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

/// Identifies one queue for the life of the process. A queue created under a
/// name that was deleted gets a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueUid(pub u64);

/// `m-<n>`, `n` counting the messages the process accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

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

/// `r-<n>`, `n` counting the deliveries the process made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(pub u64);

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

/// The position of a message among those added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(pub u64);

/// A `GroupId` or `DeduplicationId` that follows the identifier syntax: 1 to
/// 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Debug, Clone, Copy)]
pub struct Identifier<'a>(&'a str);

impl<'a> Identifier<'a> {
    pub fn parse(text: &'a str) -> Option<Self> {
        let valid = (1..=128).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then_some(Self(text))
    }

    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

/// The message group of a FIFO message. Cloning is cheap because receiving
/// tracks the groups it has seen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupId(Rc<str>);

impl GroupId {
    pub fn new(identifier: Identifier) -> Self {
        Self(Rc::from(identifier.as_str()))
    }
}

impl Serialize for GroupId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
