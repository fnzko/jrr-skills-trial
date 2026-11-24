//! Virtual time: instants on the broker's clock and spans between them.

use std::ops::{Add, AddAssign, Sub};

use serde_json::Value;

/// A span of virtual seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u64);

impl Seconds {
    pub const ZERO: Self = Self(0);

    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }
}

/// An instant on the virtual clock, in seconds since the broker started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const START: Self = Self(0);
}

impl Add<Seconds> for Timestamp {
    type Output = Self;

    fn add(self, span: Seconds) -> Self {
        Self(self.0 + span.0)
    }
}

impl AddAssign<Seconds> for Timestamp {
    fn add_assign(&mut self, span: Seconds) {
        *self = *self + span;
    }
}

/// The span from `earlier` to `self`; the clock never runs backwards, so
/// callers subtract an instant that has already passed.
impl Sub for Timestamp {
    type Output = Seconds;

    fn sub(self, earlier: Self) -> Seconds {
        Seconds(self.0 - earlier.0)
    }
}

impl From<Seconds> for Value {
    fn from(span: Seconds) -> Self {
        span.0.into()
    }
}

impl From<Timestamp> for Value {
    fn from(instant: Timestamp) -> Self {
        instant.0.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_and_spans_combine() {
        let mut now = Timestamp::START;
        now += Seconds::new(40);
        assert_eq!(now + Seconds::new(2) - now, Seconds::new(2));
        assert_eq!(Value::from(now), Value::from(40u64));
    }
}
