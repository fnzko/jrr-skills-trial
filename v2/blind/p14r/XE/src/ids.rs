//! Identifier types, so that ids of different kinds cannot be mixed up.

use std::fmt;

/// The id of an accepted message, written `m-<n>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    /// The first id the process hands out.
    pub const FIRST: Self = Self(1);

    /// The id handed out after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// The handle of one delivery, written `r-<n>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// The first handle the process hands out.
    pub const FIRST: Self = Self(1);

    /// The handle handed out after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
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

/// The position of a message in its queue's arrival order. Keys only grow, so
/// ordering by key is ordering by arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(u64);

impl ArrivalKey {
    /// The key of the first message a queue ever receives.
    pub const FIRST: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_parse_only_as_issued() {
        assert_eq!(ReceiptHandle::parse("r-7"), Some(ReceiptHandle(7)));
        for bad in ["r-01", "R-1", "r-1 ", "r-", "r--1", "r-+1", "1", ""] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
        assert_eq!(ReceiptHandle::FIRST.to_string(), "r-1");
    }
}
