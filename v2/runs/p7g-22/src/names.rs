//! Identifier types. Each has a validity rule, so the inner value is private
//! and construction goes through a parser.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

const FIFO_SUFFIX: &str = ".fifo";
const MAX_NAME_LEN: usize = 80;
const MAX_TOKEN_LEN: usize = 128;

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_name_part(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(is_name_byte)
}

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let valid = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => stem.len() <= MAX_NAME_LEN - FIFO_SUFFIX.len() && is_name_part(stem),
            None => text.len() <= MAX_NAME_LEN && is_name_part(text),
        };
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{text:?} is not a valid queue name"),
            ))
        }
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// Lookups by `&str` rely on `QueueName` ordering exactly like its string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn is_token(text: &str) -> bool {
    text.len() <= MAX_TOKEN_LEN && is_name_part(text)
}

/// A FIFO message group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text).then(|| Self(text.to_owned()))
    }
}

/// The key a FIFO queue deduplicates on: a client-given id or a message body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn from_id(text: &str) -> Option<Self> {
        is_token(text).then(|| Self(text.to_owned()))
    }

    /// Bodies are used as they are: any length and content.
    pub fn from_body(body: &str) -> Self {
        Self(body.to_owned())
    }
}

/// `m-<n>`, counted over the whole process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns the current id and moves on to the next one.
    pub fn take(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
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

/// `r-<n>`, counted over the whole process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Accepts only the canonical spelling `r-<n>` with `n >= 1` and no sign,
    /// padding or whitespace; anything else can never have been issued.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return None;
        }
        digits.parse().ok().map(Self)
    }

    /// Returns the current handle and moves on to the next one.
    pub fn take(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
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
    fn queue_name_syntax() {
        let long_stem = "a".repeat(75);
        let long_plain = "a".repeat(80);
        let cases = [
            ("jobs", true),
            ("Jobs_1-x", true),
            ("jobs.fifo", true),
            ("-", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("é", false),
            ("a.fifo.fifo", false),
            (long_plain.as_str(), true),
            (&format!("{long_plain}a"), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (text, valid) in cases {
            assert_eq!(QueueName::parse(text).is_ok(), valid, "name {text:?}");
        }
    }

    #[test]
    fn invalid_queue_name_reports_its_code() {
        let error = QueueName::parse("a b").unwrap_err();
        assert_eq!(error.code(), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn fifo_suffix_is_detected() {
        assert!(QueueName::parse("a.fifo").unwrap().has_fifo_suffix());
        assert!(!QueueName::parse("a").unwrap().has_fifo_suffix());
    }

    #[test]
    fn receipt_handle_accepts_only_the_canonical_spelling() {
        let cases = [
            ("r-1", Some(1)),
            ("r-42", Some(42)),
            ("r-01", None),
            ("r-0", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-+1", None),
            ("r--1", None),
            ("r-", None),
            ("r-99999999999999999999999", None),
            ("m-1", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse(text).map(|h| h.to_string());
            assert_eq!(
                parsed,
                expected.map(|n| format!("r-{n}")),
                "handle {text:?}"
            );
        }
    }

    #[test]
    fn counters_hand_out_consecutive_values() {
        let mut ids = MessageId::FIRST;
        assert_eq!(ids.take().to_string(), "m-1");
        assert_eq!(ids.take().to_string(), "m-2");
        let mut handles = ReceiptHandle::FIRST;
        assert_eq!(handles.take().to_string(), "r-1");
        assert_eq!(handles.take().to_string(), "r-2");
    }

    #[test]
    fn tokens_are_limited_to_128_name_characters() {
        assert!(GroupId::parse(&"g".repeat(128)).is_some());
        assert!(GroupId::parse(&"g".repeat(129)).is_none());
        assert!(GroupId::parse("").is_none());
        assert!(DedupKey::from_id("a.b").is_none());
        assert!(DedupKey::from_id("a-b_C9").is_some());
    }
}
