//! Identifiers handed out by the broker.

use std::fmt;

/// Identifies an accepted message, rendered as `m-<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

/// Identifies one delivery of a message, rendered as `r-<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

/// Position of a message among the messages added to one FIFO queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceNumber(u64);

impl MessageId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl SequenceNumber {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

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

/// Hands out numbers 1, 2, 3, ... for one kind of identifier.
#[derive(Debug)]
pub struct Counter {
    issued: u64,
}

impl Counter {
    pub fn new() -> Self {
        Self { issued: 0 }
    }

    fn next_number(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    pub fn next_message_id(&mut self) -> MessageId {
        MessageId(self.next_number())
    }

    pub fn next_receipt_handle(&mut self) -> ReceiptHandle {
        ReceiptHandle(self.next_number())
    }

    pub fn next_sequence_number(&mut self) -> SequenceNumber {
        SequenceNumber(self.next_number())
    }

    /// True if `handle` came out of this counter.
    pub fn has_issued(&self, handle: ReceiptHandle) -> bool {
        (1..=self.issued).contains(&handle.0)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiptHandle {
    /// Parses the canonical text `r-<n>` and nothing else (no sign, no
    /// leading zeros, no whitespace).
    pub fn parse(text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return None;
        }
        digits.parse().ok().map(ReceiptHandle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_only_canonical_handles() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle(1)));
        assert_eq!(ReceiptHandle::parse("r-120"), Some(ReceiptHandle(120)));
        for bad in [
            "r-01", "R-1", "r-1 ", " r-1", "r-", "r-+1", "r--1", "1", "", "r-0x1",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
        assert_eq!(ReceiptHandle::parse("r-99999999999999999999999"), None);
    }

    #[test]
    fn counter_counts_from_one_and_knows_what_it_issued() {
        let mut counter = Counter::new();
        let zero = ReceiptHandle(0);
        assert!(!counter.has_issued(zero));
        let first = counter.next_receipt_handle();
        let second = counter.next_receipt_handle();
        assert_eq!(
            (first.to_string(), second.to_string()),
            ("r-1".into(), "r-2".into())
        );
        assert!(counter.has_issued(second));
        assert!(!counter.has_issued(ReceiptHandle(3)));
    }
}
