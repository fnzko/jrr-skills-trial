//! Counters handed out by the broker and by queues. Each kind has its own
//! type so that one cannot be passed where another is meant.

use std::fmt;

use serde_json::Value;

/// Defines a counter newtype that starts at 1 and is only ever advanced by
/// [`take`](MessageId::take).
macro_rules! counter {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const FIRST: Self = Self(1);

            /// Returns the current value and moves the counter to the next.
            pub fn take(&mut self) -> Self {
                let current = *self;
                self.0 += 1;
                current
            }
        }
    };
}

counter!(
    /// `MessageId`, rendered as `m-<n>`.
    MessageId
);
counter!(
    /// `ReceiptHandle`, rendered as `r-<n>`.
    ReceiptHandle
);
counter!(
    /// `SequenceNumber` of a message in a FIFO queue.
    SequenceNumber
);
counter!(
    /// Position of a message in the arrival order of its queue.
    ArrivalIndex
);

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

impl ReceiptHandle {
    /// Parses `r-<n>` exactly as it was rendered: no sign, no leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl From<SequenceNumber> for Value {
    fn from(sequence: SequenceNumber) -> Self {
        Value::from(sequence.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_round_trip_and_reject_other_spellings() {
        let mut counter = ReceiptHandle::FIRST;
        let first = counter.take();
        assert_eq!(first.to_string(), "r-1");
        assert_eq!(ReceiptHandle::parse("r-1"), Some(first));
        assert_eq!(ReceiptHandle::parse("r-2"), Some(counter));
        for bad in [
            "r-0",
            "r-01",
            "r-+1",
            "r--1",
            "r-",
            "m-1",
            "1",
            "r-1 ",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn take_returns_the_value_before_advancing() {
        let mut id = MessageId::FIRST;
        assert_eq!(id.take().to_string(), "m-1");
        assert_eq!(id.take().to_string(), "m-2");
    }
}
