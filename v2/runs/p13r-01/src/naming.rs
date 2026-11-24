//! Character and name rules shared by queue names and message identifiers.

use std::borrow::Borrow;

const FIFO_SUFFIX: &str = ".fifo";
const MAX_QUEUE_NAME_LEN: usize = 80;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// True if `s` is made of 1 to `max_len` characters from `A-Z a-z 0-9 _ -`.
pub fn is_identifier(s: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&s.chars().count()) && s.chars().all(is_name_char)
}

/// A name that follows the queue name syntax (SPEC section 4).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueName(String);

impl QueueName {
    /// `None` if `name` breaks the queue name syntax.
    pub fn parse(name: &str) -> Option<Self> {
        let valid = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => is_identifier(stem, MAX_QUEUE_NAME_LEN - FIFO_SUFFIX.len()),
            None => is_identifier(name, MAX_QUEUE_NAME_LEN),
        };
        valid.then(|| Self(name.to_owned()))
    }

    /// True if the name denotes a FIFO queue by its suffix.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lets the queue map be searched by the plain names that requests carry.
/// Equality, ordering and hashing agree with `str`, as `Borrow` requires.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_queue_names_are_kept_as_given() {
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        for (name, fifo) in [
            ("jobs", false),
            ("a-b_C9", false),
            ("jobs.fifo", true),
            (&"a".repeat(80), false),
            (&long_fifo, true),
        ] {
            let parsed = QueueName::parse(name).expect(name);
            assert_eq!(parsed.as_str(), name);
            assert_eq!(parsed.has_fifo_suffix(), fifo, "{name}");
        }
    }

    #[test]
    fn invalid_queue_names_are_rejected() {
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        for name in [
            "",
            ".fifo",
            "a.b",
            "a.fifo.fifo",
            &"a".repeat(81),
            &too_long_fifo,
            "jöbs",
            "a b",
        ] {
            assert_eq!(QueueName::parse(name), None, "{name}");
        }
    }

    #[test]
    fn identifiers_count_characters_not_bytes() {
        assert!(is_identifier(&"x".repeat(128), 128));
        assert!(!is_identifier(&"x".repeat(129), 128));
        assert!(!is_identifier("é", 128));
    }
}
