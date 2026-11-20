//! Strings with a validity rule. Each type can only be built through `parse`.

use std::borrow::Borrow;

use serde::Serialize;

const FIFO_SUFFIX: &str = ".fifo";

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_token(text: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&text.len()) && text.bytes().all(is_token_byte)
}

/// A queue name that follows the naming rules. Ordering is by byte value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Option<Self> {
        let valid = match text.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => is_token(stem, 75),
            None => is_token(text, 80),
        };
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name carries the suffix reserved for FIFO queues.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

// Lets maps keyed by `QueueName` be searched with any `&str`; the derived
// `Ord` and `Hash` of the wrapper agree with those of `str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A message body of 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body(String);

impl Body {
    pub fn parse(text: &str) -> Option<Self> {
        (1..=1024)
            .contains(&text.len())
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The group of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text, 128).then(|| Self(text.to_owned()))
    }
}

/// A caller-chosen deduplication id of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(text: &str) -> Option<Self> {
        is_token(text, 128).then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The key under which a queue remembers a send for deduplication. A body used
/// as an id and a `DeduplicationId` share one namespace, so this has no
/// validity rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn new(text: &str) -> Self {
        Self(text.to_owned())
    }
}

// Same contract as for `QueueName`: lookups by `&str` need no allocation.
impl Borrow<str> for DedupKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_plain = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_plain = "a".repeat(81);
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A_z-9", true),
            ("jobs.fifo", true),
            (long_plain.as_str(), true),
            (long_fifo.as_str(), true),
            ("", false),
            (".fifo", false),
            ("a.fifo.fifo", false),
            ("a.b", false),
            ("jobs fifo", false),
            ("caf\u{e9}", false),
            (too_long_plain.as_str(), false),
            (too_long_fifo.as_str(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "name {name:?}");
        }
    }

    #[test]
    fn queue_names_sort_by_byte_value() {
        let mut names: Vec<QueueName> = ["a", "_", "Z", "9", "a.fifo", "a-"]
            .into_iter()
            .map(|n| QueueName::parse(n).expect("valid name"))
            .collect();
        names.sort();
        let sorted: Vec<&str> = names.iter().map(QueueName::as_str).collect();
        assert_eq!(sorted, ["9", "Z", "_", "a", "a-", "a.fifo"]);
    }

    #[test]
    fn body_length_is_counted_in_bytes() {
        let two_byte_chars = "\u{e9}".repeat(512);
        let one_too_many = format!("{two_byte_chars}x");
        assert!(Body::parse(&two_byte_chars).is_some());
        assert!(Body::parse(&one_too_many).is_none());
        assert!(Body::parse("").is_none());
    }

    #[test]
    fn group_ids_follow_the_token_rule() {
        let cases = [
            ("g-1_", true),
            ("", false),
            ("a b", false),
            (&"x".repeat(129), false),
        ];
        for (text, valid) in cases {
            assert_eq!(GroupId::parse(text).is_some(), valid, "id {text:?}");
        }
    }
}
