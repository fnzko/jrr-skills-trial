//! Names and identifiers shared by the request parser and the broker.

use std::fmt;

use serde::{Serialize, Serializer};

/// Name of a queue.
///
/// Any string can name a queue in a lookup; only `CreateQueue` enforces the
/// name syntax, so this type carries no syntax guarantee.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo_name(&self) -> bool {
        self.0.ends_with(".fifo")
    }
}

impl fmt::Display for QueueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Message group of a FIFO message: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(s: &str) -> Option<Self> {
        is_token(s).then(|| Self(s.to_owned()))
    }
}

/// Whether `s` is 1 to 128 characters from `A-Z a-z 0-9 _ -`, the rule for
/// group and deduplication ids.
pub fn is_token(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(is_name_byte)
}

/// The characters allowed in queue names, group ids and deduplication ids.
pub fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `m-<n>`, where `n` counts the messages accepted by the process.
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

/// `r-<n>`, where `n` counts the deliveries made by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
    /// Parses the exact text that `Display` produces, so `r-01`, `R-1` and
    /// `r-1 ` give `None`: they can never have been issued.
    pub fn parse(s: &str) -> Option<Self> {
        let digits = s.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // An empty string or a number past u64 fails here; neither was issued.
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

/// Hands out 1, 2, 3, ... in order.
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn issue(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// The latest number handed out, 0 before the first.
    pub fn last(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handle_parses_only_canonical_text() {
        let cases = [
            ("r-1", Some(ReceiptHandle(1))),
            ("r-42", Some(ReceiptHandle(42))),
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            (" r-1", None),
            ("r-", None),
            ("r-+1", None),
            ("r--1", None),
            ("r-99999999999999999999999", None),
            ("m-1", None),
        ];
        for (input, want) in cases {
            assert_eq!(ReceiptHandle::parse(input), want, "input {input:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId(7).to_string(), "m-7");
        assert_eq!(ReceiptHandle(12).to_string(), "r-12");
    }

    #[test]
    fn token_rule_bounds_length_and_characters() {
        let max = "a".repeat(128);
        let too_long = "a".repeat(129);
        let cases = [
            ("a", true),
            ("A-z_09", true),
            (max.as_str(), true),
            ("", false),
            (too_long.as_str(), false),
            ("a.b", false),
            ("a b", false),
            ("\u{e9}", false),
        ];
        for (input, want) in cases {
            assert_eq!(is_token(input), want, "input {input:?}");
        }
    }

    #[test]
    fn counter_starts_at_one() {
        let mut counter = Counter::default();
        assert_eq!(counter.last(), 0);
        assert_eq!(counter.issue(), 1);
        assert_eq!(counter.issue(), 2);
        assert_eq!(counter.last(), 2);
    }
}
