//! Typed values of SPEC.md. A type with a validity rule can only be built through a checked
//! constructor, so holding one is proof that the rule holds.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{ErrorCode, Failure};

/// Largest body, in bytes, of a message (section 6).
const MAX_BODY_BYTES: usize = 1024;
/// Longest group id or deduplication id, in characters (section 6).
const MAX_TOKEN_CHARS: usize = 128;
/// Suffix that marks a FIFO queue name (section 4).
const FIFO_SUFFIX: &str = ".fifo";

/// A duration in whole seconds. Every attribute and `Tick` takes one; each has its own range,
/// checked where the value is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u32);

/// The virtual clock of section 3, in whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Clock(u64);

impl Clock {
    /// Upper bound of the clock. Section 3 leaves behavior past 4000000000 unspecified, so `Tick`
    /// refuses to go beyond it. That keeps every sum of a clock and a `Seconds` far inside u64.
    pub const LIMIT: u64 = 4_000_000_000;

    pub const START: Clock = Clock(0);

    /// The clock `seconds` later, or None if that would pass `LIMIT`.
    pub fn advanced(self, seconds: Seconds) -> Option<Clock> {
        let next = self.0.checked_add(u64::from(seconds.0))?;
        (next <= Self::LIMIT).then_some(Clock(next))
    }

    /// The clock `seconds` after this one. Clocks never exceed `LIMIT`, so the sum fits in u64.
    pub fn after(self, seconds: Seconds) -> Clock {
        Clock(
            self.0
                .checked_add(u64::from(seconds.0))
                .expect("a clock is at most LIMIT and a Seconds is at most u32::MAX"),
        )
    }
}

/// A message identifier, `m-<n>` on the wire (section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: MessageId = MessageId(1);

    pub fn next(self) -> MessageId {
        MessageId(
            self.0
                .checked_add(1)
                .expect("message ids count accepted messages and cannot reach u64::MAX"),
        )
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

/// A position in a FIFO queue's sequence (section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    pub fn next(self) -> SequenceNumber {
        SequenceNumber(
            self.0.checked_add(1).expect(
                "sequence numbers count messages added to one queue and cannot reach u64::MAX",
            ),
        )
    }
}

/// A receipt handle, `r-<n>` on the wire (section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: ReceiptHandle = ReceiptHandle(1);

    /// Reads the exact text `r-<n>`, with n written in decimal without leading zeros. Any other
    /// text is None: no handle with that text was ever issued.
    pub fn parse(text: &str) -> Option<ReceiptHandle> {
        let digits = text.strip_prefix("r-")?;
        let canonical = !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit());
        if !canonical {
            return None;
        }
        // A number too large for u64 was never issued either, so the parse error means None.
        digits.parse::<u64>().ok().map(ReceiptHandle)
    }

    pub fn next(self) -> ReceiptHandle {
        ReceiptHandle(
            self.0
                .checked_add(1)
                .expect("receipt handles count deliveries and cannot reach u64::MAX"),
        )
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

/// The name of an existing queue. Only `CreateQueue` must produce one; other commands look
/// names up as given (see `QueueRef`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    /// True when the name ends in `.fifo`, which marks a FIFO queue.
    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for QueueName {
    type Error = Failure;

    fn try_from(name: &str) -> Result<Self, Failure> {
        if is_valid_queue_name(name) {
            Ok(QueueName(name.to_owned()))
        } else {
            Err(Failure::new(
                ErrorCode::QueueNameInvalid,
                "a queue name is 1 to 80 characters from A-Z a-z 0-9 _ -, or 1 to 75 of them followed by .fifo",
            ))
        }
    }
}

/// Lets a map keyed by `QueueName` be searched by `&str`, without building a name first.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A queue name as a request spelled it. Any string is accepted: a string that breaks the name
/// rules matches no queue, and the lookup reports that as `QueueDoesNotExist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRef(String);

impl QueueRef {
    pub fn new(text: String) -> QueueRef {
        QueueRef(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_queue_name(name: &str) -> bool {
    let (stem, allowed_len) = match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) => (stem, 1..=75),
        None => (name, 1..=80),
    };
    allowed_len.contains(&stem.len()) && stem.bytes().all(is_word_byte)
}

/// A message body: 1 to 1024 bytes of UTF-8 (section 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Body {
    type Error = Failure;

    fn try_from(text: String) -> Result<Self, Failure> {
        if (1..=MAX_BODY_BYTES).contains(&text.len()) {
            Ok(Body(text))
        } else {
            Err(Failure::new(
                ErrorCode::InvalidParameterValue,
                "Body must be 1 to 1024 bytes",
            ))
        }
    }
}

/// A message group of a FIFO queue (section 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GroupId(String);

impl TryFrom<String> for GroupId {
    type Error = Failure;

    fn try_from(text: String) -> Result<Self, Failure> {
        check_token("GroupId", &text)?;
        Ok(GroupId(text))
    }
}

/// A deduplication id given explicitly in a send (section 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeduplicationId {
    type Error = Failure;

    fn try_from(text: String) -> Result<Self, Failure> {
        check_token("DeduplicationId", &text)?;
        Ok(DeduplicationId(text))
    }
}

