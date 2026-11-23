//! Validated names and process-wide identifier counters.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

const FIFO_SUFFIX: &str = ".fifo";
const MAX_PLAIN_NAME_LEN: usize = 80;
const MAX_FIFO_STEM_LEN: usize = 75;
const MAX_TOKEN_LEN: usize = 128;

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `text` is 1 to 128 characters of `A-Z a-z 0-9 _ -`.
pub fn is_token(text: &str) -> bool {
    (1..=MAX_TOKEN_LEN).contains(&text.len()) && text.chars().all(is_token_char)
}

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    /// Checks the name syntax of the specification.
    pub fn parse(name: &str) -> Result<Self, Error> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, MAX_FIFO_STEM_LEN),
            None => (name, MAX_PLAIN_NAME_LEN),
        };
        if (1..=max_len).contains(&stem.len()) && stem.chars().all(is_token_char) {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue name {name:?} breaks the naming rules"),
            ))
        }
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name carries the `.fifo` suffix.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Lets maps keyed by `QueueName` be searched with a plain `&str`; `String`
// hashes and orders exactly like `str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// The message group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl GroupId {
    /// Accepts 1 to 128 characters of `A-Z a-z 0-9 _ -`.
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text).then(|| Self(text.to_owned()))
    }
}

/// `m-<n>`, unique over the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

/// `r-<n>`, one per delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

/// Position of a message among those added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(pub u64);

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
    /// Reads the one canonical spelling `r-<n>` with `n >= 1`: no sign, no
    /// leading zero, no spaces.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

/// Hands out `MessageId`s and `ReceiptHandle`s.
#[derive(Debug)]
pub struct Counters {
    next_message: u64,
    next_handle: u64,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            next_message: 1,
            next_handle: 1,
        }
    }
}

impl Counters {
    /// The next `MessageId`.
    pub fn message_id(&mut self) -> MessageId {
        let id = MessageId(self.next_message);
        self.next_message += 1;
        id
    }

    /// The next `ReceiptHandle`.
    pub fn receipt_handle(&mut self) -> ReceiptHandle {
        let handle = ReceiptHandle(self.next_handle);
        self.next_handle += 1;
        handle
    }

    /// Reads `text` as a handle this process has handed out.
    pub fn issued_handle(&self, text: &str) -> Result<ReceiptHandle, Error> {
        ReceiptHandle::parse(text)
            .filter(|handle| handle.0 < self.next_handle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("A_b-9", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a b", false),
            ("é", false),
            ("a.fifo.fifo", false),
            (&"a".repeat(80), true),
            (&"a".repeat(81), false),
            (&format!("{long_stem}.fifo"), true),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "name {name:?}");
        }
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
            ("r-", None),
            ("r-99999999999999999999999", None),
            ("m-1", None),
        ];
        for (text, expected) in cases {
            assert_eq!(
                ReceiptHandle::parse(text),
                expected.map(ReceiptHandle),
                "{text:?}"
            );
        }
    }

    #[test]
    fn counters_count_from_one_and_track_issued_handles() {
        let mut counters = Counters::default();
        assert_eq!(counters.message_id().to_string(), "m-1");
        assert_eq!(counters.message_id().to_string(), "m-2");
        let first = counters.receipt_handle();
        assert_eq!(first.to_string(), "r-1");
        assert_eq!(counters.issued_handle("r-1"), Ok(first));
        assert_eq!(
            counters
                .issued_handle("r-2")
                .expect_err("not issued yet")
                .code(),
            ErrorCode::ReceiptHandleInvalid
        );
    }
}
