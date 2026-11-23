//! Identifiers handed out by the broker (SPEC section 3).

use std::fmt;

/// `m-<n>`: counts the messages accepted by the process.
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

/// `r-<n>`: counts the deliveries made by the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_round_trip_only_in_canonical_form() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle::FIRST));
        assert_eq!(ReceiptHandle::FIRST.next().to_string(), "r-2");
        for bad in ["r-01", "R-1", "r-1 ", "r-", "r--1", "r-+1", "m-1", "1"] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn message_ids_display_with_prefix() {
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
    }
}
