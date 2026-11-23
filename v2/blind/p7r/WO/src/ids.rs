//! Identifiers that clients see as `m-<n>` and `r-<n>` strings.

use std::fmt;

/// The identifier of a message (`m-<n>`), unique over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// A receipt handle (`r-<n>`), unique over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
    /// Parses `r-<n>` exactly as it is issued: no sign, no leading zeros.
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
    fn receipt_handles_round_trip_only_in_issued_form() {
        assert_eq!(ReceiptHandle::parse("r-7"), Some(ReceiptHandle(7)));
        assert_eq!(ReceiptHandle(7).to_string(), "r-7");
        for bad in ["r-0", "r-07", "r-+7", "r--7", "r-", "r-7 ", "m-7", "7"] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn message_ids_display_with_prefix() {
        assert_eq!(MessageId(12).to_string(), "m-12");
    }
}
