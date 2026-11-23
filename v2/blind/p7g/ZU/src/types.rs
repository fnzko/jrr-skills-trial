//! Small types that carry a validity rule or identify one kind of thing.

use std::borrow::Borrow;
use std::ops::RangeInclusive;

use serde::{Serialize, Serializer};

/// A value of the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

impl Time {
    /// The clock is only specified up to 4_000_000_000, so saturating is never observable.
    pub fn after(self, span: impl Into<u64>) -> Self {
        Self(self.0.saturating_add(span.into()))
    }
}

/// An integer parameter that is only valid inside an inclusive range.
pub trait Bounded: Sized {
    const RANGE: RangeInclusive<u32>;

    fn new(value: u32) -> Option<Self>;
}

macro_rules! bounded {
    ($(#[$meta:meta])* $name:ident, $range:expr $(, default = $default:expr)?) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(u32);

        impl Bounded for $name {
            const RANGE: RangeInclusive<u32> = $range;

            fn new(value: u32) -> Option<Self> {
                Self::RANGE.contains(&value).then_some(Self(value))
            }
        }

        impl $name {
            pub fn get(self) -> u32 {
                self.0
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> u64 {
                u64::from(value.0)
            }
        }

        $(impl Default for $name {
            fn default() -> Self {
                Self($default)
            }
        })?
    };
}

bounded!(
    /// Seconds one `Tick` advances the clock by.
    TickSeconds,
    0..=1_209_600
);

bounded!(VisibilityTimeout, 0..=43_200, default = 30);
bounded!(
    /// Seconds a message stays invisible after it is sent.
    Delay,
    0..=900,
    default = 0
);
bounded!(Retention, 60..=1_209_600, default = 345_600);
bounded!(MaxReceiveCount, 1..=1000);
bounded!(MaxMessages, 1..=10, default = 1);

macro_rules! counter {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const FIRST: Self = Self(1);

            /// A process hands out far fewer than `u64::MAX` identifiers, so this cannot overflow.
            pub fn next(self) -> Self {
                Self(self.0 + 1)
            }
        }
    };
}

counter!(
    /// Counts the messages the process accepted; written `m-<n>` on the wire.
    MessageId
);
counter!(
    /// Counts the deliveries the process made; written `r-<n>` on the wire.
    ReceiptHandle
);
counter!(
    /// Counts the messages added to one FIFO queue.
    SequenceNumber
);

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("m-{}", self.0))
    }
}

impl ReceiptHandle {
    /// Only the exact text this process writes is a handle: no sign, no leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("r-{}", self.0))
    }
}

impl Serialize for SequenceNumber {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

/// A message body: 1 to 1024 bytes of UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Body(String);

impl Body {
    pub fn new(text: String) -> Option<Self> {
        (1..=1024).contains(&text.len()).then_some(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_token(text: &str) -> bool {
    (1..=128).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

macro_rules! token {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 1 to 128 characters from `A-Z a-z 0-9 _ -`.
            pub fn new(text: String) -> Option<Self> {
                is_token(&text).then_some(Self(text))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

token!(GroupId);
token!(DeduplicationId);

const FIFO_SUFFIX: &str = ".fifo";

/// A syntactically valid queue name. A name ending in `.fifo` is the name of a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct QueueName(String);

impl QueueName {
    pub fn new(name: String) -> Option<Self> {
        let (base, max_base_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(base) => (base, 75),
            None => (name.as_str(), 80),
        };
        let valid = (1..=max_base_len).contains(&base.len())
            && base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then_some(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

/// Lets the broker look queues up by any string without validating it first.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_accepts_only_its_range() {
        assert_eq!(TickSeconds::new(0), Some(TickSeconds(0)));
        assert_eq!(TickSeconds::new(1_209_600), Some(TickSeconds(1_209_600)));
        assert_eq!(TickSeconds::new(1_209_601), None);
    }

    #[test]
    fn queue_names_follow_the_naming_rules() {
        let long_base = "a".repeat(75);
        let accepted = [
            "jobs".to_owned(),
            "A-z_09".to_owned(),
            "a".repeat(80),
            format!("{long_base}.fifo"),
            "jobs.fifo".to_owned(),
        ];
        for name in accepted {
            assert_eq!(QueueName::new(name.clone()).map(|q| q.0), Some(name));
        }
        let rejected = [
            "".to_owned(),
            ".fifo".to_owned(),
            "a b".to_owned(),
            "a.b".to_owned(),
            "a.fifo.fifo".to_owned(),
            "jobs.FIFO".to_owned(),
            "caf\u{e9}".to_owned(),
            "a".repeat(81),
            format!("{long_base}a.fifo"),
        ];
        for name in rejected {
            assert_eq!(QueueName::new(name.clone()), None, "{name:?}");
        }
    }

    #[test]
    fn only_the_suffix_makes_a_name_fifo() {
        let fifo = QueueName::new("a.fifo".to_owned()).expect("valid name");
        let standard = QueueName::new("fifo".to_owned()).expect("valid name");
        assert_eq!((fifo.is_fifo(), standard.is_fifo()), (true, false));
    }

    #[test]
    fn body_and_token_limits_count_bytes() {
        assert_eq!(Body::new(String::new()), None);
        assert!(Body::new("a".repeat(1024)).is_some());
        assert_eq!(Body::new("a".repeat(1025)), None);
        assert!(Body::new("\u{e9}".repeat(512)).is_some());
        assert_eq!(Body::new("\u{e9}".repeat(513)), None);
        assert!(GroupId::new("a".repeat(128)).is_some());
        for bad in ["", "a b", "a.b", "\u{e9}"] {
            assert_eq!(GroupId::new(bad.to_owned()), None, "{bad:?}");
        }
        assert_eq!(DeduplicationId::new("a".repeat(129)), None);
    }

    #[test]
    fn identifiers_count_from_one() {
        assert_eq!(MessageId::FIRST.next(), MessageId(2));
        assert_eq!(
            serde_json::to_value(MessageId::FIRST.next()).expect("serializes"),
            serde_json::json!("m-2")
        );
        assert_eq!(
            serde_json::to_value(SequenceNumber::FIRST).expect("serializes"),
            serde_json::json!(1)
        );
    }

    #[test]
    fn receipt_handles_parse_only_their_canonical_text() {
        assert_eq!(ReceiptHandle::parse("r-1"), Some(ReceiptHandle::FIRST));
        assert_eq!(ReceiptHandle::parse("r-120").map(|h| h.0), Some(120));
        for bad in [
            "",
            "r-",
            "r-0",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r--1",
            "r-+1",
            "m-1",
            "1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn time_adds_a_span() {
        let later = Time::default().after(TickSeconds::new(7).expect("7 is in range"));
        assert_eq!(later, Time(7));
        assert_eq!(later.after(300_u64), Time(307));
    }
}
