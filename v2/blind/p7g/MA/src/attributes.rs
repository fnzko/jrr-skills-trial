//! Queue attributes as given in a request, and their relation to `Settings`.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{as_bool, as_integer, as_seconds, as_str};
use crate::queue::{DeadLetter, QueueType, Settings};
use crate::types::Seconds;

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
const DEFAULT_DELAY: Seconds = Seconds::new(0);
const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

/// The attributes present in a request; each is already range-checked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_dedup: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeSet {
    /// Checks every attribute on its own: type, range, and known name.
    pub fn parse(members: &Map<String, Value>) -> Result<Self> {
        let mut set = Self::default();
        for (name, value) in members.iter().filter(|(_, v)| !v.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    set.visibility_timeout = Some(as_seconds(value, name, 0..=43_200)?)
                }
                "DelaySeconds" => set.delay = Some(as_seconds(value, name, 0..=900)?),
                "RetentionSeconds" => {
                    set.retention = Some(as_seconds(value, name, 60..=1_209_600)?)
                }
                "Fifo" => set.fifo = Some(as_bool(value, name)?),
                "ContentBasedDeduplication" => {
                    set.content_based_dedup = Some(as_bool(value, name)?)
                }
                "DeadLetterQueue" => set.dead_letter_queue = Some(as_str(value, name)?.to_owned()),
                "MaxReceiveCount" => {
                    let n = as_integer(value, name, 1..=1000)?;
                    set.max_receive_count = u32::try_from(n).ok();
                }
                _ => {
                    return Err(Error::new(
                        ErrorCode::InvalidParameterValue,
                        format!("unknown attribute {name:?}"),
                    ));
                }
            }
        }
        Ok(set)
    }

    /// The dead-letter configuration, which is both members or neither.
    pub fn dead_letter_pair(&self) -> Result<Option<(&str, u32)>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// True when, after defaults, every attribute equals the queue's value.
    pub fn describes(&self, settings: &Settings) -> bool {
        let wanted_type = self.queue_type();
        let wanted_dead_letter = match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max)) => Some((queue.as_str(), max)),
            _ => None,
        };
        let current_dead_letter = settings
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        self.visibility_timeout
            .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            == settings.visibility_timeout
            && self.delay.unwrap_or(DEFAULT_DELAY) == settings.delay
            && self.retention.unwrap_or(DEFAULT_RETENTION) == settings.retention
            && wanted_type == settings.queue_type
            && wanted_dead_letter == current_dead_letter
    }

    fn queue_type(&self) -> QueueType {
        if self.fifo.unwrap_or(false) {
            QueueType::Fifo {
                content_based_dedup: self.content_based_dedup.unwrap_or(false),
            }
        } else {
            QueueType::Standard
        }
    }

    /// Settings of a new queue: the attributes present, defaults elsewhere.
    pub fn new_settings(&self, dead_letter: Option<DeadLetter>) -> Settings {
        Settings {
            visibility_timeout: self
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: self.delay.unwrap_or(DEFAULT_DELAY),
            retention: self.retention.unwrap_or(DEFAULT_RETENTION),
            queue_type: self.queue_type(),
            dead_letter,
        }
    }

    /// `current` with the attributes present replaced. The queue type is
    /// never changed; `dead_letter` replaces the configuration if given.
    pub fn apply_to(&self, current: &Settings, dead_letter: Option<DeadLetter>) -> Settings {
        let queue_type = match (current.queue_type, self.content_based_dedup) {
            (QueueType::Fifo { .. }, Some(content_based_dedup)) => QueueType::Fifo {
                content_based_dedup,
            },
            (unchanged, _) => unchanged,
        };
        Settings {
            visibility_timeout: self
                .visibility_timeout
                .unwrap_or(current.visibility_timeout),
            delay: self.delay.unwrap_or(current.delay),
            retention: self.retention.unwrap_or(current.retention),
            queue_type,
            dead_letter: dead_letter.or_else(|| current.dead_letter.clone()),
        }
    }
}
