use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::error::Failure;
use crate::params::Params;

pub const EXTENSION: bool = cfg!(feature = "extension");

const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
    pub max_message_bytes: u64,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
            max_message_bytes: 1024,
        }
    }
}

impl Attributes {
    pub fn to_json(&self) -> Value {
        let mut members = Map::new();
        members.insert("VisibilityTimeout".into(), self.visibility_timeout.into());
        members.insert("DelaySeconds".into(), self.delay_seconds.into());
        members.insert("RetentionSeconds".into(), self.retention_seconds.into());
        members.insert("Fifo".into(), self.fifo.into());
        members.insert(
            "ContentBasedDeduplication".into(),
            self.content_based_deduplication.into(),
        );
        if let Some(dead_letter) = &self.dead_letter {
            members.insert("DeadLetterQueue".into(), dead_letter.queue.as_str().into());
            members.insert(
                "MaxReceiveCount".into(),
                dead_letter.max_receive_count.into(),
            );
        }
        if EXTENSION {
            members.insert("MaxMessageBytes".into(), self.max_message_bytes.into());
        }
        Value::Object(members)
    }
}

#[derive(Debug, Default)]
pub struct AttributeChanges {
    visibility_timeout: Option<u64>,
    delay_seconds: Option<u64>,
    retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u64>,
    max_message_bytes: Option<u64>,
}

impl AttributeChanges {
    pub fn parse(attributes: Option<Params<'_>>) -> Result<Self, Failure> {
        let Some(attributes) = attributes else {
            return Ok(Self::default());
        };
        let mut known = vec![
            "VisibilityTimeout",
            "DelaySeconds",
            "RetentionSeconds",
            "Fifo",
            "ContentBasedDeduplication",
            "DeadLetterQueue",
            "MaxReceiveCount",
        ];
        if EXTENSION {
            known.push("MaxMessageBytes");
        }
        if let Some((name, _)) = attributes.present().find(|(name, _)| !known.contains(name)) {
            return Err(Failure::invalid_value(format!("unknown attribute {name}")));
        }
        Ok(Self {
            visibility_timeout: attributes.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: attributes.integer("DelaySeconds", 0..=900)?,
            retention_seconds: attributes.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: attributes.integer("MaxReceiveCount", 1..=1000)?,
            max_message_bytes: attributes.integer("MaxMessageBytes", 1..=1024)?,
        })
    }

    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Failure> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Failure::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    pub fn apply(&self, dead_letter: Option<DeadLetter>, attributes: &mut Attributes) {
        let Attributes {
            visibility_timeout,
            delay_seconds,
            retention_seconds,
            fifo,
            content_based_deduplication,
            dead_letter: current_dead_letter,
            max_message_bytes,
        } = attributes;
        *visibility_timeout = self.visibility_timeout.unwrap_or(*visibility_timeout);
        *delay_seconds = self.delay_seconds.unwrap_or(*delay_seconds);
        *retention_seconds = self.retention_seconds.unwrap_or(*retention_seconds);
        *fifo = self.fifo.unwrap_or(*fifo);
        *content_based_deduplication = self
            .content_based_deduplication
            .unwrap_or(*content_based_deduplication);
        *max_message_bytes = self.max_message_bytes.unwrap_or(*max_message_bytes);
        if dead_letter.is_some() {
            *current_dead_letter = dead_letter;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
struct Flight {
    handle: u64,
    until: u64,
}

#[derive(Debug)]
pub struct Message {
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    pub visible_from: u64,
    pub group: Option<String>,
    receive_count: u64,
    first_received_at: Option<u64>,
    flight: Option<Flight>,
}

impl Message {
    pub fn new(id: u64, body: &str, sent_at: u64, delay: u64, group: Option<&str>) -> Self {
        Self {
            id,
            body: body.to_owned(),
            sent_at,
            visible_from: sent_at + delay,
            group: group.map(str::to_owned),
            receive_count: 0,
            first_received_at: None,
            flight: None,
        }
    }

    fn state(&self, now: u64) -> State {
        if now < self.visible_from {
            State::Delayed
        } else if self
            .flight
            .as_ref()
            .is_some_and(|flight| now < flight.until)
        {
            State::InFlight
        } else {
            State::Visible
        }
    }

    fn dead_lettered(self, now: u64) -> Self {
        Self {
            visible_from: now,
            receive_count: 0,
            first_received_at: None,
            flight: None,
            ..self
        }
    }

    fn delivery_json(&self, sequence: u64, handle: u64) -> Value {
        let mut members = Map::new();
        members.insert("MessageId".into(), message_id(self.id).into());
        members.insert("ReceiptHandle".into(), format!("r-{handle}").into());
        members.insert("Body".into(), self.body.as_str().into());
        members.insert("ReceiveCount".into(), self.receive_count.into());
        members.insert("SentAt".into(), self.sent_at.into());
        members.insert("FirstReceivedAt".into(), self.first_received_at.into());
        if let Some(group) = &self.group {
            members.insert("GroupId".into(), group.as_str().into());
            members.insert("SequenceNumber".into(), sequence.into());
        }
        Value::Object(members)
    }
}

pub fn message_id(number: u64) -> String {
    format!("m-{number}")
}

#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub at: u64,
    pub message_id: u64,
    pub sequence: u64,
}

#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<Value>,
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    pub tags: BTreeMap<String, String>,
    messages: BTreeMap<u64, Message>,
    arrivals: u64,
    accepted: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            tags: BTreeMap::new(),
            messages: BTreeMap::new(),
            arrivals: 0,
            accepted: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn count(&self, state: State, now: u64) -> usize {
        self.messages
            .values()
            .filter(|message| message.state(now) == state)
            .count()
    }

    pub fn add(&mut self, message: Message) -> u64 {
        self.arrivals += 1;
        self.messages.insert(self.arrivals, message);
        self.arrivals
    }

    pub fn add_dead_letter(&mut self, message: Message, now: u64) {
        self.add(message.dead_lettered(now));
    }

    pub fn purge(&mut self) {
        self.messages.clear();
    }

    pub fn remove_expired(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        self.messages
            .retain(|_, message| now < message.sent_at + retention);
    }

    pub fn forget_closed_windows(&mut self, now: u64) {
        self.accepted
            .retain(|_, accepted| now - accepted.at < DEDUPLICATION_WINDOW);
    }

    pub fn duplicate_of(&self, deduplication_id: &str, now: u64) -> Option<Accepted> {
        self.accepted
            .get(deduplication_id)
            .filter(|accepted| now - accepted.at < DEDUPLICATION_WINDOW)
            .copied()
    }

    pub fn remember(&mut self, deduplication_id: &str, accepted: Accepted) {
        self.accepted.insert(deduplication_id.to_owned(), accepted);
    }

    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: u64,
        handles_issued: &mut u64,
    ) -> Received {
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut groups_with_earlier_message = HashSet::new();
        let mut to_deliver = Vec::new();
        let mut to_dead_letter = Vec::new();
        for (&sequence, message) in &self.messages {
            if to_deliver.len() == max_messages {
                break;
            }
            let group = message.group.as_deref();
            if group.is_some_and(|group| groups_with_earlier_message.contains(group)) {
                continue;
            }
            let visible = message.state(now) == State::Visible;
            if visible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                to_dead_letter.push(sequence);
                continue;
            }
            if let Some(group) = group {
                groups_with_earlier_message.insert(group);
            }
            if visible {
                to_deliver.push(sequence);
            }
        }

