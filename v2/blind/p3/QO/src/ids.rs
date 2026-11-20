//! Identifiers that clients see: `m-<n>` message ids and `r-<n>` receipt
//! handles (SPEC section 3).

use std::fmt;

/// Identifies an accepted message over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns this id and moves to the next one.
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

/// Names the latest delivery of a message. Handles are issued in increasing
/// order, so comparing two of them tells which was issued first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns this handle and moves to the next one.
    pub fn take_next(&mut self) -> Self {
        let handle = *self;
        self.0 += 1;
        handle
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_round_trip_through_their_wire_form() {
        let mut next = ReceiptHandle::FIRST;
        let first = next.take_next();
        let second = next.take_next();
        assert_eq!(first.to_string(), "r-1");
        assert_eq!(ReceiptHandle::parse("r-2"), Some(second));
        assert!(first < second && second < next);
    }

    #[test]
    fn handles_must_be_written_as_issued() {
        for bad in [
            "",
            "r-",
            "r-0",
            "r-01",
            "r--1",
            "r-+1",
            "1",
            "R-1",
            "r-1 ",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn message_ids_count_from_one() {
        let mut next = MessageId::FIRST;
        assert_eq!(next.take_next().to_string(), "m-1");
        assert_eq!(next.take_next().to_string(), "m-2");
    }
}