/// The key that a FIFO send is deduplicated by. Explicit deduplication ids and message bodies
/// share one namespace (section 6), so both become a `DedupKey`. Lookups use `&str`, so a key is
/// only built when a send is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn new(text: &str) -> DedupKey {
        DedupKey(text.to_owned())
    }
}

impl Borrow<str> for DedupKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

fn check_token(parameter: &str, text: &str) -> Result<(), Failure> {
    if (1..=MAX_TOKEN_CHARS).contains(&text.len()) && text.bytes().all(is_word_byte) {
        Ok(())
    } else {
        Err(Failure::new(
            ErrorCode::InvalidParameterValue,
            format!("{parameter} must be 1 to 128 characters from A-Z a-z 0-9 _ -"),
        ))
    }
}

/// The characters allowed in queue names and tokens.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_rules_table() {
        let cases: [(&str, bool); 9] = [
            ("jobs", true),
            ("A_b-9", true),
            ("a.fifo", true),
            (&"x".repeat(80), true),
            (&"x".repeat(81), false),
            (&format!("{}.fifo", "x".repeat(75)), true),
            (&format!("{}.fifo", "x".repeat(76)), false),
            (".fifo", false),
            ("a.b", false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::try_from(name).is_ok(), valid, "name {name:?}");
        }
    }

    #[test]
    fn queue_name_refusal_has_queue_name_invalid_code() {
        let failure = QueueName::try_from("bad name").expect_err("space is not a name character");
        assert_eq!(failure.code, ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn token_rules_table() {
        let cases: [(String, bool); 5] = [
            ("g1".to_owned(), true),
            ("".to_owned(), false),
            ("x".repeat(128), true),
            ("x".repeat(129), false),
            ("a.b".to_owned(), false),
        ];
        for (text, valid) in cases {
            assert_eq!(
                GroupId::try_from(text.clone()).is_ok(),
                valid,
                "token {text:?}"
            );
        }
    }

    #[test]
    fn body_size_rules_table() {
        let cases: [(String, bool); 3] = [
            (String::new(), false),
            ("x".to_owned(), true),
            ("é".repeat(512), true),
        ];
        for (text, valid) in cases {
            assert_eq!(Body::try_from(text.clone()).is_ok(), valid, "body {text:?}");
        }
        assert!(Body::try_from("x".repeat(1025)).is_err());
    }

    #[test]
    fn receipt_handle_parse_table() {
        let cases: [(&str, Option<u64>); 8] = [
            ("r-1", Some(1)),
            ("r-42", Some(42)),
            ("r-01", None),
            ("R-1", None),
            ("r-1 ", None),
            ("r-", None),
            ("r-+1", None),
            ("r-18446744073709551616", None),
        ];
        for (text, expected) in cases {
            let parsed = ReceiptHandle::parse(text).map(|handle| handle.to_string());
            let expected = expected.map(|n| format!("r-{n}"));
            assert_eq!(parsed, expected, "text {text:?}");
        }
    }

    #[test]
    fn clock_stops_at_the_limit_exactly() {
        // 3306 full Ticks reach 3998937600, which leaves 1062400 seconds below LIMIT.
        let mut clock = Clock::START;
        for _ in 0..3306 {
            clock = clock
                .advanced(Seconds(1_209_600))
                .expect("3306 full Ticks stay within LIMIT");
        }
        assert_eq!(clock.advanced(Seconds(1_209_600)), None);
        assert_eq!(clock.advanced(Seconds(1_062_401)), None);
        assert_eq!(
            clock.advanced(Seconds(1_062_400)),
            Some(Clock(Clock::LIMIT))
        );
    }

    #[test]
    fn message_ids_print_with_prefix() {
        assert_eq!(MessageId::FIRST.next().to_string(), "m-2");
    }
}