        let mut received = Received::default();
        for sequence in to_deliver {
            if let Some(message) = self.messages.get_mut(&sequence) {
                *handles_issued += 1;
                message.receive_count += 1;
                message.first_received_at.get_or_insert(now);
                message.flight = Some(Flight {
                    handle: *handles_issued,
                    until: now + visibility_timeout,
                });
                received
                    .delivered
                    .push(message.delivery_json(sequence, *handles_issued));
            }
        }
        received.dead_lettered = to_dead_letter
            .iter()
            .filter_map(|sequence| self.messages.remove(sequence))
            .collect();
        received
    }

    fn in_flight(&mut self, handle: u64, now: u64) -> Option<(u64, &mut Message)> {
        self.messages
            .iter_mut()
            .find(|(_, message)| {
                message
                    .flight
                    .as_ref()
                    .is_some_and(|flight| flight.handle == handle && now < flight.until)
            })
            .map(|(sequence, message)| (*sequence, message))
    }

    pub fn delete(&mut self, handle: u64, now: u64) -> bool {
        match self.in_flight(handle, now) {
            Some((sequence, _)) => self.messages.remove(&sequence).is_some(),
            None => false,
        }
    }

    pub fn change_visibility(&mut self, handle: u64, now: u64, visibility_timeout: u64) -> bool {
        match self.in_flight(handle, now) {
            Some((_, message)) => {
                message.flight = Some(Flight {
                    handle,
                    until: now + visibility_timeout,
                });
                true
            }
            None => false,
        }
    }

    pub fn counts_json(&self, now: u64) -> Value {
        json!({
            "Visible": self.count(State::Visible, now),
            "InFlight": self.count(State::InFlight, now),
            "Delayed": self.count(State::Delayed, now),
        })
    }
}
