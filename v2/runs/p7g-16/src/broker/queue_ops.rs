//! Queue commands (SPEC section 5).

use serde_json::{Map, Value, json};

use super::{Broker, MAX_QUEUES, Output, QUEUE_NAME, output};
use crate::attributes::{AttributeUpdate, DeadLetter, QueueConfig};
use crate::error::{Error, ErrorCode, Result};
use crate::params::{Param, Params, Rule};
use crate::queue::{Queue, State};

const CREATE_QUEUE: &[Param] = &[
    // Listed first: CreateQueue checks the name before any other parameter.
    Param::required("QueueName", Rule::NewQueueName),
    Param::optional("Attributes", Rule::Attributes),
];
const SET_QUEUE_ATTRIBUTES: &[Param] =
    &[QUEUE_NAME, Param::required("Attributes", Rule::Attributes)];
const QUEUE_ONLY: &[Param] = &[QUEUE_NAME];
const LIST_QUEUES: &[Param] = &[Param::optional("Prefix", Rule::Str)];

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl QueueConfig {
    /// Overwrites the attributes named in `update`; `dead_letter` replaces the
    /// dead-letter configuration when present.
    fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        if let Some(v) = update.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = update.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = update.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = update.fifo {
            self.fifo = v;
        }
        if let Some(v) = update.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

impl Broker {
    pub(super) fn create_queue(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(CREATE_QUEUE, object)?;
        let name = params.required_str("QueueName");
        let update = params.attributes().cloned().unwrap_or_default();
        let fifo = update.fifo.unwrap_or(false);

        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if update.wants_content_based_deduplication() && !fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = update.dead_letter()?;
        let mut config = QueueConfig::default();
        config.apply(&update, dead_letter.clone());

        if let Some(existing) = self.queues.get(name) {
            return if existing.config == config {
                Ok(Output::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &dead_letter {
            let target = self.queue(&dead_letter.queue)?;
            if target.config.fifo != fifo {
                return Err(invalid("the dead-letter queue must be of the same type"));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues"),
            ));
        }
        self.queues.insert(name.to_owned(), Queue::new(config));
        Ok(Output::new())
    }

    pub(super) fn set_queue_attributes(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(SET_QUEUE_ATTRIBUTES, object)?;
        let name = params.required_str("QueueName");
        let update = params.attributes().cloned().unwrap_or_default();
        let queue = self.queue(name)?;

        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.wants_content_based_deduplication() && !queue.config.fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self.queue(&dead_letter.queue)?;
            if dead_letter.queue == name || target.config.fifo != queue.config.fifo {
                return Err(invalid(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.config.apply(&update, dead_letter);
        if update.retention_seconds.is_some() {
            queue.expire(now);
        }
        Ok(Output::new())
    }

    pub(super) fn get_queue_attributes(&self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(QUEUE_ONLY, object)?;
        let queue = self.queue(params.required_str("QueueName"))?;
        let config = &queue.config;

        let mut attributes = json!({
            "VisibilityTimeout": config.visibility_timeout,
            "DelaySeconds": config.delay_seconds,
            "RetentionSeconds": config.retention_seconds,
            "Fifo": config.fifo,
            "ContentBasedDeduplication": config.content_based_deduplication,
        });
        if let Some(dead_letter) = &config.dead_letter {
            attributes["DeadLetterQueue"] = json!(dead_letter.queue);
            attributes["MaxReceiveCount"] = json!(dead_letter.max_receive_count);
        }

        let (mut visible, mut in_flight, mut delayed) = (0, 0, 0);
        for message in queue.messages.values() {
            match message.state(self.now) {
                State::Visible => visible += 1,
                State::InFlight => in_flight += 1,
                State::Delayed => delayed += 1,
            }
        }
        Ok(output(json!({
            "Attributes": attributes,
            "Counts": { "Visible": visible, "InFlight": in_flight, "Delayed": delayed },
        })))
    }

    pub(super) fn list_queues(&self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(LIST_QUEUES, object)?;
        let prefix = params.str("Prefix").unwrap_or("");
        // Names are ordered by byte value, so matches form one contiguous range.
        let names: Vec<&str> = self
            .queues
            .range(prefix.to_owned()..)
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .collect();
        Ok(output(json!({ "QueueNames": names })))
    }

    pub(super) fn delete_queue(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(QUEUE_ONLY, object)?;
        let name = params.required_str("QueueName");
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.config.dead_letter = None;
            }
        }
        Ok(Output::new())
    }

    pub(super) fn purge_queue(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(QUEUE_ONLY, object)?;
        let queue = self.queue_mut(params.required_str("QueueName"))?;
        queue.messages.clear();
        queue.handles.clear();
        Ok(Output::new())
    }
}
