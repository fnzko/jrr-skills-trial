//! Value ranges that the specification fixes for parameters and attributes.
//! Each is used by more than one command, so it is named once here.

use std::ops::RangeInclusive;

pub(crate) const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
pub(crate) const DELAY_SECONDS: RangeInclusive<u64> = 0..=900;
pub(crate) const RETENTION_SECONDS: RangeInclusive<u64> = 60..=1_209_600;
pub(crate) const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
pub(crate) const TICK_SECONDS: RangeInclusive<u64> = 0..=1_209_600;
