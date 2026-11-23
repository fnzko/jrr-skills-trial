//! All state of the process and the commands that change it.

mod message_ops;
mod queue_ops;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{Param, Params, Rule};
use crate::queue::{Queue, Time};

/// The members of a success response besides `ok`.
pub type Output = Map<String, Value>;

pub const MAX_QUEUES: usize = 1000;

const QUEUE_NAME: Param = Param::required("QueueName", Rule::Str);
const TICK: &[Param] = &[Param::required(
    "Seconds",
    Rule::Int {
        min: 0,
        max: 1_209_600,
    },
)];

#[derive(Debug, Default)]
pub struct Broker {
    now: Time,
    queues: BTreeMap<String, Queue>,
    /// Number of messages accepted so far (`MessageId` is `m-<n>`).
    messages_accepted: u64,
    /// Number of deliveries so far (`ReceiptHandle` is `r-<n>`).
    deliveries: u64,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one command given as a JSON object.
    pub fn execute(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let command = object
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "Command must be a string"))?;
        match command {
            "Tick" => self.tick(&Params::parse(TICK, object)?),
            "CreateQueue" => self.create_queue(object),
            "SetQueueAttributes" => self.set_queue_attributes(object),
            "GetQueueAttributes" => self.get_queue_attributes(object),
            "ListQueues" => self.list_queues(object),
            "DeleteQueue" => self.delete_queue(object),
            "PurgeQueue" => self.purge_queue(object),
            "SendMessage" => self.send_message(object),
            "ReceiveMessage" => self.receive_message(object),
            "DeleteMessage" => self.delete_message(object),
            "ChangeMessageVisibility" => self.change_message_visibility(object),
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {other:?}"),
            )),
        }
    }

    fn tick(&mut self, params: &Params) -> Result<Output> {
        self.now += params.required_int("Seconds") as Time;
        for queue in self.queues.values_mut() {
            queue.expire(self.now);
        }
        Ok(output(json!({ "Now": self.now })))
    }

    /// Queue lookup (SPEC section 2 step 4): the name must match exactly.
    fn queue(&self, name: &str) -> Result<&Queue> {
        self.queues.get(name).ok_or_else(|| no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues.get_mut(name).ok_or_else(|| no_such_queue(name))
    }
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Turns a `json!({...})` object literal into command output.
fn output(value: Value) -> Output {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("command output is always an object"),
    }
}
