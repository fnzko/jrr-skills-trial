//! Small value types shared by the whole service.

use std::borrow::Borrow;
use std::fmt;
use std::ops::RangeInclusive;
use std::sync::Arc;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A point on the virtual clock, in seconds since the start of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Instant(u64);

impl Instant {
    pub const START: Instant = Instant(0);

    pub fn plus(self, span: Seconds) -> Instant {
        Instant(self.0.saturating_add(u64::from(span.0)))
    }
}

/// A span of virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Seconds(u32);

impl Seconds {
    pub const fn new(seconds: u32) -> Self {
        Self(seconds)
    }
}

/// The accepted range of the `Seconds` parameter of `Tick`.
pub const TICK_RANGE: RangeInclusive<u32> = 0..=1_209_600;

/// The number of the next message to be accepted, counted over all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The number of a delivery, counted over all queues; the handle `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        Self(number)
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

/// Tells queues apart over time: a queue created under a reused name gets a new id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueId(u64);

impl QueueId {
    pub const FIRST: QueueId = QueueId(1);

    pub fn next(self) -> QueueId {
        QueueId(self.0 + 1)
    }
}

/// The position in which a message arrived in its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(u64);

impl ArrivalKey {
    pub const FIRST: ArrivalKey = ArrivalKey(0);

    pub fn next(self) -> ArrivalKey {
        ArrivalKey(self.0 + 1)
    }
}

const FIFO_SUFFIX: &str = ".fifo";

/// The name of an existing or newly created queue; always follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueName(Arc<str>);

impl QueueName {
    pub fn parse(name: &str) -> Result<Self, Error> {
        let plain = is_token(name, 1..=80);
        let fifo = name
            .strip_suffix(FIFO_SUFFIX)
            .is_some_and(|stem| is_token(stem, 1..=75));
        if plain || fifo {
            Ok(Self(name.into()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue name {name:?} breaks the naming rules"),
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

impl Serialize for QueueName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Lets maps keyed by `QueueName` be searched with any string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// True if `text` has a length in `lengths` and only the characters `A-Z a-z 0-9 _ -`.
fn is_token(text: &str, lengths: RangeInclusive<usize>) -> bool {
    lengths.contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn parse_token(parameter: &str, text: &str) -> Result<String, Error> {
    if is_token(text, 1..=128) {
        Ok(text.to_owned())
    } else {
        Err(Error::invalid(
            parameter,
            "must be 1 to 128 characters from A-Z a-z 0-9 _ -",
        ))
    }
}

/// A message body: 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body(String);

impl Body {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if (1..=1024).contains(&text.len()) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::invalid("Body", "must be 1 to 1024 bytes long"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The message group of a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_token("GroupId", text).map(Self)
    }
}

/// A deduplication id given by the sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        parse_token("DeduplicationId", text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_plain = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_plain = "a".repeat(81);
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let accepted = [
            "a",
            "A-z_9",
            "a.fifo",
            "fifo",
            long_plain.as_str(),
            long_fifo.as_str(),
        ];
        let rejected = [
            "",
            ".fifo",
            "a.fifo.fifo",
            "a b",
            "a.",
            "\u{e9}",
            "a.FIFO",
            too_long_plain.as_str(),
            too_long_fifo.as_str(),
        ];
        for name in accepted {
            let parsed = QueueName::parse(name).map(|parsed| parsed.as_str().to_owned());
            assert_eq!(parsed, Ok(name.to_owned()));
        }
        for name in rejected {
            let code = QueueName::parse(name).err().map(|error| error.code());
            assert_eq!(code, Some(ErrorCode::QueueNameInvalid), "{name:?}");
        }
    }

    #[test]
    fn only_the_suffix_makes_a_queue_fifo() {
        assert_eq!(QueueName::parse("a.fifo").map(|n| n.is_fifo()), Ok(true));
        assert_eq!(QueueName::parse("fifo").map(|n| n.is_fifo()), Ok(false));
    }
}
