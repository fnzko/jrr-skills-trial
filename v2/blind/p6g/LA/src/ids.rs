//! Validated names and process-issued identifiers.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A queue name that follows the naming rules of `CreateQueue`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueName(String);

pub(crate) const FIFO_SUFFIX: &str = ".fifo";

pub(crate) fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

impl QueueName {
    pub fn parse(name: &str) -> Result<Self, Error> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name, 80),
        };
        if (1..=max_len).contains(&stem.len()) && stem.bytes().all(is_name_char) {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "queue names are 1 to 80 characters of A-Z a-z 0-9 _ -, optionally followed by .fifo",
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo_name(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    /// Takes the next number from `counter`, which holds the last one issued.
    pub(crate) fn issue(counter: &mut u64) -> Self {
        *counter += 1;
        Self(*counter)
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
    /// Takes the next number from `counter`, which holds the last one issued.
    pub(crate) fn issue(counter: &mut u64) -> Self {
        *counter += 1;
        Self(*counter)
    }

    /// Accepts only the exact text of a handle among the first `issued` ones.
    pub(crate) fn parse(text: &str, issued: u64) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        let n: u64 = digits.parse().ok().filter(|_| canonical)?;
        (1..=issued).contains(&n).then_some(Self(n))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("a-b_C9", true),
            ("jobs.fifo", true),
            (&"a".repeat(80), true),
            (&format!("{long_stem}.fifo"), true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("caf\u{e9}", false),
            ("a.fifo.fifo", false),
            (&"a".repeat(81), false),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn queue_name_error_code() {
        assert_eq!(
            QueueName::parse("bad name").unwrap_err().code(),
            ErrorCode::QueueNameInvalid
        );
    }

    #[test]
    fn receipt_handle_parse_is_exact() {
        let cases = [
            ("r-1", Some(1)),
            ("r-3", Some(3)),
            ("r-4", None),
            ("r-0", None),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r-", None),
            ("1", None),
            ("r-99999999999999999999999", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse(text, 3).map(|h| h.0);
            assert_eq!(parsed, expected, "{text:?}");
        }
    }

    #[test]
    fn identifiers_count_from_one() {
        let mut counter = 0;
        assert_eq!(MessageId::issue(&mut counter).to_string(), "m-1");
        assert_eq!(MessageId::issue(&mut counter).to_string(), "m-2");
        assert_eq!(ReceiptHandle::issue(&mut 4).to_string(), "r-5");
    }
}
