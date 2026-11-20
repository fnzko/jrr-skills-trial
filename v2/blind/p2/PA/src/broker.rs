//! The broker: all service state plus command dispatch.

mod batch_commands;
mod message_commands;
mod queue_commands;
mod tag_commands;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::queue::Queue;
use crate::request::{Params, Request};

/// Largest `Seconds` accepted by `Tick`.
const MAX_TICK: u64 = 1_209_600;

/// Result members of a successful command (the `ok` member is added later).
type Members = Map<String, Value>;

/// The whole service state. Time is virtual and only `Tick` moves it.
pub struct Broker {
    now: u64,
    /// The next number for `MessageId` (`m-<n>`), counted over all queues.
    next_message_id: u64,
    /// The next number for `ReceiptHandle` (`r-<n>`), counted over all queues.
    next_receipt_handle: u64,
    /// Ordered by name, which is the order `ListQueues` needs.
    queues: BTreeMap<String, Queue>,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: 0,
            next_message_id: 1,
            next_receipt_handle: 1,
            queues: BTreeMap::new(),
        }
    }
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one command line (without its newline) and returns the response
    /// line (without its newline).
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match Request::parse(line).and_then(|req| self.dispatch(&req)) {
            Ok(mut members) => {
                members.insert("ok".into(), Value::Bool(true));
                Value::Object(members)
            }
            Err(Error { code, message }) => json!({
                "ok": false,
                "error": { "code": code.as_str(), "message": message },
            }),
        };
        response.to_string()
    }

    fn dispatch(&mut self, req: &Request) -> Result<Members> {
        let params = req.params();
        match req.command.as_str() {
            "CreateQueue" => self.create_queue(params),
            "SetQueueAttributes" => self.set_queue_attributes(params),
            "GetQueueAttributes" => self.get_queue_attributes(params),
            "ListQueues" => self.list_queues(params),
            "DeleteQueue" => self.delete_queue(params),
            "PurgeQueue" => self.purge_queue(params),
            "SendMessage" => self.send_message(params),
            "SendMessageBatch" => self.send_message_batch(params),
            "DeleteMessageBatch" => self.delete_message_batch(params),
            "ReceiveMessage" => self.receive_message(params),
            "DeleteMessage" => self.delete_message(params),
            "ChangeMessageVisibility" => self.change_message_visibility(params),
            "TagQueue" => self.tag_queue(params),
            "UntagQueue" => self.untag_queue(params),
            "ListQueueTags" => self.list_queue_tags(params),
            "Tick" => self.tick(params),
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command `{other}`"),
            )),
        }
    }

    fn tick(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["Seconds"])?;
        let seconds = params.integer("Seconds", 0..=MAX_TICK)?.expect("required");
        self.now += seconds;
        Ok(members([("Now", json!(self.now))]))
    }

    /// Looks up a queue (SPEC section 2, check 4) with expired messages
    /// already removed.
    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        lookup(&mut self.queues, self.now, name)
    }
}

/// [`Broker::queue_mut`] on the queue map alone, for callers that need to
/// touch other broker fields while holding the queue.
fn lookup<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue `{name}` does not exist"),
    )
}

/// Builds a result member map from name/value pairs.
fn members<const N: usize>(pairs: [(&str, Value); N]) -> Members {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}
