use std::fmt;

/// `m-<n>`: identifies an accepted message across all queues.
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

/// `r-<n>`: identifies one delivery of a message, across all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(n: u64) -> Self {
        Self(n)
    }

    /// Reads a handle written by a client, accepting only the exact text
    /// `r-<n>` (no sign, no leading zeros) of a handle below `next_unissued`.
    pub fn parse_issued(text: &str, next_unissued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (n < next_unissued).then_some(Self(n))
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}
