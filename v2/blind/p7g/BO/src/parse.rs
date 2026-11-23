//! Turns one request line into a [`Command`], applying the checks of spec
//! section 2 steps 1 to 3 in order.

use serde_json::{Map, Value};

use crate::command::{AttributeSet, Command, NewMessage};
use crate::error::{Error, Problem, RequestFault};
use crate::types::{
    Body, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

/// A JSON object whose `null` members count as absent.
struct Fields<'a>(&'a Map<String, Value>);

impl<'a> Fields<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    /// Step 2: fails on the first of `names` that is absent.
    fn require(&self, names: &[&'static str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::MissingParameter(name)),
            None => Ok(()),
        }
    }

    fn try_string<T>(
        &self,
        name: &'static str,
        build: impl FnOnce(&str) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let text = value
            .as_str()
            .ok_or_else(|| Error::invalid(name, Problem::WrongType("a string")))?;
        build(text).map(Some)
    }

    fn string<T>(
        &self,
        name: &'static str,
        build: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>, Error> {
        self.try_string(name, |text| {
            build(text).ok_or_else(|| Error::invalid(name, Problem::OutOfRange))
        })
    }

    fn owned_string(&self, name: &'static str) -> Result<Option<String>, Error> {
        self.string(name, |s| Some(s.to_owned()))
    }

    fn integer<T>(
        &self,
        name: &'static str,
        build: impl FnOnce(i64) -> Option<T>,
    ) -> Result<Option<T>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let number = value
            .as_i64()
            .ok_or_else(|| Error::invalid(name, Problem::WrongType("an integer")))?;
        build(number)
            .map(Some)
            .ok_or_else(|| Error::invalid(name, Problem::OutOfRange))
    }

    fn boolean(&self, name: &'static str) -> Result<Option<bool>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        value
            .as_bool()
            .map(Some)
            .ok_or_else(|| Error::invalid(name, Problem::WrongType("a boolean")))
    }

    fn object(&self, name: &'static str) -> Result<Option<Fields<'a>>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        value
            .as_object()
            .map(|map| Some(Fields(map)))
            .ok_or_else(|| Error::invalid(name, Problem::WrongType("an object")))
    }

    /// A parameter that `require` has already shown to be present.
    fn required_string(&self, name: &'static str) -> Result<String, Error> {
        self.owned_string(name)?
            .ok_or(Error::MissingParameter(name))
    }

    fn required_with<T>(
        &self,
        name: &'static str,
        build: impl FnOnce(i64) -> Option<T>,
    ) -> Result<T, Error> {
        self.integer(name, build)?
            .ok_or(Error::MissingParameter(name))
    }
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

fn attributes_of(fields: &Fields<'_>) -> Result<AttributeSet, Error> {
    if let Some(unknown) = fields
        .0
        .keys()
        .find(|key| !ATTRIBUTE_NAMES.contains(&key.as_str()))
    {
        return Err(Error::invalid(unknown, Problem::UnknownAttribute));
    }
    Ok(AttributeSet {
        visibility_timeout: fields.integer("VisibilityTimeout", VisibilityTimeout::new)?,
        delay: fields.integer("DelaySeconds", DelaySeconds::new)?,
        retention: fields.integer("RetentionSeconds", RetentionSeconds::new)?,
        fifo: fields.boolean("Fifo")?,
        content_based_deduplication: fields.boolean("ContentBasedDeduplication")?,
        dead_letter_queue: fields.owned_string("DeadLetterQueue")?,
        max_receive_count: fields.integer("MaxReceiveCount", MaxReceiveCount::new)?,
    })
}

