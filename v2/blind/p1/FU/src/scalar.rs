//! Numeric building blocks: range-checked integers, the virtual clock and id counters.

use serde::Serialize;

/// An integer known to lie in `MIN..=MAX`; the only ways to build one check the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Bounded<const MIN: u32, const MAX: u32>(u32);

impl<const MIN: u32, const MAX: u32> Bounded<MIN, MAX> {
    /// A compile-time checked constant, for defaults taken from the specification.
    pub const fn literal<const V: u32>() -> Self {
        const { assert!(MIN <= V && V <= MAX) };
        Self(V)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl<const MIN: u32, const MAX: u32> TryFrom<i64> for Bounded<MIN, MAX> {
    type Error = OutOfRange;

    fn try_from(value: i64) -> Result<Self, OutOfRange> {
        u32::try_from(value)
            .ok()
            .filter(|v| (MIN..=MAX).contains(v))
            .map(Self)
            .ok_or(OutOfRange)
    }
}

/// The value is outside the range of the target type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

pub type VisibilityTimeout = Bounded<0, 43_200>;
pub type DelaySeconds = Bounded<0, 900>;
pub type RetentionSeconds = Bounded<60, 1_209_600>;
pub type MaxReceiveCount = Bounded<1, 1000>;
pub type MaxMessages = Bounded<1, 10>;
pub type TickSeconds = Bounded<0, 1_209_600>;

/// A value of the virtual clock, in seconds since it started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Saturates: behavior past 4000000000 is unspecified, but must not wrap.
    pub fn plus(self, seconds: u32) -> Self {
        Self(self.0.saturating_add(u64::from(seconds)))
    }
}

/// Hands out 1, 2, 3, ... and remembers how many it issued.
#[derive(Debug, Default)]
pub struct Counter {
    issued: u64,
}

impl Counter {
    pub fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    pub fn issued(&self) -> u64 {
        self.issued
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Small = Bounded<2, 5>;

    #[test]
    fn bounded_accepts_exactly_its_inclusive_range() {
        let cases = [
            (1, None),
            (2, Some(2)),
            (5, Some(5)),
            (6, None),
            (-1, None),
            (i64::MAX, None),
        ];
        for (input, expected) in cases {
            let actual = Small::try_from(input).ok().map(Small::get);
            assert_eq!(actual, expected, "input {input}");
        }
    }

    #[test]
    fn timestamp_plus_saturates() {
        let near_end = Timestamp(u64::MAX - 1);
        assert_eq!(near_end.plus(10), Timestamp(u64::MAX));
    }

    #[test]
    fn counter_starts_at_one() {
        let mut counter = Counter::default();
        assert_eq!((counter.issue(), counter.issue()), (1, 2));
        assert_eq!(counter.issued(), 2);
    }
}
