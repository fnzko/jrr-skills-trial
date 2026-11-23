//! Virtual time. Only the `Tick` command moves the clock.

use std::ops::Add;

use serde::Serialize;

/// A length of time in whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Seconds(pub u32);

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const START: Self = Self(0);
}

impl Add<Seconds> for Timestamp {
    type Output = Timestamp;

    /// Saturates instead of wrapping; behavior past the documented clock range is unspecified.
    fn add(self, rhs: Seconds) -> Timestamp {
        Timestamp(self.0.saturating_add(u64::from(rhs.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_seconds_moves_the_timestamp() {
        assert_eq!(Timestamp::START + Seconds(30) + Seconds(12), Timestamp(42));
    }

    #[test]
    fn adding_seconds_saturates_at_the_end_of_time() {
        assert_eq!(Timestamp(u64::MAX) + Seconds(1), Timestamp(u64::MAX));
    }
}