/// Parses one input line, without its line terminator.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let text =
        std::str::from_utf8(line).map_err(|_| Error::InvalidRequest(RequestFault::NotUtf8))?;
    let value: Value =
        serde_json::from_str(text).map_err(|_| Error::InvalidRequest(RequestFault::NotJson))?;
    let Value::Object(map) = &value else {
        return Err(Error::InvalidRequest(RequestFault::NotObject));
    };
    let fields = Fields(map);
    let name = fields
        .get("Command")
        .and_then(Value::as_str)
        .ok_or(Error::InvalidRequest(RequestFault::MissingCommand))?;
    match name {
        "CreateQueue" => create_queue(&fields),
        "SetQueueAttributes" => {
            fields.require(&["QueueName", "Attributes"])?;
            let queue = fields.required_string("QueueName")?;
            let attributes = fields
                .object("Attributes")?
                .ok_or(Error::MissingParameter("Attributes"))?;
            Ok(Command::SetQueueAttributes {
                queue,
                attributes: attributes_of(&attributes)?,
            })
        }
        "GetQueueAttributes" => queue_only(&fields, |queue| Command::GetQueueAttributes { queue }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: fields.owned_string("Prefix")?,
        }),
        "DeleteQueue" => queue_only(&fields, |queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => queue_only(&fields, |queue| Command::PurgeQueue { queue }),
        "Tick" => {
            fields.require(&["Seconds"])?;
            Ok(Command::Tick {
                seconds: fields.required_with("Seconds", TickSeconds::new)?,
            })
        }
        "SendMessage" => send_message(&fields),
        "ReceiveMessage" => {
            fields.require(&["QueueName"])?;
            Ok(Command::ReceiveMessage {
                queue: fields.required_string("QueueName")?,
                max: fields
                    .integer("MaxMessages", MaxMessages::new)?
                    .unwrap_or_default(),
                visibility_timeout: fields.integer("VisibilityTimeout", VisibilityTimeout::new)?,
            })
        }
        "DeleteMessage" => {
            fields.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Command::DeleteMessage {
                queue: fields.required_string("QueueName")?,
                receipt_handle: fields.required_string("ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            fields.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Command::ChangeMessageVisibility {
                queue: fields.required_string("QueueName")?,
                receipt_handle: fields.required_string("ReceiptHandle")?,
                visibility_timeout: fields
                    .required_with("VisibilityTimeout", VisibilityTimeout::new)?,
            })
        }
        _ => Err(Error::InvalidRequest(RequestFault::UnknownCommand)),
    }
}

fn queue_only(
    fields: &Fields<'_>,
    build: impl FnOnce(String) -> Command,
) -> Result<Command, Error> {
    fields.require(&["QueueName"])?;
    fields.required_string("QueueName").map(build)
}

fn create_queue(fields: &Fields<'_>) -> Result<Command, Error> {
    fields.require(&["QueueName"])?;
    let name = fields
        .try_string("QueueName", |s| {
            QueueName::new(s).map_err(Error::QueueNameInvalid)
        })?
        .ok_or(Error::MissingParameter("QueueName"))?;
    let attributes = match fields.object("Attributes")? {
        Some(given) => attributes_of(&given)?,
        None => AttributeSet::default(),
    };
    Ok(Command::CreateQueue { name, attributes })
}

fn send_message(fields: &Fields<'_>) -> Result<Command, Error> {
    fields.require(&["QueueName", "Body"])?;
    let queue = fields.required_string("QueueName")?;
    let body = fields
        .string("Body", Body::new)?
        .ok_or(Error::MissingParameter("Body"))?;
    Ok(Command::SendMessage {
        queue,
        message: NewMessage {
            body,
            delay: fields.integer("DelaySeconds", DelaySeconds::new)?,
            group: fields.string("GroupId", GroupId::new)?,
            deduplication: fields.string("DeduplicationId", DeduplicationId::new)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> &'static str {
        parse(line.as_bytes())
            .expect_err("line must be rejected")
            .code()
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        let cases = [
            ("", "InvalidRequest"),
            ("[]", "InvalidRequest"),
            ("{}", "InvalidRequest"),
            (r#"{"Command": 5}"#, "InvalidRequest"),
            (r#"{"Command": null}"#, "InvalidRequest"),
            (r#"{"Command": "tick"}"#, "InvalidRequest"),
            (r#"{"Command": "Tick"} x"#, "InvalidRequest"),
        ];
        for (line, expected) in cases {
            assert_eq!(code(line), expected, "{line}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code(),
            "InvalidRequest"
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), "MissingParameter");
        let null_is_absent = r#"{"Command": "Tick", "Seconds": null}"#;
        assert_eq!(code(null_is_absent), "MissingParameter");
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for seconds in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(code(&line), "InvalidParameterValue", "{seconds}");
        }
        let line = r#"{"Command": "Tick", "Seconds": 1209600}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::Tick {
                seconds: TickSeconds::new(1_209_600).unwrap()
            })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), "QueueNameInvalid");
        let line = r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 3}"#;
        assert_eq!(code(line), "InvalidParameterValue");
        let line = r#"{"Command": "CreateQueue", "QueueName": 7, "Attributes": 3}"#;
        assert_eq!(code(line), "InvalidParameterValue");
    }

    #[test]
    fn unknown_attributes_are_rejected_even_when_null() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Color": null}}"#;
        assert_eq!(code(line), "InvalidParameterValue");
    }

    #[test]
    fn other_commands_accept_any_queue_name_string() {
        let line = r#"{"Command": "PurgeQueue", "QueueName": "not a valid name!", "Extra": 1}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::PurgeQueue {
                queue: "not a valid name!".into()
            })
        );
    }

    #[test]
    fn send_message_parameters_are_validated_on_their_own() {
        let ok = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}"#;
        assert!(parse(ok.as_bytes()).is_ok());
        let bad = [
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": 4}"#,
        ];
        for line in bad {
            assert_eq!(code(line), "InvalidParameterValue", "{line}");
        }
    }
}
