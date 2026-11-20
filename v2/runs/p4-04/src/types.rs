use std::borrow::Borrow;
use std::fmt;
use std::ops::Add;

use serde::Serialize;

/// A value of the virtual clock, in whole seconds since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Timestamp(u64);

/// A span of virtual time in whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Seconds(u32);

impl Seconds {
    pub const fn new(seconds: u32) -> Self {
        Seconds(seconds)
    }
}

impl Add<Seconds> for Timestamp {
    type Output = Timestamp;

    /// Saturates: the spec leaves the clock unspecified past 4000000000, and
    /// saturating keeps a long run from wrapping around to the past.
    fn add(self, span: Seconds) -> Timestamp {
        Timestamp(self.0.saturating_add(u64::from(span.0)))
    }
}

/// A queue name that satisfies the syntax of SPEC.md section 4.
///
/// Lookups by an arbitrary `&str` go through [`Borrow<str>`]; only names that
/// are stored need this type, so a name is validated once, on creation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct QueueName(String);

const FIFO_SUFFIX: &str = ".fifo";

impl QueueName {
    /// Returns the rejected string on a syntax error.
    pub fn parse(name: String) -> Result<Self, String> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name.as_str(), 80),
        };
        let valid = (1..=max_len).contains(&stem.len()) && stem.bytes().all(is_name_byte);
        if valid {
            Ok(QueueName(name))
        } else {
            Err(name)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
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

/// The character class shared by queue names, group ids and deduplication ids.
pub fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Hands out 1, 2, 3, ... in order and remembers how far it got.
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    pub fn has_issued(&self, n: u64) -> bool {
        (1..=self.0).contains(&n)
    }
}

/// `m-<n>`: the n-th message accepted by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(pub u64);

/// `r-<n>`: the n-th delivery made by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

/// The position of a message among those ever added to one FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl ReceiptHandle {
    /// Reads the exact form `r-<n>` with no sign or leading zero, so that
    /// every handle has one spelling.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return None;
        }
        digits.parse().ok().map(ReceiptHandle)
    }
}

/// The FIFO group of a message: 1 to 128 name characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    /// Returns the rejected string on a syntax error.
    pub fn parse(id: String) -> Result<Self, String> {
        if is_identifier(&id) {
            Ok(GroupId(id))
        } else {
            Err(id)
        }
    }
}

/// What a FIFO send is deduplicated by: its `DeduplicationId`, or its body
/// under content-based deduplication. Both share one key space per queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationKey(String);

impl DeduplicationKey {
    /// Returns the rejected string if it is not a valid `DeduplicationId`.
    pub fn parse_id(id: String) -> Result<Self, String> {
        if is_identifier(&id) {
            Ok(DeduplicationKey(id))
        } else {
            Err(id)
        }
    }

    pub fn of_body(body: &str) -> Self {
        DeduplicationKey(body.to_owned())
    }
}

fn is_identifier(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(is_name_byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let cases: &[(String, bool)] = &[
            ("jobs".into(), true),
            ("A-z_09".into(), true),
            ("a".repeat(80), true),
            ("a".repeat(81), false),
            ("".into(), false),
            ("has space".into(), false),
            ("dot.name".into(), false),
            ("jobs.fifo".into(), true),
            (format!("{}.fifo", "a".repeat(75)), true),
            (format!("{}.fifo", "a".repeat(76)), false),
            (".fifo".into(), false),
            ("a.fifo.fifo".into(), false),
            ("jobs.FIFO".into(), false),
            ("caf\u{e9}".into(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name.clone()).is_ok(), *valid, "{name:?}");
        }
    }

    #[test]
    fn receipt_handle_has_one_spelling() {
        let cases = [
            ("r-1", Some(1)),
            ("r-42", Some(42)),
            ("r-01", None),
            ("r-0", None),
            ("r-+1", None),
            ("r-", None),
            ("R-1", None),
            ("r-1 ", None),
            (" r-1", None),
            ("r-99999999999999999999999", None),
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
    fn identifier_syntax() {
        let cases: &[(String, bool)] = &[
            ("g".into(), true),
            ("A-z_9".into(), true),
            ("x".repeat(128), true),
            ("x".repeat(129), false),
            ("".into(), false),
            ("a.b".into(), false),
            ("a b".into(), false),
        ];
        for (id, valid) in cases {
            assert_eq!(GroupId::parse(id.clone()).is_ok(), *valid, "{id:?}");
        }
    }
}
