//! Small domain types: virtual time, identifiers and validated names.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Add;

use serde::{Serialize, Serializer};

use crate::error::{Code, Error, Result};

/// A point on the virtual clock, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Time(u64);

impl Time {
    pub const START: Time = Time(0);

    pub fn advance(&mut self, span: Seconds) {
        *self = *self + span;
    }
}

/// A span of virtual time, in seconds. Every span the protocol accepts fits in `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Seconds(u32);

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub const fn new(seconds: u32) -> Self {
        Self(seconds)
    }
}

impl Add<Seconds> for Time {
    type Output = Time;

    fn add(self, span: Seconds) -> Time {
        Time(self.0.saturating_add(u64::from(span.0)))
    }
}

/// Position of a message in its queue's arrival order. Never reused within a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot(u64);

impl Slot {
    pub const FIRST: Slot = Slot(0);

    pub fn next(self) -> Slot {
        Slot(self.0 + 1)
    }
}

/// Identity of one queue incarnation; a queue recreated under the same name gets a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueId(u64);

impl QueueId {
    pub const FIRST: QueueId = QueueId(1);

    pub fn next(self) -> QueueId {
        QueueId(self.0 + 1)
    }
}

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
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Reads the canonical text `r-<n>` with `n >= 1`; any other spelling was never issued.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
            return None;
        }
        digits.parse().ok().map(Self)
    }

    /// The handle with the given 1-based issue number.
    pub(crate) fn from_issue_number(number: u64) -> Self {
        Self(number)
    }

    /// The 1-based issue number.
    pub(crate) fn issue_number(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Per-FIFO-queue message counter value.
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

/// A name that satisfies the queue naming rules. Names of lookups stay plain `&str`: any
/// string can be looked up, only an existing queue has a valid name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Result<Self> {
        let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
        let stem_limit = if stem.len() == name.len() { 80 } else { 75 };
        if stem.is_empty() || stem.len() > stem_limit || !stem.chars().all(is_name_char) {
            return Err(Error::new(
                Code::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn validate_token(what: &str, text: &str) -> Result<()> {
    if (1..=128).contains(&text.len()) && text.chars().all(is_name_char) {
        Ok(())
    } else {
        Err(Error::invalid_value(format!(
            "{what} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self> {
        validate_token("GroupId", text)?;
        Ok(Self(text.to_owned()))
    }
}

/// A FIFO deduplication id: either a validated `DeduplicationId` or a message body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupId(String);

impl DedupId {
    pub fn parse(text: &str) -> Result<Self> {
        validate_token("DeduplicationId", text)?;
        Ok(Self(text.to_owned()))
    }

    /// Content-based deduplication uses the body as is, whatever its length or content.
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_in_canonical_form() {
        assert_eq!(
            ReceiptHandle::parse("r-1").map(ReceiptHandle::issue_number),
            Some(1)
        );
        for text in [
            "r-01",
            "R-1",
            "r-1 ",
            "r-",
            "r-0",
            "r--1",
            "r-+1",
            " r-1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(
                ReceiptHandle::parse(text).map(ReceiptHandle::issue_number),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn handles_print_as_issued() {
        assert_eq!(ReceiptHandle::from_issue_number(12).to_string(), "r-12");
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
    }
}
