//! Parsing of command lines and typed access to their parameters.
//!
//! Parameter access follows the check order of SPEC section 2: a handler
//! first calls [`Params::require`] for all required names, then reads each
//! value with a typed getter, which reports `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

/// The closed set of commands miniq understands.
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
    fn from_name(name: &str) -> Result<Self> {
        Ok(match name {
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
            other => {
                return Err(Error::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown command `{other}`"),
                ));
            }
        })
    }
}

/// A parsed command line: the command and its parameter object.
pub struct Request {
    pub command: Command,
    pub members: Map<String, Value>,
}

impl Request {
    /// Checks the request shape (SPEC section 2, check 1).
    pub fn parse(line: &[u8]) -> Result<Self> {
        let invalid = |msg: &str| Error::new(ErrorCode::InvalidRequest, msg);
        let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8"))?;
        let value: Value =
            serde_json::from_str(text).map_err(|e| invalid(&format!("invalid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(invalid("request is not a JSON object"));
        };
        let command = match members.get("Command") {
            Some(Value::String(name)) => Command::from_name(name)?,
            _ => return Err(invalid("`Command` is missing or not a string")),
        };
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

/// A parameter that is present, waiting for its typed read.
#[derive(Clone, Copy)]
pub struct Param<'a> {
    name: &'static str,
    value: &'a Value,
}

fn invalid_value(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must be {expected}"),
    )
}

impl<'a> Param<'a> {
    pub fn string(self) -> Result<&'a str> {
        self.value
            .as_str()
            .ok_or_else(|| invalid_value(self.name, "a string"))
    }

    pub fn boolean(self) -> Result<bool> {
        self.value
            .as_bool()
            .ok_or_else(|| invalid_value(self.name, "a boolean"))
    }

    /// An integer written without fraction or exponent, within `range`.
    pub fn integer(self, range: RangeInclusive<u64>) -> Result<u64> {
        let number = self
            .value
            .as_number()
            .and_then(|n| n.as_i64())
            .ok_or_else(|| invalid_value(self.name, "an integer"))?;
        u64::try_from(number)
            .ok()
            .filter(|n| range.contains(n))
            .ok_or_else(|| {
                invalid_value(
                    self.name,
                    &format!("between {} and {}", range.start(), range.end()),
                )
            })
    }

    /// A string of 1 to `max_len` characters from `A-Z a-z 0-9 _ -`.
    pub fn identifier(self, max_len: usize) -> Result<&'a str> {
        let text = self.string()?;
        if naming::is_identifier(text, max_len) {
            Ok(text)
        } else {
            Err(invalid_value(
                self.name,
                &format!("1 to {max_len} characters from A-Z a-z 0-9 _ -"),
            ))
        }
    }

    pub fn object(self) -> Result<Params<'a>> {
        self.value
            .as_object()
            .map(Params::new)
            .ok_or_else(|| invalid_value(self.name, "an object"))
    }
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &'static str) -> Option<Param<'a>> {
        self.members
            .get(name)
            .filter(|v| !v.is_null())
            .map(|value| Param { name, value })
    }

    /// Returns the named parameters in the order given, or fails with
    /// `MissingParameter` for the first absent one.
    pub fn require<const N: usize>(&self, names: [&'static str; N]) -> Result<[Param<'a>; N]> {
        static NULL: Value = Value::Null;
        let mut first_missing = None;
        let found = names.map(|name| {
            self.get(name).unwrap_or_else(|| {
                first_missing.get_or_insert(name);
                Param { name, value: &NULL }
            })
        });
        match first_missing {
            Some(name) => Err(Error::missing_parameter(name)),
            None => Ok(found),
        }
    }

    pub fn string(&self, name: &'static str) -> Result<Option<&'a str>> {
        self.get(name).map(Param::string).transpose()
    }

    pub fn boolean(&self, name: &'static str) -> Result<Option<bool>> {
        self.get(name).map(Param::boolean).transpose()
    }

    pub fn integer(&self, name: &'static str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        self.get(name).map(|p| p.integer(range)).transpose()
    }

    pub fn identifier(&self, name: &'static str, max_len: usize) -> Result<Option<&'a str>> {
        self.get(name).map(|p| p.identifier(max_len)).transpose()
    }

    pub fn object(&self, name: &'static str) -> Result<Option<Params<'a>>> {
        self.get(name).map(Param::object).transpose()
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
            p.require(["a"]).err().expect("should fail").code,
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn require_reports_the_first_absent_name() {
        let m = params_of(r#"{"a":1,"c":null}"#);
        let p = Params::new(&m);
        let err = p.require(["a", "b", "c"]).err().expect("should fail");
        assert_eq!(err, Error::missing_parameter("b"));
    }

    #[test]
    fn range_is_inclusive() {
        let m = params_of(r#"{"lo":1,"hi":10,"under":0,"over":11}"#);
        let p = Params::new(&m);
        assert!(p.integer("lo", 1..=10).is_ok());
        assert!(p.integer("hi", 1..=10).is_ok());
        assert!(p.integer("under", 1..=10).is_err());
        assert!(p.integer("over", 1..=10).is_err());
    }

    #[test]
    fn unknown_command_is_an_invalid_request() {
        let err = Request::parse(br#"{"Command":"tick"}"#)
            .err()
            .expect("should fail");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn shape_errors() {
        for line in [&b"not json"[..], b"[]", b"{}", b"{\"Command\":1}", b"\xff"] {
            let err = Request::parse(line).err().expect("should fail");
            assert_eq!(err.code, ErrorCode::InvalidRequest);
        }
        assert!(Request::parse(br#"{"Command":"Tick"}"#).is_ok());
    }
}
