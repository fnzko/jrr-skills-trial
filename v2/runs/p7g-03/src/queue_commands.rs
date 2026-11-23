//! Queue commands (SPEC section 5): everything except `Tick`.

use serde_json::{Map, Value, json};

use crate::attributes::{AttributeRequest, has_fifo_suffix, is_valid_queue_name};
use crate::error::{Code, Error, Result};
use crate::params::Params;
use crate::queue::Queue;
use crate::service::{Service, no_such_queue, object};

const MAX_QUEUES: usize = 1000;

/// Reads the optional `Attributes` object (all of it validated).
fn attribute_request(params: &Params) -> Result<Option<AttributeRequest>> {
    let Some(value) = params.get("Attributes") else {
        return Ok(None);
    };
    let members = value
        .as_object()
        .ok_or_else(|| Error::invalid_value("Attributes must be an object"))?;
    AttributeRequest::parse(members).map(Some)
}

impl Service {
    pub(crate) fn create_queue(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        // Name first: its syntax error outranks every other value error.
        let name = params.required_string("QueueName")?;
        if !is_valid_queue_name(name) {
            return Err(Error::new(
                Code::QueueNameInvalid,
                format!("invalid queue name {name:?}"),
            ));
        }
        let request = attribute_request(params)?.unwrap_or_default();

        let fifo = request.fifo.unwrap_or(false);
        if has_fifo_suffix(name) != fifo {
            return Err(Error::new(
                Code::QueueNameInvalid,
                "the .fifo suffix must be used exactly by FIFO queues",
            ));
        }
        if request.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        if request.has_partial_dead_letter() {
            return Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            ));
        }
        let attributes = request.with_defaults();
        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == attributes {
                Ok(Map::new())
            } else {
                Err(Error::new(
                    Code::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| no_such_queue(&dead_letter.queue))?;
            if target.attributes.fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(Code::LimitExceeded, "too many queues"));
        }
        let uid = self.next_queue_uid();
        self.queues
            .insert(name.to_owned(), Queue::new(uid, attributes));
        Ok(Map::new())
    }

    pub(crate) fn set_queue_attributes(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        params.require("Attributes")?;
        let name = params.required_string("QueueName")?;
        let request = attribute_request(params)?.unwrap_or_default();
        let queue = self.queue_mut(name)?;

        if request.fifo.is_some() {
            return Err(Error::new(
                Code::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if request.content_based_deduplication == Some(true) && !queue.attributes.fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        if request.has_partial_dead_letter() {
            return Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            ));
        }
        let fifo = queue.attributes.fifo;
        if let Some(target_name) = &request.dead_letter_queue {
            let target = self
                .queues
                .get(target_name)
                .ok_or_else(|| no_such_queue(target_name))?;
            if target_name == name || target.attributes.fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }
        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.attributes.apply(&request);
        // A lowered retention takes effect at once.
        queue.expire(now);
        Ok(Map::new())
    }

    pub(crate) fn get_queue_attributes(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        let name = params.required_string("QueueName")?;
        let now = self.now;
        let queue = self.queue_mut(name)?;
        let (visible, in_flight, delayed) = queue.counts(now);
        Ok(object([
            ("Attributes", queue.attributes.to_json()),
            (
                "Counts",
                json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed}),
            ),
        ]))
    }

    pub(crate) fn list_queues(&mut self, params: &Params) -> Result<Map<String, Value>> {
        let prefix = params.string("Prefix")?.unwrap_or("");
        // `str` ordering is byte-wise, as the spec asks.
        let mut names: Vec<&str> = self
            .queues
            .keys()
            .map(String::as_str)
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort_unstable();
        Ok(object([("QueueNames", json!(names))]))
    }

    pub(crate) fn delete_queue(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        let name = params.required_string("QueueName")?;
        self.queue_mut(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Map::new())
    }

    pub(crate) fn purge_queue(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        let name = params.required_string("QueueName")?;
        self.queue_mut(name)?.clear();
        Ok(Map::new())
    }
}
