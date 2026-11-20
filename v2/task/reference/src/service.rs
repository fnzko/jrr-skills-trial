#[cfg(feature = "extension")]
mod batch;
#[cfg(feature = "extension")]
mod tags;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::error::{Code, Failure};
use crate::params::{Params, is_token};
use crate::queue::{Accepted, AttributeChanges, Attributes, EXTENSION, Message, Queue, message_id};

const MAX_QUEUES: usize = 1000;
const MAX_MESSAGES: usize = 100_000;
const MAX_BODY_BYTES: usize = 1024;

type Fields = Map<String, Value>;
type Outcome = Result<Fields, Failure>;

#[derive(Debug, Default)]
pub struct Service {
    now: u64,
    queues: BTreeMap<String, Queue>,
    messages_accepted: u64,
    handles_issued: u64,
}

struct Send<'a> {
    body: &'a str,
    delay: Option<u64>,
    group: Option<&'a str>,
    deduplication_id: Option<&'a str>,
}

impl<'a> Send<'a> {
    fn parse(params: &Params<'a>) -> Result<Self, Failure> {
        let body = params.required_string("Body")?;
        if body.is_empty() || body.len() > MAX_BODY_BYTES {
            return Err(Failure::invalid_value(format!(
                "Body must be 1 to {MAX_BODY_BYTES} bytes"
            )));
        }
        Ok(Self {
            body,
            delay: params.integer("DelaySeconds", 0..=900)?,
            group: params.token("GroupId", 128)?,
            deduplication_id: params.token("DeduplicationId", 128)?,
        })
    }
}

impl Service {
    pub fn respond(&mut self, line: &[u8]) -> Value {
        match self.execute(line) {
            Ok(mut fields) => {
                fields.insert("ok".into(), true.into());
                Value::Object(fields)
            }
            Err(failure) => json!({"ok": false, "error": failure.to_json()}),
        }
    }

