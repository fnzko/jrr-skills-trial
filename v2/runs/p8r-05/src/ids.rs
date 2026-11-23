//! Identifiers handed out by the broker and their wire format (SPEC section 3).

use std::fmt;

/// `m-<n>`: counts the messages accepted over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// The id after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: counts the deliveries made over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// The handle after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
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
    fn handles_round_trip_and_reject_other_spellings() {
        let handle = ReceiptHandle::FIRST.next();
        assert_eq!(handle.to_string(), "r-2");
        assert_eq!(ReceiptHandle::parse("r-2"), Some(handle));
        for bad in ["r-02", "R-2", "r-2 ", "r-+2", "r-", "2", "r-0"] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn message_ids_display_with_prefix() {
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
    }
}
