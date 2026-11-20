//! Process-wide identifiers: message ids and receipt handles.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Reads a handle sent by a client. Only the exact text `r-<n>` of a handle
    /// that was issued is accepted: no sign, no leading zeros, no whitespace.
    pub fn parse(text: &str, issued: &Ids) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (1..=issued.receipts)
            .contains(&number)
            .then_some(Self(number))
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// Counters behind the identifiers; one per process, shared by all queues.
#[derive(Debug, Default)]
pub struct Ids {
    messages: u64,
    receipts: u64,
}

impl Ids {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_count_independently_from_one() {
        let mut ids = Ids::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_receipt().to_string(), "r-1");
    }

    #[test]
    fn only_exact_issued_handles_parse() {
        let mut ids = Ids::default();
        let issued: Vec<_> = (0..12).map(|_| ids.next_receipt()).collect();
        assert_eq!(ReceiptHandle::parse("r-1", &ids), Some(issued[0]));
        assert_eq!(ReceiptHandle::parse("r-12", &ids), Some(issued[11]));
        for bad in [
            "r-0", "r-13", "r-01", "R-1", "r-1 ", " r-1", "r-+1", "r-", "r--1", "1", "",
        ] {
            assert_eq!(ReceiptHandle::parse(bad, &ids), None, "{bad:?}");
        }
    }
}
