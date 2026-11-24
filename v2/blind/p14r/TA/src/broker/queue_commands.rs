//! Queue management commands (SPEC section 5).

use serde_json::json;

use super::{Broker, Members, members};
use crate::attributes::{AttributeUpdate, Attributes, DeadLetter};
use crate::error::{Error, ErrorCode, Result};
use crate::naming::QueueName;
use crate::queue::Queue;
use crate::request::Params;

const MAX_QUEUES: usize = 1000;

/// Reads the `QueueName` every queue command takes.
fn queue_name<'a>(params: Params<'a>) -> Result<&'a str> {
    params.require(&["QueueName"])?;
    params.required_string("QueueName")
}

impl Broker {
    /// The dead-letter queue of the queue `owner` must exist, be another queue
    /// and be of the same type (`fifo`) as `owner`.
    fn check_dead_letter_target(
        &self,
        dead_letter: &DeadLetter,
        owner: &str,
        fifo: bool,
    ) -> Result<()> {
        let target = self
            .queues
            .get(dead_letter.queue.as_str())
            .ok_or_else(|| Error::queue_does_not_exist(&dead_letter.queue))?;
        if dead_letter.queue == owner || target.attributes.fifo != fifo {
            return Err(Error::invalid_parameter(
                "the dead-letter queue must be another queue of the same type",
            ));
        }
        Ok(())
    }

    pub(super) fn create_queue(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        let Some(queue_name) = QueueName::parse(name) else {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("`{name}` is not a valid queue name"),
            ));
        };
        let update = AttributeUpdate::parse(params.object("Attributes")?)?;

        let fifo = update.fifo.unwrap_or(false);
        if queue_name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the `.fifo` suffix and the `Fifo` attribute must agree",
            ));
        }
        update.check_content_based_deduplication(fifo)?;
        let dead_letter = update.dead_letter()?;

        let mut attributes = Attributes::default();
        attributes.apply(update, dead_letter);

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
            self.check_dead_letter_target(dead_letter, name, fifo)?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(queue_name, Queue::new(attributes));
        Ok(Members::new())
    }

    pub(super) fn set_queue_attributes(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.required_string("QueueName")?;
        let update = AttributeUpdate::parse(params.object("Attributes")?)?;
        let fifo = self.queue_mut(name)?.attributes.fifo;

        if update.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue cannot change"));
        }
        update.check_content_based_deduplication(fifo)?;
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            self.check_dead_letter_target(dead_letter, name, fifo)?;
        }

        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.attributes.apply(update, dead_letter);
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
        let names: Vec<&str> = self
            .queues
            .keys()
            .map(QueueName::as_str)
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
