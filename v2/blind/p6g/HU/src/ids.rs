use std::fmt;

/// Identifies an accepted message, over all queues: `m-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// Identifies one delivery of a message, over all queues: `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReceiptHandle(pub u64);

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl ReceiptHandle {
    /// The handle written as `text`, if `text` is exactly the form that one of
    /// the first `issued` deliveries got (so not `r-01`, `R-1` or `r-1 `).
    pub fn parse(text: &str, issued: u64) -> Option<Self> {
        let handle = Self(text.strip_prefix("r-")?.parse().ok()?);
        (handle.to_string() == text && (1..=issued).contains(&handle.0)).then_some(handle)
    }
}