    fn execute(&mut self, line: &[u8]) -> Outcome {
        let invalid = |message: &str| Failure::new(Code::InvalidRequest, message);
        let text = std::str::from_utf8(line).map_err(|_| invalid("the line is not UTF-8"))?;
        let Ok(Value::Object(members)) = serde_json::from_str(text) else {
            return Err(invalid("the line is not a JSON object"));
        };
        let Some(Value::String(command)) = members.get("Command") else {
            return Err(invalid("Command must be a string"));
        };
        let params = Params::new(&members);
        match command.as_str() {
            "CreateQueue" => self.create_queue(&params),
            "DeleteQueue" => self.delete_queue(&params),
            "ListQueues" => self.list_queues(&params),
            "GetQueueAttributes" => self.get_queue_attributes(&params),
            "SetQueueAttributes" => self.set_queue_attributes(&params),
            "PurgeQueue" => self.purge_queue(&params),
            "SendMessage" => self.send_message(&params),
            "ReceiveMessage" => self.receive_message(&params),
            "DeleteMessage" => self.delete_message(&params),
            "ChangeMessageVisibility" => self.change_message_visibility(&params),
            "Tick" => self.tick(&params),
            #[cfg(feature = "extension")]
            "SendMessageBatch" => self.send_message_batch(&params),
            #[cfg(feature = "extension")]
            "DeleteMessageBatch" => self.delete_message_batch(&params),
            #[cfg(feature = "extension")]
            "TagQueue" => self.tag_queue(&params),
            #[cfg(feature = "extension")]
            "UntagQueue" => self.untag_queue(&params),
            #[cfg(feature = "extension")]
            "ListQueueTags" => self.list_queue_tags(&params),
            other => Err(invalid(&format!("unknown command {other}"))),
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue, Failure> {
        self.queues.get(name).ok_or_else(|| no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Failure> {
        self.queues.get_mut(name).ok_or_else(|| no_such_queue(name))
    }

    fn create_queue(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        if !is_queue_name(name) {
            return Err(Failure::new(
                Code::QueueNameInvalid,
                format!("{name:?} is not a queue name"),
            ));
        }
        let changes = AttributeChanges::parse(params.object("Attributes")?)?;
        let fifo = changes.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Failure::new(
                Code::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must go together",
            ));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(Failure::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = changes.dead_letter()?;
        let mut attributes = Attributes::default();
        changes.apply(dead_letter, &mut attributes);

        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == attributes {
                Ok(Fields::new())
            } else {
                Err(Failure::new(
                    Code::QueueAlreadyExists,
                    format!("{name} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter
            && self.queue(&dead_letter.queue)?.attributes.fifo != fifo
        {
            return Err(Failure::invalid_value(
                "the dead-letter queue must be of the same type",
            ));
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Failure::new(Code::LimitExceeded, "too many queues"));
        }
        self.queues.insert(name.to_owned(), Queue::new(attributes));
        Ok(Fields::new())
    }

    fn delete_queue(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        if self.queues.remove(name).is_none() {
            return Err(no_such_queue(name));
        }
        for queue in self.queues.values_mut() {
            let attributes = &mut queue.attributes;
            if attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                attributes.dead_letter = None;
            }
        }
        Ok(Fields::new())
    }

    fn list_queues(&mut self, params: &Params) -> Outcome {
        let prefix = params.string("Prefix")?.unwrap_or("");
        let names: Vec<&str> = self
            .queues
            .keys()
            .filter(|name| name.starts_with(prefix))
            .map(String::as_str)
            .collect();
        Ok(fields([("QueueNames", json!(names))]))
    }

    fn get_queue_attributes(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        let queue = self.queue(params.required_string("QueueName")?)?;
        Ok(fields([
            ("Attributes", queue.attributes.to_json()),
            ("Counts", queue.counts_json(self.now)),
        ]))
    }

    fn set_queue_attributes(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.required_string("QueueName")?;
        let changes = AttributeChanges::parse(Some(params.required_object("Attributes")?))?;
        let fifo = self.queue(name)?.attributes.fifo;
        if changes.fifo.is_some() {
            return Err(Failure::new(
                Code::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(Failure::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = changes.dead_letter()?;
        if let Some(dead_letter) = &dead_letter
            && (self.queue(&dead_letter.queue)?.attributes.fifo != fifo
                || dead_letter.queue == name)
        {
            return Err(Failure::invalid_value(
                "the dead-letter queue must be another queue of the same type",
            ));
        }
        let now = self.now;
        let queue = self.queue_mut(name)?;
        changes.apply(dead_letter, &mut queue.attributes);
        queue.remove_expired(now);
        Ok(Fields::new())
    }

    fn purge_queue(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        self.queue_mut(params.required_string("QueueName")?)?
            .purge();
        Ok(Fields::new())
    }

    fn send_message(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "Body"])?;
        let name = params.required_string("QueueName")?;
        let send = Send::parse(params)?;
        self.send(name, &send)
    }

    fn send(&mut self, queue_name: &str, send: &Send) -> Outcome {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(queue_name)
            .ok_or_else(|| no_such_queue(queue_name))?;
        if EXTENSION && send.body.len() as u64 > queue.attributes.max_message_bytes {
            return Err(Failure::invalid_value(
                "Body is longer than the queue's MaxMessageBytes",
            ));
        }
        let unsupported = |parameter: &str, kind: &str| {
            Failure::new(
                Code::UnsupportedOperation,
                format!("{parameter} is not supported on a {kind} queue"),
            )
        };
        let deduplication_id = if queue.attributes.fifo {
            if send.delay.is_some() {
                return Err(unsupported("DelaySeconds", "FIFO"));
            }
            if send.group.is_none() {
                return Err(Failure::missing("GroupId"));
            }
            let id = match send.deduplication_id {
                Some(id) => id,
                None if queue.attributes.content_based_deduplication => send.body,
                None => return Err(Failure::missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(id, now) {
                return Ok(fields([
                    ("MessageId", message_id(original.message_id).into()),
                    ("SequenceNumber", original.sequence.into()),
                ]));
            }
            Some(id)
        } else {
            if send.group.is_some() {
                return Err(unsupported("GroupId", "standard"));
            }
            if send.deduplication_id.is_some() {
                return Err(unsupported("DeduplicationId", "standard"));
            }
            None
        };
        if queue.len() >= MAX_MESSAGES {
            return Err(Failure::new(Code::LimitExceeded, "the queue is full"));
        }

        self.messages_accepted += 1;
        let id = self.messages_accepted;
        let delay = send.delay.unwrap_or(queue.attributes.delay_seconds);
        let sequence = queue.add(Message::new(id, send.body, now, delay, send.group));
        let mut result = fields([("MessageId", message_id(id).into())]);
        if let Some(deduplication_id) = deduplication_id {
            let accepted = Accepted {
                at: now,
                message_id: id,
                sequence,
            };
            queue.remember(deduplication_id, accepted);
            result.insert("SequenceNumber".into(), sequence.into());
        }
        Ok(result)
    }

    fn receive_message(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| no_such_queue(name))?;
        let received = queue.receive(
            now,
            max_messages as usize,
            visibility_timeout.unwrap_or(queue.attributes.visibility_timeout),
            &mut self.handles_issued,
        );
        let target = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        if let Some(target) = target.and_then(|target| self.queues.get_mut(&target)) {
            for message in received.dead_lettered {
                target.add_dead_letter(message, now);
            }
            target.remove_expired(now);
        }
        Ok(fields([("Messages", Value::Array(received.delivered))]))
    }

    fn issued_handle(&self, text: &str) -> Result<u64, Failure> {
        text.strip_prefix("r-")
            .and_then(|digits| digits.parse::<u64>().ok())
            .filter(|number| (1..=self.handles_issued).contains(number))
            .filter(|number| format!("r-{number}") == text)
            .ok_or_else(|| {
                Failure::new(
                    Code::ReceiptHandleInvalid,
                    format!("{text:?} was never issued"),
                )
            })
    }

    fn delete_by_handle(&mut self, queue_name: &str, handle: &str) -> Result<(), Failure> {
        let handle = self.issued_handle(handle)?;
        let now = self.now;
        if self.queue_mut(queue_name)?.delete(handle, now) {
            Ok(())
        } else {
            Err(expired_handle())
        }
    }

    fn delete_message(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        self.queue(name)?;
        self.delete_by_handle(name, handle)?;
        Ok(Fields::new())
    }

    fn change_message_visibility(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let visibility_timeout = params.required_integer("VisibilityTimeout", 0..=43_200)?;
        self.queue(name)?;
        let handle = self.issued_handle(handle)?;
        let now = self.now;
        if self
            .queue_mut(name)?
            .change_visibility(handle, now, visibility_timeout)
        {
            Ok(Fields::new())
        } else {
            Err(expired_handle())
        }
    }

    fn tick(&mut self, params: &Params) -> Outcome {
        params.require(&["Seconds"])?;
        self.now += params.required_integer("Seconds", 0..=1_209_600)?;
        for queue in self.queues.values_mut() {
            queue.remove_expired(self.now);
            queue.forget_closed_windows(self.now);
        }
        Ok(fields([("Now", self.now.into())]))
    }
}

fn fields<const N: usize>(members: [(&str, Value); N]) -> Fields {
    members
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn no_such_queue(name: &str) -> Failure {
    Failure::new(Code::QueueDoesNotExist, format!("no queue named {name:?}"))
}

fn expired_handle() -> Failure {
    Failure::new(
        Code::ReceiptHandleExpired,
        "the receipt handle is not the latest handle of a message in flight in this queue",
    )
}

fn is_queue_name(name: &str) -> bool {
    name.len() <= 80 && is_token(name.strip_suffix(".fifo").unwrap_or(name), 80)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(service: &mut Service, line: &str) -> Value {
        service.respond(line.as_bytes())
    }

    #[test]
    fn queue_names() {
        assert!(is_queue_name("a"));
        assert!(is_queue_name(&"a".repeat(80)));
        assert!(is_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_queue_name(&"a".repeat(81)));
        assert!(!is_queue_name(".fifo"));
        assert!(!is_queue_name("a.b"));
        assert!(!is_queue_name(""));
    }

    #[test]
    fn send_receive_delete() {
        let mut service = Service::default();
        run(&mut service, r#"{"Command":"CreateQueue","QueueName":"q"}"#);
        let sent = run(
            &mut service,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        );
        assert_eq!(sent, json!({"ok": true, "MessageId": "m-1"}));
        let received = run(
            &mut service,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        );
        assert_eq!(received["Messages"][0]["ReceiptHandle"], "r-1");
        let stale = r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#;
        assert_eq!(
            run(&mut service, stale)["error"]["code"],
            "ReceiptHandleInvalid"
        );
        let delete = r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#;
        assert_eq!(run(&mut service, delete), json!({"ok": true}));
        assert_eq!(
            run(&mut service, delete)["error"]["code"],
            "ReceiptHandleExpired"
        );
    }
}
