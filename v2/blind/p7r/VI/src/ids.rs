//! Identifiers handed out by the broker. Each kind has its own type so that a
//! receipt handle can never be used where an arrival index is meant.

use std::fmt;

/// `m-<n>`: counts the messages accepted by the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: counts the deliveries made by the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
    /// Parses a handle exactly as it was issued: no sign, no leading zeros.
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

/// The position in which a message arrived in its queue; it never changes
/// and is never reused within one queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_round_trip_only_in_canonical_form() {
        assert_eq!(ReceiptHandle::parse("r-17"), Some(ReceiptHandle(17)));
        assert_eq!(ReceiptHandle(17).to_string(), "r-17");
        for bad in [
            "r-01",
            "R-1",
            "r-1 ",
            "r-",
            "r--1",
            "r-+1",
            "1",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }
}
