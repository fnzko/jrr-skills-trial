//! Commands that create, configure, list, empty and delete queues.

use serde_json::{Value, json};

use crate::broker::{Broker, Fields, fields};
use crate::error::{Error, ErrorCode, Result};
use crate::params::{Params, invalid};
use crate::queue::{
    AttributePatch, Queue, Settings, has_fifo_suffix, is_valid_queue_name, queue_does_not_exist,
};

const MAX_QUEUES: usize = 1000;

impl Broker {
    pub(crate) fn create_queue(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName"])?;
        // The name is checked before any other parameter, with its own error code.
        let name = params
            .string("QueueName")?
            .expect("required parameter is present");
        if !is_valid_queue_name(name) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ));
        }
        let patch = match params.object("Attributes")? {
            Some(attributes) => AttributePatch::parse(attributes)?,
            None => AttributePatch::default(),
        };
        let fifo = patch.fifo.unwrap_or(false);

        if has_fifo_suffix(name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid(
                "ContentBasedDeduplication requires a FIFO queue".into(),
            ));
        }
        let dead_letter = patch.dead_letter()?;
        let settings = patch.apply(Settings::default(), dead_letter.clone());

        if let Some(existing) = self.queues.get(name) {
            return if existing.settings == settings {
                Ok(Fields::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| queue_does_not_exist(&dead_letter.queue))?;
            if target.settings.fifo != fifo {
                return Err(invalid(
                    "the dead-letter queue must be of the same type".into(),
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        let id = self.next_queue_id();
        self.queues
            .insert(name.to_owned(), Queue::new(id, settings));
        Ok(Fields::new())
    }

    pub(crate) fn set_queue_attributes(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.string("QueueName")?.expect("required");
        let attributes = params.object("Attributes")?.expect("required");
        let patch = AttributePatch::parse(attributes)?;

        let now = self.now;
        let queue = self.queue_mut(name)?;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !queue.settings.fifo {
            return Err(invalid(
                "ContentBasedDeduplication requires a FIFO queue".into(),
            ));
        }
        let dead_letter = patch.dead_letter()?;
        let fifo = queue.settings.fifo;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| queue_does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.settings.fifo != fifo {
                return Err(invalid(
                    "the dead-letter queue must be another queue of the same type".into(),
                ));
            }
        }

        let queue = self.queue_mut(name)?;
        // Messages the old retention period already removed must not come back.
        queue.expire(now);
        queue.settings = patch.apply(queue.settings.clone(), dead_letter);
        queue.expire(now);
        Ok(Fields::new())
    }

    pub(crate) fn get_queue_attributes(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let now = self.now;
        let queue = self.queue_mut(name)?;
        let settings = &queue.settings;
        let mut attributes = json!({
            "VisibilityTimeout": settings.visibility_timeout,
            "DelaySeconds": settings.delay_seconds,
            "RetentionSeconds": settings.retention_seconds,
            "Fifo": settings.fifo,
            "ContentBasedDeduplication": settings.content_based_deduplication,
        });
        if let Some(dead_letter) = &settings.dead_letter {
            attributes["DeadLetterQueue"] = json!(dead_letter.queue);
            attributes["MaxReceiveCount"] = json!(dead_letter.max_receive_count);
        }
        let counts = queue.counts(now);
        Ok(fields([
            ("Attributes", attributes),
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

    pub(crate) fn list_queues(&mut self, params: &Params) -> Result<Fields> {
        let prefix = params.string("Prefix")?.unwrap_or_default();
        // The map is ordered by name, and `String` orders by byte value.
        let names: Vec<Value> = self
            .queues
            .range(prefix.to_owned()..)
            .take_while(|(name, _)| name.starts_with(prefix))
            .map(|(name, _)| json!(name))
            .collect();
        Ok(fields([("QueueNames", Value::Array(names))]))
    }

    pub(crate) fn delete_queue(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        self.queue_mut(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .settings
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.settings.dead_letter = None;
            }
        }
        Ok(Fields::new())
    }

    pub(crate) fn purge_queue(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        self.queue_mut(name)?.purge();
        Ok(Fields::new())
    }
}
