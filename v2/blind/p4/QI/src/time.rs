//! The virtual clock and durations. Both count whole seconds; nothing reads the wall clock.

use serde::Serialize;
use std::ops::Add;

/// The clock stops here. The spec leaves behavior past this value unspecified, so `Tick`
/// refuses to go beyond it instead of guessing.
pub const MAX_CLOCK: u64 = 4_000_000_000;

/// A point on the virtual clock, in seconds since the process started.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Instant(u64);

/// A length of time in whole seconds. Every duration the protocol accepts fits in u32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Seconds(u32);

impl Instant {
    pub const START: Instant = Instant(0);

    /// Moves the clock forward by `by`, or returns None if that passes `MAX_CLOCK`.
    pub fn advanced_by(self, by: Seconds) -> Option<Instant> {
        let next = self.0.checked_add(u64::from(by.0))?;
        (next <= MAX_CLOCK).then_some(Instant(next))
    }
}

impl Add<Seconds> for Instant {
    type Output = Instant;

    /// The result is compared with other instants, never shown to a client past `MAX_CLOCK`
    /// plus one duration, so this cannot overflow: the clock is bounded and durations are u32.
    fn add(self, rhs: Seconds) -> Instant {
        Instant(self.0.checked_add(u64::from(rhs.0)).expect(
            "clock is at most MAX_CLOCK and a duration at most u32::MAX, so the sum fits in u64",
        ))
    }
}

impl Seconds {
    pub const fn new(seconds: u32) -> Seconds {
        Seconds(seconds)
    }
}
