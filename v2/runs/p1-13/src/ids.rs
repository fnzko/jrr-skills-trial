//! Identifiers issued by the process.

use std::fmt;

use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        Self(number)
    }

    /// Recognizes exactly the strings `Display` can produce for a number up to
    /// `issued`: no sign, no leading zero, no surrounding characters.
    pub fn parse(text: &str, issued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        (number <= issued).then_some(Self(number))
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Position of a message among those added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_canonical_issued_numbers() {
        let cases = [
            ("r-1", Some(1)),
            ("r-12", Some(12)),
            ("r-13", None),
            ("r-0", None),
            ("r-01", None),
            ("r-+1", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            let actual = ReceiptHandle::parse(text, 12);
            assert_eq!(actual, expected.map(ReceiptHandle::new), "handle {text:?}");
        }
    }

    #[test]
    fn ids_render_with_their_prefix() {
        assert_eq!(MessageId::new(7).to_string(), "m-7");
        assert_eq!(ReceiptHandle::new(3).to_string(), "r-3");
    }
}
