use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};

const MAX_QUEUE_NAME: usize = 80;
const FIFO_SUFFIX: &str = ".fifo";
const MAX_GROUP_ID: usize = 128;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `s` is 1 to 128 characters from `A-Z a-z 0-9 _ -`.
pub(crate) fn is_token(s: &str) -> bool {
    (1..=MAX_GROUP_ID).contains(&s.len()) && s.chars().all(is_name_char)
}

/// A queue name that follows the naming rules. Names given to commands other
/// than `CreateQueue` are looked up as plain `&str` and never need this type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub(crate) fn parse(name: &str) -> Result<Self, Error> {
        let (stem, max_stem) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, MAX_QUEUE_NAME - FIFO_SUFFIX.len()),
            None => (name, MAX_QUEUE_NAME),
        };
        if stem.is_empty() || stem.len() > max_stem || !stem.chars().all(is_name_char) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ));
        }
        Ok(Self(name.to_owned()))
    }

    pub(crate) fn ends_in_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

// Lookups by `&str` rely on this agreeing with the derived `Ord` and `Hash`,
// which both delegate to the inner string.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// The message group of a FIFO message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        is_token(s).then(|| Self(s.to_owned()))
    }
}

macro_rules! numbered_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "-{}"), self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    };
}

numbered_id!(
    /// Identifies a message in every queue it passes through.
    MessageId,
    "m"
);
numbered_id!(
    /// Identifies one delivery of a message.
    ReceiptHandle,
    "r"
);

/// Issues message ids and receipt handles, each counting from 1 over the
/// whole process.
#[derive(Debug, Default)]
pub(crate) struct Ids {
    messages: u64,
    receipts: u64,
}

impl Ids {
    pub(crate) fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub(crate) fn next_receipt(&mut self) -> ReceiptHandle {
        self.receipts += 1;
        ReceiptHandle(self.receipts)
    }

    /// The handle spelled exactly `s`, if this process has issued it.
    pub(crate) fn issued_receipt(&self, s: &str) -> Option<ReceiptHandle> {
        let digits = s.strip_prefix("r-")?;
        // A leading zero never matches the canonical spelling, and `r-0` was
        // never issued, so every digit string starting with 0 is rejected.
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (n <= self.receipts).then_some(ReceiptHandle(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_stem = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("Jobs_1-x", true),
            ("jobs.fifo", true),
            (&"a".repeat(80), true),
            (&format!("{long_stem}.fifo"), true),
            ("", false),
            (".fifo", false),
            ("a.fifo.fifo", false),
            ("a.b", false),
            ("a b", false),
            ("caf\u{e9}", false),
            (&"a".repeat(81), false),
            (&format!("{long_stem}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn invalid_queue_name_has_its_own_code() {
        let err = QueueName::parse("a b").unwrap_err();
        assert_eq!(err.code(), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn group_ids_are_limited_to_128_name_characters() {
        assert!(GroupId::parse("g-1_A").is_some());
        assert!(GroupId::parse(&"g".repeat(128)).is_some());
        assert!(GroupId::parse(&"g".repeat(129)).is_none());
        assert!(GroupId::parse("").is_none());
        assert!(GroupId::parse("a.b").is_none());
    }

    #[test]
    fn receipts_are_recognised_only_by_their_exact_spelling() {
        let mut ids = Ids::default();
        let first = ids.next_receipt();
        let second = ids.next_receipt();
        assert_eq!(ids.issued_receipt("r-1"), Some(first));
        assert_eq!(ids.issued_receipt("r-2"), Some(second));
        for bad in [
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-3",
            "r-",
            "r--1",
            "r-+1",
            "1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ids.issued_receipt(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        let mut ids = Ids::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_receipt().to_string(), "r-1");
    }
}
