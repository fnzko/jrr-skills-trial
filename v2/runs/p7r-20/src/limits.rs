//! Numeric limits from the specification, each stated once.

use std::ops::RangeInclusive;

/// Seconds a delivered message stays in flight (SPEC section 4).
pub const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
/// Seconds a message stays delayed after a send.
pub const DELAY_SECONDS: RangeInclusive<u64> = 0..=900;
/// Seconds a message is kept after it was sent.
pub const RETENTION_SECONDS: RangeInclusive<u64> = 60..=1_209_600;
/// Deliveries after which a message is dead-lettered.
pub const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
/// Seconds one `Tick` may advance the clock.
pub const TICK_SECONDS: RangeInclusive<u64> = 0..=1_209_600;
/// Messages one `ReceiveMessage` may deliver.
pub const RECEIVE_BATCH: RangeInclusive<u64> = 1..=10;
