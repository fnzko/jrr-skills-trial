//! Character and name rules shared by queue names and message identifiers.

use std::borrow::Borrow;
use std::fmt;

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
    /// `None` if `name` breaks the syntax.
    pub fn parse(name: &str) -> Option<Self> {
        let valid = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => is_identifier(stem, MAX_QUEUE_NAME_LEN - FIFO_SUFFIX.len()),
            None => is_identifier(name, MAX_QUEUE_NAME_LEN),
        };
        valid.then(|| Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True if the name denotes a FIFO queue by its suffix.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

/// Lets the queue map be searched with the unchecked name of a request.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for QueueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let valid = [
            "jobs".to_owned(),
            "a-b_C9".to_owned(),
            "jobs.fifo".to_owned(),
            "a".repeat(80),
            format!("{}.fifo", "a".repeat(75)),
        ];
        for name in &valid {
            assert_eq!(QueueName::parse(name).unwrap().to_string(), *name);
        }
        let invalid = [
            String::new(),
            ".fifo".to_owned(),
            "a.b".to_owned(),
            "a.fifo.fifo".to_owned(),
            "a".repeat(81),
            format!("{}.fifo", "a".repeat(76)),
            "jöbs".to_owned(),
            "a b".to_owned(),
        ];
        for name in &invalid {
            assert_eq!(QueueName::parse(name), None, "{name}");
        }
    }

    #[test]
    fn only_the_suffix_makes_a_queue_name_fifo() {
        assert!(QueueName::parse("jobs.fifo").unwrap().has_fifo_suffix());
        assert!(!QueueName::parse("jobs").unwrap().has_fifo_suffix());
    }

    #[test]
    fn identifiers_count_characters_not_bytes() {
        assert!(is_identifier(&"x".repeat(128), 128));
        assert!(!is_identifier(&"x".repeat(129), 128));
        assert!(!is_identifier("é", 128));
    }
}
