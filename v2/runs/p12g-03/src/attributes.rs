use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::params::Param;
use crate::queue::DeadLetter;
use crate::types::Seconds;

/// The queue attributes of a request, each checked on its own.
#[derive(Debug, Default)]
pub struct Attributes<'a> {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<&'a str>,
    max_receive_count: Option<u32>,
}

impl<'a> Attributes<'a> {
    pub fn parse(object: &'a Map<String, Value>) -> Result<Self, Error> {
        let mut attributes = Self::default();
        for (name, value) in object.iter().filter(|(_, value)| !value.is_null()) {
            let param = Param::new(name, value);
            match name.as_str() {
                "VisibilityTimeout" => {
                    attributes.visibility_timeout = Some(param.seconds(0..=43_200)?)
                }
                "DelaySeconds" => attributes.delay = Some(param.seconds(0..=900)?),
                "RetentionSeconds" => attributes.retention = Some(param.seconds(60..=1_209_600)?),
                "Fifo" => attributes.fifo = Some(param.boolean()?),
                "ContentBasedDeduplication" => {
                    attributes.content_based_deduplication = Some(param.boolean()?);
                }
                "DeadLetterQueue" => attributes.dead_letter_queue = Some(param.string()?),
                "MaxReceiveCount" => attributes.max_receive_count = Some(param.bounded(1..=1000)?),
                _ => {
                    return Err(Error::new(
                        ErrorCode::InvalidParameterValue,
                        format!("{name:?} is not a queue attribute"),
                    ));
                }
            }
        }
        Ok(attributes)
    }

    pub fn dead_letter_queue(&self) -> Option<&'a str> {
        self.dead_letter_queue
    }

    /// The dead-letter configuration of the request: both attributes or none.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.to_owned(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}
