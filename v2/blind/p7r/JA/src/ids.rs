//! Identifiers handed out by the broker. Each kind is its own type so that a
//! message number cannot be used where a receipt handle is expected.

use std::fmt;

/// `m-<n>`, counted over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`, counted over all queues in delivery order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Parses `r-<n>` exactly as it was issued: no sign, no leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
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

/// Counts the messages added to one FIFO queue, starting at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_as_issued() {
        let cases = [
            ("r-1", Some(ReceiptHandle(1))),
            ("r-10", Some(ReceiptHandle(10))),
            ("r-01", None),
            ("r-0", None),
            ("r--1", None),
            ("r-+1", None),
            ("r-", None),
            ("m-1", None),
            ("1", None),
            ("r-99999999999999999999", None),
        ];
        for (text, expected) in cases {
            assert_eq!(ReceiptHandle::parse(text), expected, "{text}");
        }
    }

    #[test]
    fn identifiers_display_with_their_prefix() {
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
        assert_eq!(ReceiptHandle::FIRST.to_string(), "r-1");
    }
}
