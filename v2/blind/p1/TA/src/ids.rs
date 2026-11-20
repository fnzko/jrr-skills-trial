//! Process-wide identifiers: message ids and receipt handles.

use std::fmt;

use crate::error::{Error, ErrorCode};

/// `m-<n>`, numbered in the order messages are accepted over all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`, numbered in the order deliveries are made over all queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(u64);

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

const MAX_ID_CHARS: usize = 128;

/// The characters allowed in queue names, group ids and deduplication ids.
pub(crate) fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn validated_id(kind: &str, value: &str) -> Result<String, Error> {
    if value.is_empty() || value.len() > MAX_ID_CHARS || !value.bytes().all(is_name_char) {
        return Err(Error::invalid_value(format!(
            "{kind} must be 1 to {MAX_ID_CHARS} characters from A-Z a-z 0-9 _ -"
        )));
    }
    Ok(value.to_owned())
}

/// The ordering key of a FIFO queue's messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupId(String);

impl GroupId {
    pub fn parse(value: &str) -> Result<Self, Error> {
        validated_id("GroupId", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The key a FIFO send is deduplicated by, when the sender names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicationId(String);

impl DeduplicationId {
    pub fn parse(value: &str) -> Result<Self, Error> {
        validated_id("DeduplicationId", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Hands out the counters behind [`MessageId`] and [`ReceiptHandle`].
#[derive(Debug, Default)]
pub struct IdSource {
    messages: u64,
    handles: u64,
}

impl IdSource {
    pub fn next_message(&mut self) -> MessageId {
        self.messages += 1;
        MessageId(self.messages)
    }

    pub fn next_handle(&mut self) -> ReceiptHandle {
        self.handles += 1;
        ReceiptHandle(self.handles)
    }

    /// Reads `text` as a handle this source has issued. Only the exact
    /// canonical spelling counts, so `r-01`, `R-1` and `r-1 ` are rejected.
    pub fn parse_handle(&self, text: &str) -> Result<ReceiptHandle, Error> {
        text.strip_prefix("r-")
            .filter(|digits| {
                digits.bytes().all(|b| b.is_ascii_digit())
                    && !digits.is_empty()
                    && !digits.starts_with('0')
            })
            .and_then(|digits| digits.parse::<u64>().ok())
            .filter(|&n| n <= self.handles)
            .map(ReceiptHandle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_numbered_from_one_and_format_as_in_the_spec() {
        let mut ids = IdSource::default();
        assert_eq!(ids.next_message().to_string(), "m-1");
        assert_eq!(ids.next_message().to_string(), "m-2");
        assert_eq!(ids.next_handle().to_string(), "r-1");
    }

    #[test]
    fn only_canonical_issued_handles_parse() {
        let mut ids = IdSource::default();
        let issued = ids.next_handle();
        assert_eq!(ids.parse_handle("r-1"), Ok(issued));
        for text in [
            "r-2",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-+1",
            "r-",
            "1",
            "",
            "r-99999999999999999999999",
        ] {
            let err = ids.parse_handle(text).expect_err(text);
            assert_eq!(err.code(), ErrorCode::ReceiptHandleInvalid, "{text:?}");
        }
    }
}
