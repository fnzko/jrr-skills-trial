//! Identifiers handed out by the broker. Both count over all queues, so the
//! broker owns one counter of each kind.

use std::fmt;

/// `m-<n>`: identifies a message for as long as it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns the current id and moves the counter to the next one.
    pub fn take_next(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: stands for one delivery of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns the current handle and moves the counter to the next one.
    pub fn take_next(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
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
    fn receipt_handles_parse_only_canonical_text() {
        assert_eq!(ReceiptHandle::parse("r-7"), Some(ReceiptHandle(7)));
        for bad in ["r-0", "r-07", "r-+7", "r--7", "r-", "7", "m-7", "r-7 "] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId::FIRST.to_string(), "m-1");
        assert_eq!(ReceiptHandle::FIRST.to_string(), "r-1");
    }
}
