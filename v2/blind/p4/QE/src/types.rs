//! Domain values with a format rule. Each type is only constructed through
//! a check, so code that holds one never re-checks the rule.

use std::borrow::Borrow;
use std::fmt;

const NAME_MAX_LEN: usize = 80;
const FIFO_SUFFIX: &str = ".fifo";
const FIFO_BASE_MAX_LEN: usize = NAME_MAX_LEN - FIFO_SUFFIX.len();
const TOKEN_MAX_LEN: usize = 128;

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A queue name that follows the naming rules of section 4.
///
/// Ordering is byte order, which is the order `ListQueues` returns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueName(String);

impl QueueName {
    /// Returns the name if it is 1 to 80 characters made of `A-Z a-z 0-9 _ -`,
    /// or 1 to 75 such characters followed by `.fifo`.
    pub fn parse(name: &str) -> Option<Self> {
        // The suffix does not count toward the 75 characters of a FIFO base,
        // so the whole name stays within the 80-character limit either way.
        let (base, max_base_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(base) => (base, FIFO_BASE_MAX_LEN),
            None => (name, NAME_MAX_LEN),
        };
        let valid = (1..=max_base_len).contains(&base.len()) && base.chars().all(is_token_char);
        valid.then(|| QueueName(name.to_owned()))
    }

    /// True when the name ends in `.fifo`.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

/// A group identifier of a FIFO send: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(value: &str) -> Option<Self> {
        is_token(value).then(|| GroupId(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An explicit deduplication id: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(value: &str) -> Option<Self> {
        is_token(value).then(|| DeduplicationId(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= TOKEN_MAX_LEN && value.chars().all(is_token_char)
}

/// A message identifier, written `m-<n>` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(number: u64) -> Self {
        MessageId(number)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// A receipt handle, written `r-<n>` on the wire. `n` counts deliveries and
/// starts at 1, so 0 is never issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub fn new(number: u64) -> Self {
        ReceiptHandle(number)
    }

    /// The delivery number `n` of `r-<n>`.
    pub fn number(self) -> u64 {
        self.0
    }

    /// Parses the wire form exactly. `r-01`, `r-+1`, `R-1` and `r-1 ` are not
    /// handles, because a handle is compared as its exact string.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let number: u64 = digits.parse().ok()?;
        (number.to_string() == digits).then_some(ReceiptHandle(number))
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_accepts_plain_and_fifo_names() {
        assert!(QueueName::parse("jobs").is_some());
        assert!(QueueName::parse("A-b_9").is_some());
        assert!(QueueName::parse("jobs.fifo").is_some());
        assert!(QueueName::parse(&"a".repeat(80)).is_some());
        assert!(QueueName::parse(&format!("{}.fifo", "a".repeat(75))).is_some());
    }

    #[test]
    fn queue_name_rejects_bad_syntax() {
        assert!(QueueName::parse("").is_none());
        assert!(QueueName::parse(".fifo").is_none());
        assert!(QueueName::parse("a.b").is_none());
        assert!(QueueName::parse("a b").is_none());
        assert!(QueueName::parse(&"a".repeat(81)).is_none());
        assert!(QueueName::parse(&format!("{}.fifo", "a".repeat(76))).is_none());
    }

    #[test]
    fn queue_name_orders_by_bytes() {
        let mut names: Vec<QueueName> = ["b", "_", "A", "-", "0", "a", "."]
            .iter()
            .filter_map(|n| QueueName::parse(n))
            .collect();
        names.sort();
        let order: Vec<&str> = names.iter().map(QueueName::as_str).collect();
        assert_eq!(order, ["-", "0", "A", "_", "a", "b"]);
    }

    #[test]
    fn token_rules_match_section_4_limits() {
        assert!(GroupId::parse("").is_none());
        assert_eq!(
            GroupId::parse(&"g".repeat(128)).map(|g| g.as_str().len()),
            Some(128)
        );
        assert_eq!(GroupId::parse(&"g".repeat(129)), None);
        assert_eq!(DeduplicationId::parse("a.b"), None);
        assert_eq!(
            DeduplicationId::parse("ok-1_2").map(|d| d.as_str().to_owned()),
            Some("ok-1_2".to_owned())
        );
    }

    #[test]
    fn message_id_uses_wire_form() {
        assert_eq!(MessageId::new(1).to_string(), "m-1");
        assert_eq!(MessageId::new(42).to_string(), "m-42");
    }

    #[test]
    fn receipt_handle_parses_only_its_exact_form() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle::new(1)));
        assert_eq!(ReceiptHandle::parse("r-120"), Some(ReceiptHandle::new(120)));
        assert_eq!(ReceiptHandle::parse("r-0"), Some(ReceiptHandle::new(0)));
        for text in [
            "r-01",
            "r-+1",
            "R-1",
            "r-1 ",
            "r-",
            "r--1",
            "1",
            "r-1.0",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(text), None, "input {text:?}");
        }
    }

    #[test]
    fn receipt_handle_round_trips_through_display() {
        let handle = ReceiptHandle::new(7);
        assert_eq!(ReceiptHandle::parse(&handle.to_string()), Some(handle));
    }
}
