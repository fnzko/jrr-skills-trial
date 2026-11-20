//! Identifiers handed out by the process: message ids and receipt handles.

use std::fmt;

use serde::{Serialize, Serializer};

/// `m-<n>`, unique over all queues of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageId(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// `r-<n>`, one per delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Parses a handle the process has issued: the exact canonical text of
    /// one of the first `issued` handles, so `r-01` or `r-1 ` are rejected.
    pub(crate) fn parse(text: &str, issued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (1..=issued).contains(&n).then_some(Self(n))
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("r-{}", self.0))
    }
}

/// Process-wide counters behind `MessageId` and `ReceiptHandle`.
#[derive(Debug, Default)]
pub(crate) struct Ids {
    messages: u64,
    receipts: u64,
}

impl Ids {
    pub(crate) fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub(crate) fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    /// How many receipt handles have been issued so far.
    pub(crate) fn receipts_issued(&self) -> u64 {
        self.receipts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_only_issued_canonical_handles() {
        let accepted = [("r-1", true), ("r-3", true), ("r-4", false), ("r-0", false)];
        for (text, ok) in accepted {
            assert_eq!(ReceiptHandle::parse(text, 3).is_some(), ok, "{text}");
        }
        for text in [
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-+1",
            "r-",
            "1",
            "",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(text, 3), None, "{text:?}");
        }
    }

    #[test]
    fn counters_start_at_one() {
        let mut ids = Ids::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_receipt(), ReceiptHandle(1));
        assert_eq!(ids.receipts_issued(), 1);
    }
}
