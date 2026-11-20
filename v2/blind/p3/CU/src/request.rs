//! Parsing of command lines and typed access to their parameters.
//!
//! Parameter access follows the check order of SPEC section 2: a handler
//! first calls [`Params::require`] for all required names, then reads each
//! value with a typed getter, which reports `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::time::Seconds;

/// The commands miniq implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue,
    SetQueueAttributes,
    GetQueueAttributes,
    ListQueues,
    DeleteQueue,
    PurgeQueue,
    SendMessage,
    ReceiveMessage,
    DeleteMessage,
    ChangeMessageVisibility,
    Tick,
}

impl Command {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "CreateQueue" => Self::CreateQueue,
            "SetQueueAttributes" => Self::SetQueueAttributes,
            "GetQueueAttributes" => Self::GetQueueAttributes,
            "ListQueues" => Self::ListQueues,
            "DeleteQueue" => Self::DeleteQueue,
            "PurgeQueue" => Self::PurgeQueue,
            "SendMessage" => Self::SendMessage,
            "ReceiveMessage" => Self::ReceiveMessage,
            "DeleteMessage" => Self::DeleteMessage,
            "ChangeMessageVisibility" => Self::ChangeMessageVisibility,
            "Tick" => Self::Tick,
            _ => return None,
        })
    }
}

/// A parsed command line: the command and its parameter object.
pub struct Request {
    pub command: Command,
    pub members: Map<String, Value>,
}

impl Request {
    /// Checks the request shape (SPEC section 2, check 1), which includes the
    /// command being a known one.
    pub fn parse(line: &[u8]) -> Result<Self> {
        let invalid = |msg: &str| Error::new(ErrorCode::InvalidRequest, msg);
        let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8"))?;
        let value: Value =
            serde_json::from_str(text).map_err(|e| invalid(&format!("invalid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(invalid("request is not a JSON object"));
        };
        let Some(Value::String(name)) = members.get("Command") else {
            return Err(invalid("`Command` is missing or not a string"));
        };
        let command = Command::from_name(name)
            .ok_or_else(|| invalid(&format!("unknown command `{name}`")))?;
        Ok(Self { command, members })
    }

    pub fn params(&self) -> Params<'_> {
        Params::new(&self.members)
    }
}

/// Read-only typed view of a JSON object. `null` members count as absent.
#[derive(Clone, Copy)]
pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

/// A parameter that `require` found present, not yet checked for type.
#[derive(Clone, Copy)]
pub struct Required<'a> {
    name: &'static str,
    value: &'a Value,
}

/// Stands in for a slot of `require`'s result until the slot is filled.
static ABSENT: Value = Value::Null;

impl<'a> Required<'a> {
    pub fn string(self) -> Result<&'a str> {
        as_string(self.name, self.value)
    }

    pub fn integer(self, range: RangeInclusive<u64>) -> Result<u64> {
        as_integer(self.name, self.value, range)
    }

    /// Like [`Params::seconds`], for a required parameter.
    pub fn seconds(self, range: RangeInclusive<u64>) -> Result<Seconds> {
        self.integer(range).map(Seconds::new)
    }

    pub fn object(self) -> Result<Params<'a>> {
        as_object(self.name, self.value)
    }
}

fn invalid_value(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must be {expected}"),
    )
}

fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| invalid_value(name, "a string"))
}

fn as_integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64> {
    let number = value
        .as_number()
        .and_then(|n| n.as_i64())
        .ok_or_else(|| invalid_value(name, "an integer"))?;
    u64::try_from(number)
        .ok()
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            invalid_value(
                name,
                &format!("between {} and {}", range.start(), range.end()),
            )
        })
}

