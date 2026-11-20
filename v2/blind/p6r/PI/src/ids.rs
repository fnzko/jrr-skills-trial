//! Identifiers that the process hands out: they are all counters, so they
//! get distinct types to keep them from being mixed up.

use std::fmt;

/// `m-<n>`, counted over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub fn first() -> Self {
        Self(1)
    }

    /// Returns the current id and moves `self` on to the next one.
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

/// `r-<n>`, counted over all deliveries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn first() -> Self {
        Self(1)
    }

    /// Returns the current handle and moves `self` on to the next one.
    pub fn take_next(&mut self) -> Self {
        let handle = *self;
        self.0 += 1;
        handle
    }

    /// Parses `r-<n>` exactly as it is written: no sign, no leading zeros.
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

/// The position of a message in the arrival order of one queue. It only
/// grows, so it stays valid while other messages come and go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Arrival(u64);

impl Arrival {
    pub fn first() -> Self {
        Self(0)
    }

    /// Returns the current position and moves `self` on to the next one.
    pub fn take_next(&mut self) -> Self {
        let arrival = *self;
        self.0 += 1;
        arrival
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_as_issued() {
        let mut next = ReceiptHandle::first();
        let one = next.take_next();
        assert_eq!(ReceiptHandle::parse("r-1"), Some(one));
        assert_eq!(one.to_string(), "r-1");
        assert!(ReceiptHandle::parse("r-2").unwrap() >= next);
        for bad in ["r-01", "R-1", "r-1 ", "r-", "r-+1", "r--1", "1", "r-0x1"] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn message_ids_count_from_one() {
        let mut next = MessageId::first();
        assert_eq!(next.take_next().to_string(), "m-1");
        assert_eq!(next.take_next().to_string(), "m-2");
    }
}
