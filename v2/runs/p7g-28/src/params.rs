//! Typed access to the members of a command object.
//!
//! A member whose value is `null` is treated as absent (SPEC section 1). Accessors
//! return `Ok(None)` for absent members and `InvalidParameterValue` for wrong types
//! or values out of range.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub struct Params<'a>(&'a Map<String, Value>);

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Looks up `name` in `map`, mapping `null` to absent.
pub fn member<'a>(map: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    map.get(name).filter(|v| !v.is_null())
}

pub fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| invalid(format!("{name} must be a string")))
}

pub fn as_bool(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{name} must be a boolean")))
}

/// An integer is a JSON number without fraction or exponent.
pub fn as_int(name: &str, value: &Value, range: &RangeInclusive<i64>) -> Result<i64> {
    let n = value
        .as_i64()
        .ok_or_else(|| invalid(format!("{name} must be an integer")))?;
    if range.contains(&n) {
        Ok(n)
    } else {
        Err(invalid(format!(
            "{name} must be between {} and {}",
            range.start(),
            range.end()
        )))
    }
}

impl<'a> Params<'a> {
    pub fn new(map: &'a Map<String, Value>) -> Self {
        Self(map)
    }

    /// Fails with `MissingParameter` for the first listed parameter that is absent.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|n| member(self.0, n).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("{name} is required"),
            )),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        member(self.0, name).map(|v| as_string(name, v)).transpose()
    }

    /// A string whose UTF-8 encoding has a length within `bytes`.
    pub fn string_bytes(
        &self,
        name: &str,
        bytes: RangeInclusive<usize>,
    ) -> Result<Option<&'a str>> {
        let s = self.string(name)?;
        match s {
            Some(s) if !bytes.contains(&s.len()) => Err(invalid(format!(
                "{name} must be {} to {} bytes",
                bytes.start(),
                bytes.end()
            ))),
            _ => Ok(s),
        }
    }

    /// A 1 to 128 character identifier from `A-Z a-z 0-9 _ -`.
    pub fn identifier(&self, name: &str) -> Result<Option<&'a str>> {
        let s = self.string(name)?;
        match s {
            Some(s) if !(1..=128).contains(&s.len()) || !s.bytes().all(is_name_byte) => {
                Err(invalid(format!(
                    "{name} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
                )))
            }
            _ => Ok(s),
        }
    }

    pub fn int(&self, name: &str, range: RangeInclusive<i64>) -> Result<Option<i64>> {
        member(self.0, name)
            .map(|v| as_int(name, v, &range))
            .transpose()
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>> {
        member(self.0, name)
            .map(|v| {
                v.as_object()
                    .ok_or_else(|| invalid(format!("{name} must be an object")))
            })
            .transpose()
    }
}

/// Characters allowed in queue names (before any `.fifo` suffix) and identifiers.
pub fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}
