//! Small typed values shared by the queue model and the protocol layer.

use std::fmt;

/// Whole seconds on the virtual clock, used both for instants and durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Seconds(u64);

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn plus(self, other: Seconds) -> Seconds {
        Seconds(self.0.saturating_add(other.0))
    }

    pub fn minus(self, other: Seconds) -> Seconds {
        Seconds(self.0.saturating_sub(other.0))
    }
}

/// `m-<n>`, unique per accepted message in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`, unique per delivery in the process; `n` starts at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(n: u64) -> Self {
        Self(n)
    }

    /// Accepts only the canonical spelling `r-<n>` with `n >= 1`: no sign,
    /// no leading zeros, no surrounding characters.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let first = digits.bytes().next()?;
        if first == b'0' || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }

    /// Zero-based position of this handle in issue order.
    pub fn index(self) -> Option<usize> {
        usize::try_from(self.0.checked_sub(1)?).ok()
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// Per-FIFO-queue sequence number, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> SequenceNumber {
        SequenceNumber(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handle_accepts_only_canonical_spelling() {
        let cases = [
            ("r-1", Some(0)),
            ("r-42", Some(41)),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("r-0", None),
            ("r--1", None),
            ("r-+1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, index) in cases {
            let parsed = ReceiptHandle::parse(text).map(|h| h.index().unwrap());
            assert_eq!(parsed, index, "{text:?}");
        }
    }
}
