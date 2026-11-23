//! Virtual time: whole seconds, advanced only by `Tick`.

/// A point on the virtual clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

/// A span of virtual time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seconds(u32);

impl Instant {
    pub const START: Instant = Instant(0);

    pub fn after(self, span: Seconds) -> Instant {
        Instant(self.0.saturating_add(u64::from(span.0)))
    }

    /// Whole seconds from `earlier` to `self`; zero if `earlier` is later.
    pub fn since(self, earlier: Instant) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    pub fn as_secs(self) -> u64 {
        self.0
    }
}

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub const fn new(secs: u32) -> Seconds {
        Seconds(secs)
    }

    pub fn as_secs(self) -> u32 {
        self.0
    }
}
