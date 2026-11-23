//! Queue management commands (SPEC section 5).

use serde_json::json;

use super::{Broker, Members, does_not_exist, members};
use crate::attributes::{AttributeUpdate, Attributes};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::queue::Queue;
use crate::request::Params;

const MAX_QUEUES: usize = 1000;

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Reads the `QueueName` every queue command takes.
fn queue_name<'a>(params: Params<'a>) -> Result<&'a str> {
    params.require(&["QueueName"])?;
    params.required_string("QueueName")
}

impl Broker {
    pub(super) fn create_queue(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        if !naming::is_valid_queue_name(name) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("`{name}` is not a valid queue name"),
            ));
        }
        let update = AttributeUpdate::parse(params.object("Attributes")?)?;

        let fifo = update.fifo.unwrap_or(false);
        if naming::has_fifo_suffix(name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the `.fifo` suffix and the `Fifo` attribute must agree",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "`ContentBasedDeduplication` requires a FIFO queue",
            ));
        }
        update.dead_letter()?;

        let mut attributes = Attributes::default();
        attributes.apply(update);

        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == attributes {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue `{name}` exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.attributes.fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name.to_owned(), Queue::new(attributes));
        Ok(Members::new())
    }

    pub(super) fn set_queue_attributes(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.required_string("QueueName")?;
        let update = AttributeUpdate::parse(params.object("Attributes")?)?;
        let fifo = self.queue_mut(name)?.attributes.fifo;

        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "`ContentBasedDeduplication` requires a FIFO queue",
            ));
        }
        if let Some(dead_letter) = update.dead_letter()? {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.attributes.fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.attributes.apply(update);
        queue.expire(now);
        Ok(Members::new())
    }

    pub(super) fn get_queue_attributes(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        let now = self.now;
        let queue = self.queue_mut(name)?;
        let counts = queue.counts(now);
        Ok(members([
            ("Attributes", queue.attributes.to_json()),
            (
                "Counts",
                json!({
                    "Visible": counts.visible,
                    "InFlight": counts.in_flight,
                    "Delayed": counts.delayed,
                }),
            ),
        ]))
    }

    pub(super) fn list_queues(&mut self, params: Params<'_>) -> Result<Members> {
        let prefix = params.string("Prefix")?.unwrap_or("");
        let names: Vec<&String> = self
            .queues
            .keys()
            .filter(|name| name.starts_with(prefix))
            .collect();
        Ok(members([("QueueNames", json!(names))]))
    }

    pub(super) fn delete_queue(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        self.queue_mut(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dl| dl.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Members::new())
    }

    pub(super) fn purge_queue(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        self.queue_mut(name)?.purge();
        Ok(Members::new())
    }
}
