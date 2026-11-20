//! Names and identifiers. Each kind has its own type so a queue name cannot be used as a
//! message id, and each validated kind can only be built through its parser.

use serde::Serialize;
use std::borrow::Borrow;
use std::ops::RangeInclusive;

/// A queue name that follows the naming rules of SPEC section 4.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    /// Returns None when `text` breaks the naming rules. Lookups by name do not need this:
    /// a name that breaks the rules cannot match a stored queue.
    pub fn parse(text: &str) -> Option<QueueName> {
        let valid = match text.strip_suffix(".fifo") {
            Some(stem) => is_word(stem, 1..=75),
            None => is_word(text, 1..=80),
        };
        valid.then(|| QueueName(text.to_owned()))
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

/// True when `text` is `length` bytes long and only uses the characters A-Z a-z 0-9 _ -.
/// Every spelling rule in the spec is this character set with a length range, so one check
/// serves all of them. Non-ASCII text fails the character check, so byte length is enough.
fn is_word(text: &str, length: RangeInclusive<usize>) -> bool {
    length.contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A GroupId: 1 to 128 characters from A-Z a-z 0-9 _ -.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<GroupId> {
        is_word(text, 1..=128).then(|| GroupId(text.to_owned()))
    }
}

/// An explicit DeduplicationId: same syntax as a GroupId, but a separate kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Option<DeduplicationId> {
        is_word(text, 1..=128).then(|| DeduplicationId(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A MessageId, shown on the wire as `m-<n>`. Numbers start at 1 and are issued by the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    /// The broker numbers messages and deliveries; this is for the broker to call.
    pub const fn issued(number: u64) -> MessageId {
        MessageId(number)
    }
}

impl Serialize for MessageId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("m-{}", self.0))
    }
}

/// A ReceiptHandle, shown on the wire as `r-<n>`. Numbers start at 1 and count deliveries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const fn issued(number: u64) -> ReceiptHandle {
        ReceiptHandle(number)
    }

    /// Reads a handle as a client wrote it. Only the canonical spelling is recognized: no sign,
    /// no leading zero, so `r-01` and `R-1` do not parse. Returns None for anything else.
    pub fn parse(text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0');
        if !canonical {
            return None;
        }
        digits.parse::<u64>().ok().map(ReceiptHandle)
    }

    pub const fn number(self) -> u64 {
        self.0
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("r-{}", self.0))
    }
}

/// A SequenceNumber of a FIFO queue: counts the messages added to that queue, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const fn issued(number: u64) -> SequenceNumber {
        SequenceNumber(number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let cases: Vec<(String, bool)> = vec![
            ("jobs".into(), true),
            ("A-b_9".into(), true),
            ("a".repeat(80), true),
            ("a".repeat(81), false),
            (String::new(), false),
            ("bad.name".into(), false),
            ("jobs.fifo".into(), true),
            (".fifo".into(), false),
            (format!("{}.fifo", "a".repeat(75)), true),
            (format!("{}.fifo", "a".repeat(76)), false),
            ("caf\u{e9}".into(), false),
        ];
        for (text, valid) in cases {
            assert_eq!(QueueName::parse(&text).is_some(), valid, "name {text:?}");
        }
    }

    #[test]
    fn group_and_deduplication_ids_are_words_of_up_to_128() {
        let cases: Vec<(String, bool)> = vec![
            ("g".into(), true),
            ("A_z-0".into(), true),
            ("a".repeat(128), true),
            ("a".repeat(129), false),
            (String::new(), false),
            ("a b".into(), false),
            ("a.b".into(), false),
        ];
        for (text, valid) in cases {
            assert_eq!(GroupId::parse(&text).is_some(), valid, "group {text:?}");
            assert_eq!(
                DeduplicationId::parse(&text).is_some(),
                valid,
                "dedup {text:?}"
            );
        }
    }

    #[test]
    fn receipt_handles_parse_only_in_canonical_form() {
        let cases: &[(&str, Option<u64>)] = &[
            ("r-1", Some(1)),
            ("r-42", Some(42)),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("r--1", None),
            ("r-+1", None),
            ("1", None),
            ("r-99999999999999999999999", None),
        ];
        for &(text, number) in cases {
            assert_eq!(
                ReceiptHandle::parse(text).map(ReceiptHandle::number),
                number,
                "handle {text:?}"
            );
        }
    }
}
