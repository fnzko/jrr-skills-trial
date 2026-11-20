//! Virtual time: only `Tick` moves the clock (SPEC section 3).

use std::ops::{Add, AddAssign};

/// A span of virtual seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u64);

impl Seconds {
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    /// The span as a number, for results.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A value of the virtual clock, in seconds since the process started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

impl Instant {
    pub const START: Self = Self(0);

    /// The clock value as a number, for results.
    pub fn get(self) -> u64 {
        self.0
    }

    /// The time from `earlier` to `self`, which must not be later than `self`.
    pub fn since(self, earlier: Self) -> Seconds {
        Seconds(self.0 - earlier.0)
    }
}

impl Add<Seconds> for Instant {
    type Output = Self;

    fn add(self, span: Seconds) -> Self {
        Self(self.0 + span.0)
    }
}

impl AddAssign<Seconds> for Instant {
    fn add_assign(&mut self, span: Seconds) {
        self.0 += span.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_move_by_seconds() {
        let mut now = Instant::START;
        now += Seconds::new(5);
        assert_eq!(now + Seconds::new(2), Instant(7));
        assert_eq!((now + Seconds::new(2)).since(now), Seconds::new(2));
    }
}
