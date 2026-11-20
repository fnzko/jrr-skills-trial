//! The whole service state and command dispatch.

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode, Result};
use std::collections::BTreeMap;

use crate::params::Params;
use crate::queue::{Queue, queue_does_not_exist};

/// Fields a successful command adds to `{"ok": true}`.
pub(crate) type Fields = Map<String, Value>;

/// The virtual clock may not be advanced by more than this in one `Tick`.
const MAX_TICK: i64 = 1_209_600;

#[derive(Debug, Default)]
pub struct Broker {
    /// The virtual clock in seconds; only `Tick` advances it.
    pub(crate) now: u64,
    /// Ordered by name, which is the order `ListQueues` reports.
    pub(crate) queues: BTreeMap<String, Queue>,
    queues_created: u64,
    /// Messages accepted so far, which numbers the next `MessageId`.
    pub(crate) messages_accepted: u64,
    /// Every delivery so far as `(queue id, arrival key)`; handle `r-<n>` is entry `n - 1`.
    pub(crate) handles: Vec<(u64, u64)>,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes one input line (without its newline) and returns the response object.
    pub fn handle_line(&mut self, line: &[u8]) -> Value {
        match self.execute(line) {
            Ok(mut fields) => {
                fields.insert("ok".into(), Value::Bool(true));
                Value::Object(fields)
            }
            Err(error) => json!({
                "ok": false,
                "error": { "code": error.code.as_str(), "message": error.message },
            }),
        }
    }

    fn execute(&mut self, line: &[u8]) -> Result<Fields> {
        let request = parse_request(line)?;
        let params = Params::new(&request);
        let command = request
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_request("Command must be a string"))?;
        match command {
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
            "Tick" => self.tick(&params),
            other => Err(invalid_request(format!("unknown command {other:?}"))),
        }
    }

    pub(crate) fn next_queue_id(&mut self) -> u64 {
        self.queues_created += 1;
        self.queues_created
    }

    /// Looks a queue up by exact name, first dropping messages that have expired.
    pub(crate) fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| queue_does_not_exist(name))?;
        queue.expire(now);
        Ok(queue)
    }

    fn tick(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["Seconds"])?;
        let seconds = params.integer("Seconds", 0..=MAX_TICK)?.unwrap_or(0);
        self.now += seconds as u64;
        Ok(fields([("Now", json!(self.now))]))
    }
}

fn parse_request(line: &[u8]) -> Result<Map<String, Value>> {
    let text = std::str::from_utf8(line).map_err(|_| invalid_request("line is not valid UTF-8"))?;
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(invalid_request("line is not a JSON object")),
        Err(error) => Err(invalid_request(format!("line is not valid JSON: {error}"))),
    }
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

pub(crate) fn fields<const N: usize>(members: [(&str, Value); N]) -> Fields {
    members
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}
