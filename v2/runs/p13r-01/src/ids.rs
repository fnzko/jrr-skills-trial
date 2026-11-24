//! Identifiers that are numbers inside the service and text on the wire.

use std::fmt;

/// Defines a counter-backed identifier. `take` hands out the current value
/// and advances, so a value is never issued twice.
macro_rules! counter_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const FIRST: Self = Self(1);

            pub fn take(&mut self) -> Self {
                let issued = *self;
                self.0 += 1;
                issued
            }
        }
    };
}

counter_id! {
    /// The `MessageId` of a message, written `m-<n>`.
    MessageId
}

counter_id! {
    /// A receipt handle, written `r-<n>`.
    ReceiptHandle
}

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
    /// Parses `r-<n>` exactly as it was issued: no sign, no leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

/// The position of a message in its queue's arrival order. It only grows, so
/// it stays valid as a key while other messages come and go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Arrival(u64);

impl Arrival {
    pub fn take(&mut self) -> Self {
        let issued = *self;
        self.0 += 1;
        issued
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_the_issued_form() {
        let mut next = ReceiptHandle::FIRST;
        let first = next.take();
        let second = next.take();
        assert_eq!(ReceiptHandle::parse("r-1"), Some(first));
        assert_eq!(ReceiptHandle::parse("r-2"), Some(second));
        for bad in [
            "",
            "r-",
            "r-0",
            "r-01",
            "r-+1",
            "r--1",
            "m-1",
            "r-1 ",
            "r-99999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ids_display_with_their_prefix() {
        assert_eq!(MessageId::FIRST.to_string(), "m-1");
        assert_eq!(ReceiptHandle::FIRST.to_string(), "r-1");
    }
}
