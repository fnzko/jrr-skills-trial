use std::borrow::Borrow;

use serde::Serialize;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that obeys the naming rules; only `CreateQueue` needs one.
/// Lookups elsewhere use plain `&str` because any string may name a queue
/// that does not exist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct QueueName(String);

impl QueueName {
    const FIFO_SUFFIX: &'static str = ".fifo";

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (stem, max_len) = match text.strip_suffix(Self::FIFO_SUFFIX) {
            Some(stem) => (stem, 80 - Self::FIFO_SUFFIX.len()),
            None => (text, 80),
        };
        let valid = (1..=max_len).contains(&stem.len()) && stem.chars().all(is_name_char);
        valid.then(|| Self(text.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_fifo(&self) -> bool {
        self.0.ends_with(Self::FIFO_SUFFIX)
    }
}

// Lets the queue map be searched by `&str`; `Ord` and `Hash` of the wrapped
// `String` agree with those of `str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Message group of a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct GroupId(String);

impl GroupId {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        is_message_token(text).then(|| Self(text.to_owned()))
    }
}

/// Character and length rule shared by group ids and deduplication ids.
pub(crate) fn is_message_token(text: &str) -> bool {
    (1..=128).contains(&text.len()) && text.chars().all(is_name_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_stem = "a".repeat(75);
        let long_plain = "a".repeat(80);
        let cases = [
            ("jobs", Some(false)),
            ("Jobs_1-x", Some(false)),
            ("jobs.fifo", Some(true)),
            (long_plain.as_str(), Some(false)),
            (&format!("{long_stem}.fifo"), Some(true)),
            ("", None),
            (".fifo", None),
            ("a.b", None),
            ("a.fifo.fifo", None),
            ("a b", None),
            ("caf\u{e9}", None),
            (&"a".repeat(81), None),
            (&format!("{}.fifo", "a".repeat(76)), None),
        ];
        for (text, fifo) in cases {
            assert_eq!(
                QueueName::parse(text).map(|name| name.is_fifo()),
                fifo,
                "{text:?}"
            );
        }
    }

    #[test]
    fn message_token_rule() {
        assert!(is_message_token("a"));
        assert!(is_message_token(&"Z".repeat(128)));
        assert!(!is_message_token(""));
        assert!(!is_message_token(&"Z".repeat(129)));
        assert!(!is_message_token("a.b"));
    }
}
