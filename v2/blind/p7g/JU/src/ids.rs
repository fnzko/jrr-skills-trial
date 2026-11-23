//! Identifiers. Types with a validity rule can only be built through their constructor.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

const FIFO_SUFFIX: &str = ".fifo";
const MAX_QUEUE_NAME_LEN: usize = 80;
const MAX_TOKEN_LEN: usize = 128;

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Counters shared by every queue: `m-<n>` message ids and `r-<n>` receipt handles.
macro_rules! counter_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const FIRST: Self = Self(1);

            /// Returns the current value and moves the counter to the next one.
            pub fn advance(&mut self) -> Self {
                let issued = *self;
                self.0 = self.0.saturating_add(1);
                issued
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    };
}

counter_id!(
    /// Identifies an accepted message across all queues.
    MessageId,
    "m-"
);
counter_id!(
    /// Identifies one delivery of a message across all queues.
    ReceiptHandle,
    "r-"
);

impl ReceiptHandle {
    /// Parses the canonical spelling `r-<n>` only: no sign, padding or whitespace.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

/// Position of a message among all messages ever added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    /// Returns the current value and moves the counter to the next one.
    pub fn advance(&mut self) -> Self {
        let issued = *self;
        self.0 = self.0.saturating_add(1);
        issued
    }
}

/// The given text breaks the naming rules of its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidIdentifier;

impl fmt::Display for InvalidIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("identifier breaks the naming rules")
    }
}

impl std::error::Error for InvalidIdentifier {}

/// A syntactically valid queue name. Ordering is by byte value, as `ListQueues` requires.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: &str) -> Result<Self, InvalidIdentifier> {
        let base = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
        let valid =
            name.len() <= MAX_QUEUE_NAME_LEN && !base.is_empty() && base.chars().all(is_token_char);
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidIdentifier)
        }
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

/// A FIFO message group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(id: &str) -> Result<Self, InvalidIdentifier> {
        is_valid_token(id)
            .then(|| Self(id.to_owned()))
            .ok_or(InvalidIdentifier)
    }
}

/// The key under which a FIFO queue remembers accepted sends: an explicit `DeduplicationId`
/// or, for content-based deduplication, the body itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn explicit(id: &str) -> Result<Self, InvalidIdentifier> {
        is_valid_token(id)
            .then(|| Self(id.to_owned()))
            .ok_or(InvalidIdentifier)
    }

    /// Bodies are used as they are: any length and content.
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

fn is_valid_token(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TOKEN_LEN && text.chars().all(is_token_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_issue_consecutive_values_from_one() {
        let mut next = MessageId::FIRST;
        assert_eq!(next.advance().to_string(), "m-1");
        assert_eq!(next.advance().to_string(), "m-2");
        let mut next = ReceiptHandle::FIRST;
        assert_eq!(next.advance().to_string(), "r-1");
    }

    #[test]
    fn receipt_handles_parse_only_their_canonical_spelling() {
        let mut counter = ReceiptHandle::FIRST;
        let first = counter.advance();
        assert_eq!(ReceiptHandle::parse("r-1"), Some(first));
        let rejected = [
            "",
            "r-",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r--1",
            "r-+1",
            "m-1",
            "r-1.0",
            "1",
            "r-99999999999999999999999",
        ];
        for text in rejected {
            assert_eq!(ReceiptHandle::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let max_plain = "a".repeat(80);
        let too_long = "a".repeat(81);
        let max_fifo = format!("{}.fifo", "a".repeat(75));
        let long_fifo = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A_b-9", true),
            ("jobs.fifo", true),
            (max_plain.as_str(), true),
            (max_fifo.as_str(), true),
            ("", false),
            (too_long.as_str(), false),
            (long_fifo.as_str(), false),
            (".fifo", false),
            ("a.fifo.fifo", false),
            ("a.b", false),
            ("a b", false),
            ("caf\u{e9}", false),
            ("jobs\n", false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::new(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn queue_names_sort_by_byte_value() {
        let mut names: Vec<_> = ["a", "_", "Z", "9", "a.fifo", "-"]
            .iter()
            .map(|n| QueueName::new(n).map(|q| q.0))
            .collect::<Result<_, _>>()
            .expect("all test names are valid");
        names.sort();
        assert_eq!(names, ["-", "9", "Z", "_", "a", "a.fifo"]);
    }

    #[test]
    fn tokens_are_one_to_128_name_characters() {
        let longest = "g".repeat(128);
        let too_long = "g".repeat(129);
        let cases = [
            ("group-1_A", true),
            (longest.as_str(), true),
            ("", false),
            (too_long.as_str(), false),
            ("a.b", false),
            ("a b", false),
        ];
        for (text, valid) in cases {
            assert_eq!(GroupId::new(text).is_ok(), valid, "{text:?}");
            assert_eq!(DedupKey::explicit(text).is_ok(), valid, "{text:?}");
        }
    }
}
