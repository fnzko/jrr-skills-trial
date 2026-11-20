//! Identifiers that are all `u64` underneath but must never be mixed up.

use std::fmt;

/// Hands out the next value of an identifier counter, starting at `first`.
macro_rules! counter_id {
    ($name:ident, $first:expr) => {
        impl $name {
            pub(crate) const FIRST: Self = Self($first);

            /// Returns the current value and advances to the next one.
            pub(crate) fn issue(&mut self) -> Self {
                let issued = *self;
                self.0 += 1;
                issued
            }
        }
    };
}

/// Position of a message in a queue's arrival order. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ArrivalIndex(u64);
counter_id!(ArrivalIndex, 0);

/// `m-<n>`: identifies a message over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MessageId(u64);
counter_id!(MessageId, 1);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: identifies one delivery over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReceiptHandle(u64);
counter_id!(ReceiptHandle, 1);

impl ReceiptHandle {
    /// Parses `r-<n>` exactly as it is written by `Display`: no sign and no
    /// leading zeros.
    pub(crate) fn parse(text: &str) -> Option<Self> {
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
    fn receipt_handles_parse_only_their_own_format() {
        let mut next = ReceiptHandle::FIRST;
        let first = next.issue();
        assert_eq!(first.to_string(), "r-1");
        assert_eq!(ReceiptHandle::parse("r-1"), Some(first));
        for bad in ["r-01", "R-1", "r-1 ", "r-+1", "r-", "r--1", "1", "r-0"] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn counters_issue_consecutive_values() {
        let mut next = MessageId::FIRST;
        assert_eq!(next.issue().to_string(), "m-1");
        assert_eq!(next.issue().to_string(), "m-2");
    }
}