fn as_object<'a>(name: &str, value: &'a Value) -> Result<Params<'a>> {
    value
        .as_object()
        .map(Params::new)
        .ok_or_else(|| invalid_value(name, "an object"))
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|v| !v.is_null())
    }

    /// Fails with `MissingParameter` for the first absent name.
    pub fn require<const N: usize>(&self, names: [&'static str; N]) -> Result<[Required<'a>; N]> {
        let mut found = [Required {
            name: "",
            value: &ABSENT,
        }; N];
        for (slot, name) in found.iter_mut().zip(names) {
            let value = self.get(name).ok_or_else(|| {
                Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
            })?;
            *slot = Required { name, value };
        }
        Ok(found)
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name).map(|v| as_string(name, v)).transpose()
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>> {
        self.get(name)
            .map(|v| v.as_bool().ok_or_else(|| invalid_value(name, "a boolean")))
            .transpose()
    }

    /// An integer written without fraction or exponent, within `range`.
    pub fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        self.get(name)
            .map(|v| as_integer(name, v, range))
            .transpose()
    }

    /// Like [`Params::integer`], for a number of seconds.
    pub fn seconds(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<Seconds>> {
        Ok(self.integer(name, range)?.map(Seconds::new))
    }

    /// A string of 1 to `max_len` characters from `A-Z a-z 0-9 _ -`.
    pub fn identifier(&self, name: &str, max_len: usize) -> Result<Option<&'a str>> {
        match self.string(name)? {
            Some(s) if !naming::is_identifier(s, max_len) => Err(invalid_value(
                name,
                &format!("1 to {max_len} characters from A-Z a-z 0-9 _ -"),
            )),
            other => Ok(other),
        }
    }

    pub fn object(&self, name: &str) -> Result<Option<Params<'a>>> {
        self.get(name).map(|v| as_object(name, v)).transpose()
    }

    /// Fails with `InvalidParameterValue` if a member is not in `known`.
    pub fn reject_unknown(&self, known: &[&str]) -> Result<()> {
        match self.members.keys().find(|k| !known.contains(&k.as_str())) {
            Some(key) => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("unknown attribute `{key}`"),
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_of(json: &str) -> Map<String, Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn integers_must_have_no_fraction_or_exponent() {
        let m = params_of(
            r#"{"a":30,"b":30.0,"c":3e1,"d":"30","e":-0,"f":-1,"g":99999999999999999999}"#,
        );
        let p = Params::new(&m);
        assert_eq!(p.integer("a", 0..=100).unwrap(), Some(30));
        for bad in ["b", "c", "d", "f", "g"] {
            let err = p.integer(bad, 0..=100).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParameterValue, "{bad}");
        }
        assert_eq!(p.integer("e", 0..=100).unwrap(), Some(0));
        assert_eq!(p.integer("missing", 0..=100).unwrap(), None);
    }

    #[test]
    fn null_is_absent() {
        let m = params_of(r#"{"a":null}"#);
        let p = Params::new(&m);
        assert_eq!(p.string("a").unwrap(), None);
        assert_eq!(
            p.require(["a"]).err().map(|e| e.code),
            Some(ErrorCode::MissingParameter)
        );
    }

    #[test]
    fn range_is_inclusive() {
        let m = params_of(r#"{"lo":1,"hi":10,"under":0,"over":11}"#);
        let p = Params::new(&m);
        assert_eq!(p.integer("lo", 1..=10).unwrap(), Some(1));
        assert_eq!(p.integer("hi", 1..=10).unwrap(), Some(10));
        for bad in ["under", "over"] {
            let err = p.integer(bad, 1..=10).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParameterValue, "{bad}");
        }
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let lines = [
            &b"not json"[..],
            b"[]",
            b"{}",
            b"{\"Command\":1}",
            b"{\"Command\":\"Nope\"}",
            b"\xff",
        ];
        for line in lines {
            let err = Request::parse(line).err().expect("should fail");
            assert_eq!(err.code, ErrorCode::InvalidRequest, "{line:?}");
        }
    }

    #[test]
    fn a_known_command_parses() {
        let request = Request::parse(br#"{"Command":"Tick"}"#).unwrap();
        assert_eq!(request.command, Command::Tick);
    }
}
