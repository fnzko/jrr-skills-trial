//! Small value types shared by the whole crate.

use std::borrow::Borrow;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, NameProblem, Problem};

const FIFO_SUFFIX: &str = ".fifo";
const QUEUE_NAME_MAX: usize = 80;
const BODY_MAX: usize = 1024;
const TOKEN_MAX: usize = 128;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The name of an existing or about-to-be-created queue. Names of queues that do not exist are
/// plain `&str` because lookups accept any string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let stem = raw.strip_suffix(FIFO_SUFFIX).unwrap_or(raw);
        let max = if stem.len() == raw.len() {
            QUEUE_NAME_MAX
        } else {
            QUEUE_NAME_MAX - FIFO_SUFFIX.len()
        };
        if stem.is_empty() || stem.len() > max || !stem.chars().all(is_name_char) {
            return Err(Error::QueueNameInvalid {
                name: raw.to_owned(),
                problem: NameProblem::Syntax,
            });
        }
        Ok(Self(raw.to_owned()))
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

/// A message body of 1 to 1024 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body(String);

impl Body {
    pub fn parse(parameter: &str, raw: &str) -> Result<Self, Error> {
        if raw.is_empty() || raw.len() > BODY_MAX {
            return Err(Error::invalid_value(
                parameter,
                Problem::Length {
                    min: 1,
                    max: BODY_MAX,
                },
            ));
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn parse_token(parameter: &str, raw: &str) -> Result<String, Error> {
    if raw.is_empty() || raw.len() > TOKEN_MAX {
        return Err(Error::invalid_value(
            parameter,
            Problem::Length {
                min: 1,
                max: TOKEN_MAX,
            },
        ));
    }
    if !raw.chars().all(is_name_char) {
        return Err(Error::invalid_value(parameter, Problem::Characters));
    }
    Ok(raw.to_owned())
}

/// The message group of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(parameter: &str, raw: &str) -> Result<Self, Error> {
        parse_token(parameter, raw).map(Self)
    }
}

/// An explicit deduplication id of a FIFO send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(parameter: &str, raw: &str) -> Result<Self, Error> {
        parse_token(parameter, raw).map(Self)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

macro_rules! counter_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
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

counter_id!(
    /// `m-<n>`, unique over the process.
    MessageId, "m"
);
counter_id!(
    /// `r-<n>`, one per delivery.
    ReceiptHandle, "r"
);

impl ReceiptHandle {
    /// Accepts exactly the strings `r-<n>` that `Ids` has issued, in their canonical spelling.
    pub fn parse(raw: &str, issued: u64) -> Option<Self> {
        let digits = raw.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (1..=issued).contains(&n).then_some(Self(n))
    }
}

/// Source of the process-wide message ids and receipt handles.
#[derive(Debug, Default)]
pub struct Ids {
    messages: u64,
    handles: u64,
}

impl Ids {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_handle(&mut self) -> ReceiptHandle {
        self.handles += 1;
        ReceiptHandle(self.handles)
    }

    pub fn issued_handles(&self) -> u64 {
        self.handles
    }
}

/// Position of a message in a FIFO queue, counting every message ever added to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: Self = Self(1);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// A length of virtual time in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Seconds(u64);

impl Seconds {
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }
}

/// A point of virtual time, in seconds since the process started.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Time(u64);

impl Time {
    /// Saturates: behavior past 4000000000 is unspecified, but it must not wrap.
    pub fn after(self, span: Seconds) -> Self {
        Self(self.0.saturating_add(span.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_follow_the_syntax() {
        let long = "a".repeat(80);
        let longest_fifo = format!("{}.fifo", "a".repeat(75));
        let accepted = ["a", "A_z-9", long.as_str(), "q.fifo", longest_fifo.as_str()];
        for name in accepted {
            assert_eq!(QueueName::parse(name).unwrap().as_str(), name, "{name}");
        }
        let too_long = "a".repeat(81);
        let fifo_stem_too_long = format!("{}.fifo", "a".repeat(76));
        let rejected = [
            "",
            ".fifo",
            "a b",
            "a.b",
            "a.fifo.fifo",
            "é",
            too_long.as_str(),
            fifo_stem_too_long.as_str(),
        ];
        for name in rejected {
            assert_eq!(
                QueueName::parse(name).unwrap_err().code(),
                crate::error::Code::QueueNameInvalid,
                "{name:?}"
            );
        }
    }

    #[test]
    fn receipt_handles_must_be_canonical_and_issued() {
        let mut ids = Ids::default();
        let first = ids.next_handle();
        let second = ids.next_handle();
        assert_eq!(ReceiptHandle::parse("r-1", 2), Some(first));
        assert_eq!(ReceiptHandle::parse("r-2", 2), Some(second));
        for raw in [
            "r-3",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            "r-+1",
            "r-",
            "1",
            "r--1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(raw, 2), None, "{raw:?}");
        }
    }

    #[test]
    fn tokens_are_limited_in_length_and_characters() {
        let longest = "g".repeat(128);
        assert_eq!(
            GroupId::parse("GroupId", &longest).unwrap(),
            GroupId(longest)
        );
        for raw in [
            "".to_owned(),
            "g".repeat(129),
            "a b".to_owned(),
            "é".to_owned(),
        ] {
            assert_eq!(
                GroupId::parse("GroupId", &raw).unwrap_err().code(),
                crate::error::Code::InvalidParameterValue
            );
        }
    }

    #[test]
    fn bodies_are_limited_in_bytes() {
        let largest = "é".repeat(512);
        assert_eq!(Body::parse("Body", &largest).unwrap(), Body(largest));
        for raw in ["é".repeat(513), String::new()] {
            let Err(Error::InvalidParameterValue { problem, .. }) = Body::parse("Body", &raw)
            else {
                panic!("expected a length error for {} bytes", raw.len());
            };
            assert_eq!(problem, Problem::Length { min: 1, max: 1024 });
        }
    }
}
