use std::fmt;

use serde::{Serialize, Serializer};

/// Identifier of an accepted message, rendered `m-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

/// Identifier of one delivery, rendered `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl ReceiptHandle {
    /// Reads a handle that this process may have issued. `issued` is how many
    /// handles exist, so `r-1` to `r-<issued>` are accepted; the text must be
    /// exactly what `Display` produces (no sign, no leading zeros, no spaces).
    pub fn parse_issued(text: &str, issued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (1..=issued).contains(&n).then_some(Self(n))
    }
}

/// Hands out the consecutive numbers 1, 2, 3, ... for both kinds of id.
#[derive(Debug, Default)]
pub struct IdGenerator {
    messages: u64,
    receipts: u64,
}

impl IdGenerator {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    pub fn receipts_issued(&self) -> u64 {
        self.receipts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_issued_accepts_only_exact_issued_handles() {
        let cases = [
            ("r-1", Some(1)),
            ("r-3", Some(3)),
            ("r-4", None),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("r-99999999999999999999999", None),
            ("1", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse_issued(text, 3).map(|h| h.0);
            assert_eq!(parsed, expected, "{text:?}");
        }
    }

    #[test]
    fn generator_counts_each_kind_separately() {
        let mut ids = IdGenerator::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_receipt().to_string(), "r-1");
        assert_eq!(ids.receipts_issued(), 1);
    }
}
