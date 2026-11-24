//! Identifiers that miniq hands out: message ids and receipt handles.
//!
//! Both are counters over the whole broker; the text forms `m-<n>` and
//! `r-<n>` exist only on the wire.

use std::fmt;

/// The identifier of a sent message (`m-<n>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// Hands out message ids in order, starting at `m-1`.
pub struct MessageIds {
    next: u64,
}

impl MessageIds {
    pub fn new() -> Self {
        Self { next: 1 }
    }

    pub fn issue(&mut self) -> MessageId {
        let id = MessageId(self.next);
        self.next += 1;
        id
    }
}

/// The receipt handle of one delivery (`r-<n>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// Hands out receipt handles in order, starting at `r-1`.
pub struct ReceiptHandles {
    next: u64,
}

impl ReceiptHandles {
    pub fn new() -> Self {
        Self { next: 1 }
    }

    pub fn issue(&mut self) -> ReceiptHandle {
        let handle = ReceiptHandle(self.next);
        self.next += 1;
        handle
    }

    /// Reads `text` as a handle exactly as it was issued (no sign, no leading
    /// zeros) and returns it only if this broker has issued it.
    pub fn parse_issued(&self, text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (number < self.next).then_some(ReceiptHandle(number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_count_from_one() {
        let mut ids = MessageIds::new();
        assert_eq!(ids.issue().to_string(), "m-1");
        assert_eq!(ids.issue().to_string(), "m-2");
        let mut handles = ReceiptHandles::new();
        assert_eq!(handles.issue().to_string(), "r-1");
    }

    #[test]
    fn only_issued_canonical_handles_parse() {
        let mut handles = ReceiptHandles::new();
        let first = handles.issue();
        let second = handles.issue();
        assert_eq!(handles.parse_issued("r-1"), Some(first));
        assert_eq!(handles.parse_issued("r-2"), Some(second));
        for bad in [
            "r-3",
            "r-0",
            "r-01",
            "r-+1",
            "r--1",
            "r-",
            "x-1",
            "1",
            "r-99999999999999999999",
        ] {
            assert_eq!(handles.parse_issued(bad), None, "{bad}");
        }
    }
}
