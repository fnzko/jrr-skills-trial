//! Values with a format rule from the spec, parsed once where they enter.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Serialize, Serializer};

/// A point on the virtual clock, in whole seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    pub fn plus(self, seconds: u32) -> Time {
        Time(self.0 + u64::from(seconds))
    }
}

/// A queue name that follows the naming rules of `CreateQueue`.
///
/// Orders by byte value, which is the order `ListQueues` reports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(name: &str) -> Option<QueueName> {
        let base = name.strip_suffix(".fifo").unwrap_or(name);
        let base_ok = !base.is_empty() && base.bytes().all(is_name_byte);
        // A `.fifo` name is at most 75 + 5 bytes, so 80 bounds both forms.
        (base_ok && name.len() <= 80).then(|| QueueName(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(".fifo")
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

/// The characters allowed in queue names, group ids and deduplication ids.
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// A `GroupId` or `DeduplicationId`: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Tag(String);

impl Tag {
    pub fn parse(text: &str) -> Option<Tag> {
        let ok = (1..=128).contains(&text.len()) && text.bytes().all(is_name_byte);
        ok.then(|| Tag(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `m-<n>`: the n-th message accepted by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

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

/// `r-<n>`: the n-th delivery made by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

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

/// The text is not exactly what `Display` writes for some handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAHandle;

impl FromStr for ReceiptHandle {
    type Err = NotAHandle;

    /// Accepts only the canonical form, since handles compare as exact strings:
    /// `r-01` or `r-+1` never name handle 1.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.strip_prefix("r-").ok_or(NotAHandle)?;
        let canonical = !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit());
        if !canonical {
            return Err(NotAHandle);
        }
        // Too many digits to fit: no such handle can have been issued.
        digits.parse().map(ReceiptHandle).map_err(|_| NotAHandle)
    }
}

/// Hands out message ids 1, 2, 3, ... over all queues.
#[derive(Debug, Default)]
pub struct MessageIds {
    last: u64,
}

impl MessageIds {
    pub fn next(&mut self) -> MessageId {
        self.last += 1;
        MessageId(self.last)
    }
}

/// Hands out receipt handles 1, 2, 3, ... over all queues.
#[derive(Debug, Default)]
pub struct ReceiptHandles {
    last: u64,
}

impl ReceiptHandles {
    pub fn next(&mut self) -> ReceiptHandle {
        self.last += 1;
        ReceiptHandle(self.last)
    }

    pub fn has_issued(&self, handle: ReceiptHandle) -> bool {
        handle.0 <= self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules() {
        let long_base = "a".repeat(75);
        let cases = [
            ("jobs", true),
            ("A-z_09", true),
            ("jobs.fifo", true),
            ("", false),
            (".fifo", false),
            ("a.fifo.fifo", false),
            ("jobs.txt", false),
            ("jo bs", false),
            ("j\u{e9}", false),
            (&*"a".repeat(80), true),
            (&*"a".repeat(81), false),
            (&*format!("{long_base}.fifo"), true),
            (&*format!("{long_base}a.fifo"), false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "name {name:?}");
        }
    }

    #[test]
    fn tag_rules() {
        let cases = [
            ("g1", true),
            ("A-_z", true),
            ("", false),
            ("a.b", false),
            (&*"x".repeat(128), true),
            (&*"x".repeat(129), false),
        ];
        for (text, valid) in cases {
            assert_eq!(Tag::parse(text).is_some(), valid, "tag {text:?}");
        }
    }

    #[test]
    fn receipt_handle_accepts_only_canonical_text() {
        assert_eq!("r-1".parse(), Ok(ReceiptHandle(1)));
        assert_eq!("r-907".parse(), Ok(ReceiptHandle(907)));
        for text in [
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-",
            "r-+1",
            "r--1",
            "m-1",
            "r-1.0",
            "r-99999999999999999999999",
        ] {
            assert_eq!(
                text.parse::<ReceiptHandle>(),
                Err(NotAHandle),
                "text {text:?}"
            );
        }
    }

    #[test]
    fn receipt_handles_are_issued_from_one() {
        let mut handles = ReceiptHandles::default();
        assert!(!handles.has_issued(ReceiptHandle(1)));
        assert_eq!(handles.next().to_string(), "r-1");
        assert_eq!(handles.next().to_string(), "r-2");
        assert!(handles.has_issued(ReceiptHandle(2)));
        assert!(!handles.has_issued(ReceiptHandle(3)));
    }
}
