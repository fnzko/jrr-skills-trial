//! Virtual time. Only `Tick` moves the clock, so there is no wall clock anywhere.

use serde::Serialize;

/// A length of virtual time in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u64);

impl Seconds {
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }
}

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Instant(u64);

impl Instant {
    pub const START: Instant = Instant(0);

    /// Behavior past 4000000000 is unspecified; saturating keeps it free of overflow.
    pub fn after(self, duration: Seconds) -> Instant {
        Instant(self.0.saturating_add(duration.0))
    }

    /// Time elapsed since `earlier`, zero if `earlier` is in the future.
    pub fn since(self, earlier: Instant) -> Seconds {
        Seconds(self.0.saturating_sub(earlier.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_adds_and_since_inverts() {
        let later = Instant::START.after(Seconds::new(7));
        assert_eq!(later, Instant(7));
        assert_eq!(later.since(Instant::START), Seconds::new(7));
    }

    #[test]
    fn since_a_future_instant_is_zero() {
        assert_eq!(Instant::START.since(Instant(5)), Seconds::new(0));
    }

    #[test]
    fn after_saturates_instead_of_overflowing() {
        let far = Instant(u64::MAX);
        assert_eq!(far.after(Seconds::new(1)), far);
    }
}
