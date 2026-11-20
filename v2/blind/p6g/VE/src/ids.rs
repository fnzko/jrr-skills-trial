use std::fmt;

use serde::{Serialize, Serializer};

/// `m-<n>`: identifies an accepted message across all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageId(u64);

/// `r-<n>`: identifies one delivery of a message across all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReceiptHandle(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl ReceiptHandle {
    /// Accepts only the canonical spelling `r-<n>`: `n >= 1`, digits only,
    /// no leading zero. Anything else was never issued.
    fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

/// Hands out the process-wide counters behind message ids and receipt handles.
#[derive(Debug, Default)]
pub(crate) struct IdSource {
    messages: u64,
    deliveries: u64,
}

impl IdSource {
    pub(crate) fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub(crate) fn next_handle(&mut self) -> ReceiptHandle {
        self.deliveries += 1;
        ReceiptHandle(self.deliveries)
    }

    /// The handle spelled by `text`, if this process has issued it.
    pub(crate) fn issued_handle(&self, text: &str) -> Option<ReceiptHandle> {
        ReceiptHandle::parse(text).filter(|handle| handle.0 <= self.deliveries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_issued_handles_are_recognized() {
        let mut ids = IdSource::default();
        let first = ids.next_handle();
        let second = ids.next_handle();

        assert_eq!(ids.issued_handle("r-1"), Some(first));
        assert_eq!(ids.issued_handle("r-2"), Some(second));
        for text in [
            "r-3",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            "r-",
            "r--1",
            "r-+1",
            "m-1",
            "",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ids.issued_handle(text), None, "{text:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        let mut ids = IdSource::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_handle().to_string(), "r-1");
    }
}
