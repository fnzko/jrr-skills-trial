//! Parsing of command lines and typed access to their parameters.
//!
//! Parameter access follows the check order of SPEC section 2: a handler
//! first calls [`Params::require`] for all required names, then reads each
//! value with a typed getter, which reports `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

/// A parsed command line: the command name and its parameter object.
pub struct Request {
    pub command: String,
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
            Some(Value::String(name)) => name.clone(),
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

fn invalid_value(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must be {expected}"),
    )
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|v| !v.is_null())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Fails with `MissingParameter` for the first absent name.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| !self.has(name)) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("`{name}` is required"),
            )),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name)
            .map(|v| v.as_str().ok_or_else(|| invalid_value(name, "a string")))
            .transpose()
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>> {
        self.get(name)
            .map(|v| v.as_bool().ok_or_else(|| invalid_value(name, "a boolean")))
            .transpose()
    }

    /// An integer written without fraction or exponent, within `range`.
    pub fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let number = value
            .as_number()
            .and_then(|n| n.as_i64())
            .ok_or_else(|| invalid_value(name, "an integer"))?;
        u64::try_from(number)
            .ok()
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| {
                invalid_value(
                    name,
                    &format!("between {} and {}", range.start(), range.end()),
                )
            })
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
        self.get(name)
            .map(|v| {
                v.as_object()
                    .map(Params::new)
                    .ok_or_else(|| invalid_value(name, "an object"))
            })
            .transpose()
    }

    pub fn array(&self, name: &str) -> Result<Option<&'a [Value]>> {
        self.get(name)
            .map(|v| {
                v.as_array()
                    .map(Vec::as_slice)
                    .ok_or_else(|| invalid_value(name, "an array"))
            })
            .transpose()
    }

    /// The raw members, `null` ones included.
    pub fn members(&self) -> &'a Map<String, Value> {
        self.members
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
            p.require(&["a"]).unwrap_err().code,
            ErrorCode::MissingParameter
        );
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
    fn shape_errors() {
        for line in [&b"not json"[..], b"[]", b"{}", b"{\"Command\":1}", b"\xff"] {
            let err = Request::parse(line).err().expect("should fail");
            assert_eq!(err.code, ErrorCode::InvalidRequest);
        }
        assert!(Request::parse(br#"{"Command":"Tick"}"#).is_ok());
    }
}
