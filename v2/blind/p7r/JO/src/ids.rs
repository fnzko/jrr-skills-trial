//! Identifiers that must not be mixed up with each other or with counts.

use std::fmt;

/// The number behind a `MessageId` (`m-<n>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// The number behind a `ReceiptHandle` (`r-<n>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
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

/// The position of a message in its queue's arrival order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalIndex(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_as_issued() {
        assert_eq!(ReceiptHandle::parse("r-7"), Some(ReceiptHandle(7)));
        for bad in [
            "r-",
            "r-07",
            "r-+7",
            "r--7",
            "m-7",
            "7",
            "r-7 ",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }
}
