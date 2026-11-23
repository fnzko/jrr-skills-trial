//! Identifiers handed out by the broker, kept apart from plain numbers so
//! message ids, receipt handles and queue positions cannot be mixed up.

use std::fmt;

/// `m-<n>`: counts the accepted messages over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: counts the deliveries over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(n: u64) -> Self {
        Self(n)
    }

    /// Parses `r-<n>` exactly as it is written by `Display`: no sign and no
    /// leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }

    /// True if the counter value `next` has already passed this handle.
    pub fn is_issued_before(self, next: u64) -> bool {
        self.0 < next
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
    fn receipt_handles_round_trip_only_in_canonical_form() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle::new(1)));
        assert_eq!(ReceiptHandle::new(42).to_string(), "r-42");
        for bad in ["r-01", "R-1", "r-1 ", "r--1", "r-+1", "r-", "1", "r-0", ""] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }
}
