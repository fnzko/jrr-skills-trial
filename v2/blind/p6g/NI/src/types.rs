//! Small value types: time, identifiers and validated strings.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Add;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

/// A point in virtual time or a duration, in whole seconds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u64);

impl Seconds {
    pub const ZERO: Self = Self(0);
}

impl From<u32> for Seconds {
    fn from(value: u32) -> Self {
        Self(value.into())
    }
}

impl Add for Seconds {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

/// `m-<n>`, unique across all queues of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

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
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    /// Parses the canonical text of a handle; any other spelling was never issued.
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

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Hands out receipt handles and knows which ones exist.
#[derive(Debug, Default)]
pub struct Handles {
    issued: u64,
}

impl Handles {
    pub fn issue(&mut self) -> ReceiptHandle {
        self.issued += 1;
        ReceiptHandle(self.issued)
    }

    pub fn is_issued(&self, handle: ReceiptHandle) -> bool {
        (1..=self.issued).contains(&handle.0)
    }
}

/// Position of a message in a FIFO queue, counted per queue from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(pub u64);

const FIFO_SUFFIX: &str = ".fifo";

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let stem = text.strip_suffix(FIFO_SUFFIX).unwrap_or(text);
        let valid = !stem.is_empty()
            && stem.len() <= if stem.len() == text.len() { 80 } else { 75 }
            && stem.chars().all(is_name_char);
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!(
                    "queue name {text:?} must be 1 to 80 characters of A-Z a-z 0-9 _ -, \
                     optionally followed by .fifo"
                ),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Lets a map keyed by `QueueName` be searched with any `&str`; the derived
// ordering and hashing of the inner string agree with those of `str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Checks the rule shared by `GroupId` and `DeduplicationId`: 1 to 128
/// characters of `A-Z a-z 0-9 _ -`.
pub fn validate_token<'a>(text: &'a str, parameter: &str) -> Result<&'a str, Error> {
    if (1..=128).contains(&text.len()) && text.chars().all(is_name_char) {
        Ok(text)
    } else {
        Err(Error::invalid_value(format!(
            "{parameter} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
        )))
    }
}

/// The ordering group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        validate_token(text, "GroupId").map(|t| Self(t.to_owned()))
    }
}

/// A message body: 1 to 1024 bytes of UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if (1..=1024).contains(&text.len()) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::invalid_value("Body must be 1 to 1024 bytes long"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_stem = "a".repeat(75);
        let cases: [(String, bool); 12] = [
            ("jobs".into(), true),
            ("A-z_9".into(), true),
            ("a".repeat(80), true),
            ("a".repeat(81), false),
            (format!("{long_stem}.fifo"), true),
            (format!("{long_stem}a.fifo"), false),
            ("jobs.fifo".into(), true),
            (".fifo".into(), false),
            ("".into(), false),
            ("a.b".into(), false),
            ("a.fifo.fifo".into(), false),
            ("caf\u{e9}".into(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(&name).is_ok(), valid, "{name}");
        }
    }

    #[test]
    fn only_canonical_handles_parse() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle(1)));
        for text in [
            "r-01",
            "R-1",
            "r-1 ",
            "r-",
            "r-0",
            "r-+1",
            "1",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(text), None, "{text}");
        }
    }

    #[test]
    fn handles_are_issued_in_order() {
        let mut handles = Handles::default();
        let first = handles.issue();
        assert!(handles.is_issued(first));
        assert!(!handles.is_issued(ReceiptHandle(2)));
        assert_eq!(handles.issue(), ReceiptHandle(2));
        assert!(handles.is_issued(ReceiptHandle(2)));
    }

    #[test]
    fn bodies_are_limited_in_bytes() {
        assert!(Body::parse("").is_err());
        assert!(Body::parse(&"a".repeat(1024)).is_ok());
        assert!(Body::parse(&"\u{e9}".repeat(513)).is_err());
    }
}
