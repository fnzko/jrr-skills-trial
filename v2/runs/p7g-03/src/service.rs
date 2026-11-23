//! The service: request parsing, dispatch and response shaping.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::error::{Code, Error, Result};
use crate::params::Params;
use crate::queue::Queue;

/// Largest value of the virtual clock a command may add up to; behaviour past
/// it is unspecified, so `Tick` simply saturates.
const MAX_TICK: i64 = 1_209_600;

#[derive(Default)]
pub struct Service {
    pub(crate) now: u64,
    pub(crate) queues: HashMap<String, Queue>,
    next_queue_uid: u64,
    /// Messages accepted so far, over all queues (the last `MessageId`).
    pub(crate) messages_accepted: u64,
    /// `(queue uid, arrival key)` of each delivery; `r-n` is entry `n - 1`.
    pub(crate) deliveries: Vec<(u64, u64)>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line (without its `\n`) and returns the response object.
    pub fn handle_line(&mut self, line: &[u8]) -> Value {
        match self.execute(line) {
            Ok(result) => {
                let mut response = Map::new();
                response.insert("ok".into(), Value::Bool(true));
                response.extend(result);
                Value::Object(response)
            }
            Err(e) => json!({
                "ok": false,
                "error": {"code": e.code.as_str(), "message": e.message},
            }),
        }
    }

    fn execute(&mut self, line: &[u8]) -> Result<Map<String, Value>> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::invalid_request("line is not valid UTF-8"))?;
        let Ok(Value::Object(members)) = serde_json::from_str::<Value>(text) else {
            return Err(Error::invalid_request("line is not a JSON object"));
        };
        let Some(Value::String(command)) = members.get("Command") else {
            return Err(Error::invalid_request("Command must be a string"));
        };
        let params = Params::new(&members);
        match command.as_str() {
            "Tick" => self.tick(&params),
            "CreateQueue" => self.create_queue(&params),
            "SetQueueAttributes" => self.set_queue_attributes(&params),
            "GetQueueAttributes" => self.get_queue_attributes(&params),
            "ListQueues" => self.list_queues(&params),
            "DeleteQueue" => self.delete_queue(&params),
            "PurgeQueue" => self.purge_queue(&params),
            "SendMessage" => self.send_message(&params),
            "ReceiveMessage" => self.receive_message(&params),
            "DeleteMessage" => self.delete_message(&params),
            "ChangeMessageVisibility" => self.change_message_visibility(&params),
            other => Err(Error::invalid_request(format!("unknown command {other}"))),
        }
    }

    fn tick(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("Seconds")?;
        let seconds = params.integer("Seconds", 0..=MAX_TICK)?.unwrap_or(0);
        self.now += seconds as u64;
        Ok(object([("Now", json!(self.now))]))
    }
}

impl Service {
    pub(crate) fn next_queue_uid(&mut self) -> u64 {
        self.next_queue_uid += 1;
        self.next_queue_uid
    }

    /// Looks a queue up by exact name, first applying retention so that every
    /// command sees the queue as it is at `now`.
    pub(crate) fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        lookup(&mut self.queues, self.now, name)
    }
}

/// Like [`Service::queue_mut`], but borrows only the queue map, so callers can
/// keep using the other fields of the service meanwhile.
pub(crate) fn lookup<'a>(
    queues: &'a mut HashMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

pub(crate) fn no_such_queue(name: &str) -> Error {
    Error::new(
        Code::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Builds a result object from `(member, value)` pairs.
pub(crate) fn object<const N: usize>(members: [(&str, Value); N]) -> Map<String, Value> {
    members
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}
