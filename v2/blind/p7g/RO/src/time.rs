//! Virtual time. Only `Tick` moves the clock.

use serde::Serialize;

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Time(u64);

/// A length of virtual time in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Seconds(pub u32);

impl Time {
    /// The clock value when the process starts.
    pub const START: Self = Self(0);

    /// The point `span` after `self`.
    #[must_use]
    pub fn plus(self, span: Seconds) -> Self {
        // Behavior past 4_000_000_000 is unspecified, so saturating is enough.
        Self(self.0.saturating_add(u64::from(span.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_adds_seconds() {
        assert_eq!(Time::START.plus(Seconds(30)), Time(30));
    }

    #[test]
    fn plus_saturates_instead_of_wrapping() {
        assert_eq!(Time(u64::MAX).plus(Seconds(1)), Time(u64::MAX));
    }
}
