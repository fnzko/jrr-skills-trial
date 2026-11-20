//! Counters the broker hands out. Each kind is its own type so that a message
//! id cannot be used where a receipt handle or sequence number is meant.

use std::fmt;

/// `m-<n>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns this id and moves on to the next one.
    pub fn take_next(&mut self) -> Self {
        let id = *self;
        self.0 += 1;
        id
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns this handle and moves on to the next one.
    pub fn take_next(&mut self) -> Self {
        let handle = *self;
        self.0 += 1;
        handle
    }

    /// Parses `r-<n>` exactly as it is written: no sign, no leading zeros.
    /// Whether the handle was ever issued is for the caller to decide.
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

/// The per-queue counter of a FIFO queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    /// Returns this number and moves on to the next one.
    pub fn take_next(&mut self) -> Self {
        let number = *self;
        self.0 += 1;
        number
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_the_canonical_spelling() {
        let mut next = ReceiptHandle::FIRST;
        let first = next.take_next();
        assert_eq!(ReceiptHandle::parse("r-1"), Some(first));
        assert_eq!(first.to_string(), "r-1");
        assert_eq!(ReceiptHandle::parse("r-1").map(|h| h < next), Some(true));
        for bad in [
            "r-01", "R-1", "r-1 ", "r-", "r--1", "r-+1", "1", "r-0x1", "",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
        assert_eq!(ReceiptHandle::parse("r-99999999999999999999"), None);
    }

    #[test]
    fn counters_start_at_one_and_advance() {
        let mut next = MessageId::FIRST;
        assert_eq!(next.take_next().to_string(), "m-1");
        assert_eq!(next.take_next().to_string(), "m-2");
        let mut sequence = SequenceNumber::FIRST;
        assert_eq!(sequence.take_next().get(), 1);
        assert_eq!(sequence.take_next().get(), 2);
    }
}
