//! Value types shared by the protocol and the broker.
//!
//! Types with a validity rule can only be built through a checking constructor,
//! so a value of the type is proof that the rule holds.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

/// The rule a rejected value broke, phrased to follow the parameter name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{rule}")]
pub struct InvalidValue {
    pub rule: &'static str,
}

/// A point on the virtual clock, in whole seconds since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Saturates instead of wrapping; the spec leaves behavior past 4000000000
    /// unspecified, and saturation keeps that region free of panics.
    #[must_use]
    pub fn plus(self, duration: Seconds) -> Self {
        Self(self.0.saturating_add(u64::from(duration.0)))
    }

    /// The point `duration` before this one, or `None` if that is before 0.
    #[must_use]
    pub fn minus(self, duration: Seconds) -> Option<Self> {
        self.0.checked_sub(u64::from(duration.0)).map(Self)
    }
}

/// A span of virtual time in whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u32);

impl Seconds {
    #[must_use]
    pub const fn new(seconds: u32) -> Self {
        Self(seconds)
    }
}

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.0)
    }
}

const FIFO_SUFFIX: &str = ".fifo";

/// A queue name that follows the naming rules of the spec.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    /// Accepts 1 to 80 characters that are either all from `A-Z a-z 0-9 _ -`,
    /// or 1 to 75 such characters followed by `.fifo`.
    pub fn parse(name: String) -> Result<Self, InvalidValue> {
        let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(&name);
        let max_len = if stem.len() == name.len() { 80 } else { 75 };
        if stem.is_empty() || stem.len() > max_len {
            return Err(InvalidValue {
                rule: "must be 1 to 80 characters long, with 1 to 75 before a .fifo suffix",
            });
        }
        if !stem.bytes().all(is_name_byte) {
            return Err(InvalidValue {
                rule: "may only contain A-Z a-z 0-9 _ - and an optional .fifo suffix",
            });
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// Lets queue maps be searched with the unvalidated `&str` that most commands carry.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for QueueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// A message body of 1 to 1024 bytes of UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn parse(body: String) -> Result<Self, InvalidValue> {
        if (1..=1024).contains(&body.len()) {
            Ok(Self(body))
        } else {
            Err(InvalidValue {
                rule: "must be 1 to 1024 bytes long",
            })
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn parse_token(token: &str) -> Result<(), InvalidValue> {
    if (1..=128).contains(&token.len()) && token.bytes().all(is_name_byte) {
        Ok(())
    } else {
        Err(InvalidValue {
            rule: "must be 1 to 128 characters from A-Z a-z 0-9 _ -",
        })
    }
}

/// The message group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(id: String) -> Result<Self, InvalidValue> {
        parse_token(&id).map(|()| Self(id))
    }
}

/// An explicit deduplication id given with a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(id: String) -> Result<Self, InvalidValue> {
        parse_token(&id).map(|()| Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How many deliveries a message may have before it moves to the dead-letter
/// queue: 1 to 1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MaxReceiveCount(u32);

impl MaxReceiveCount {
    pub fn new(count: u32) -> Result<Self, InvalidValue> {
        if (1..=1000).contains(&count) {
            Ok(Self(count))
        } else {
            Err(InvalidValue {
                rule: "must be from 1 to 1000",
            })
        }
    }

    /// Whether a message delivered `receive_count` times has used up its deliveries.
    #[must_use]
    pub fn is_reached_by(self, receive_count: u64) -> bool {
        receive_count >= u64::from(self.0)
    }
}

/// A dead-letter configuration whose queue is referred to by name only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DeadLetterRef<'a> {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: &'a str,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: MaxReceiveCount,
}

/// The process-wide id of an accepted message, shown as `m-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("m-{}", self.0))
    }
}

/// The handle of one delivery, shown as `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Reads the exact form `r-<n>` with `n` in canonical decimal, which is the
    /// only form ever issued; anything else could not have been issued.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0');
        if !canonical {
            return None;
        }
        digits.parse().ok().map(Self)
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

/// The position of a message among those added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

/// Hands out ids `1, 2, 3, ...` in order. One counter exists per id space.
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn next_message_id(&mut self) -> MessageId {
        MessageId(self.advance())
    }

    pub fn next_receipt_handle(&mut self) -> ReceiptHandle {
        ReceiptHandle(self.advance())
    }

    pub fn next_sequence_number(&mut self) -> SequenceNumber {
        SequenceNumber(self.advance())
    }

    /// Whether `handle` is among those this counter has handed out.
    #[must_use]
    pub fn has_issued(&self, handle: ReceiptHandle) -> bool {
        (1..=self.0).contains(&handle.0)
    }

    fn advance(&mut self) -> u64 {
        // One id per input line at most, so u64 cannot run out.
        self.0 += 1;
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules() {
        let long = |n: usize| "a".repeat(n);
        let cases = [
            ("jobs", true),
            ("A-z_09", true),
            ("jobs.fifo", true),
            (".fifo", false),
            ("", false),
            ("a.b", false),
            ("jobs.fifo.fifo", false),
            ("jobs ", false),
            ("j\u{e9}", false),
            (&long(80), true),
            (&long(81), false),
            (&format!("{}.fifo", long(75)), true),
            (&format!("{}.fifo", long(76)), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name.to_owned()).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn body_length_counts_utf8_bytes() {
        assert_eq!(
            Body::parse("\u{e9}".repeat(512)).map(|b| b.0.len()),
            Ok(1024)
        );
        assert_eq!(
            Body::parse(format!("{}a", "\u{e9}".repeat(512))),
            Err(InvalidValue {
                rule: "must be 1 to 1024 bytes long"
            })
        );
        assert_eq!(
            Body::parse(String::new()),
            Err(InvalidValue {
                rule: "must be 1 to 1024 bytes long"
            })
        );
    }

    #[test]
    fn receipt_handle_accepts_only_the_issued_form() {
        let cases = [
            ("r-1", Some(1)),
            ("r-42", Some(42)),
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
                ReceiptHandle::parse(text),
                expected.map(ReceiptHandle),
                "{text:?}"
            );
        }
    }

    #[test]
    fn counter_starts_at_one_and_tracks_issued_handles() {
        let mut counter = Counter::default();
        assert!(!counter.has_issued(ReceiptHandle(1)));
        assert_eq!(counter.next_receipt_handle(), ReceiptHandle(1));
        assert_eq!(counter.next_receipt_handle(), ReceiptHandle(2));
        assert!(counter.has_issued(ReceiptHandle(2)));
        assert!(!counter.has_issued(ReceiptHandle(3)));
    }

    #[test]
    fn ids_serialize_with_their_prefix() {
        let mut counter = Counter::default();
        assert_eq!(
            serde_json::to_string(&counter.next_message_id()).ok(),
            Some("\"m-1\"".to_owned())
        );
        assert_eq!(
            serde_json::to_string(&counter.next_receipt_handle()).ok(),
            Some("\"r-2\"".to_owned())
        );
    }
}
