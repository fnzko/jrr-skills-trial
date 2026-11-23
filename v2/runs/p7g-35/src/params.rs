//! Typed access to the members of a JSON object (SPEC section 1).
//!
//! The checks mirror the order of SPEC section 2: callers first make sure all
//! required members are present, then read each member with the accessor for
//! its type. A member whose value is `null` is treated as absent everywhere.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    /// The member's value, or `None` if absent or `null`.
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.members.get(key).filter(|v| !v.is_null())
    }

    fn is_present(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Fails with `MissingParameter` unless every key is present. Callers
    /// run this before reading any value, as the check order demands.
    pub fn require_all(&self, keys: &[&str]) -> Result<()> {
        keys.iter().try_for_each(|key| {
            if self.is_present(key) {
                Ok(())
            } else {
                Err(missing(key))
            }
        })
    }

    pub fn required(&self, key: &str) -> Result<&'a Value> {
        self.get(key).ok_or_else(|| missing(key))
    }

    pub fn required_string(&self, key: &str) -> Result<&'a str> {
        self.string(key)?.ok_or_else(|| missing(key))
    }

    pub fn required_integer(&self, key: &str, min: u64, max: u64) -> Result<u64> {
        self.integer(key, min, max)?.ok_or_else(|| missing(key))
    }

    pub fn string(&self, key: &str) -> Result<Option<&'a str>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(invalid(key, "must be a string")),
        }
    }

    pub fn boolean(&self, key: &str) -> Result<Option<bool>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(invalid(key, "must be a boolean")),
        }
    }

    /// An integer member within `min..=max`.
    pub fn integer(&self, key: &str, min: u64, max: u64) -> Result<Option<u64>> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        // With `arbitrary_precision`, `as_i64` only accepts a plain integer
        // literal, so `30.0` and `3e1` are rejected here.
        let number = match value {
            Value::Number(n) => n.as_i64(),
            _ => None,
        };
        match number {
            Some(n) if n >= 0 && (min..=max).contains(&(n as u64)) => Ok(Some(n as u64)),
            Some(_) => Err(invalid(key, format!("must be between {min} and {max}"))),
            // Integers beyond i64 are out of range for every parameter.
            None if is_integer_literal(value) => {
                Err(invalid(key, format!("must be between {min} and {max}")))
            }
            None => Err(invalid(key, "must be an integer")),
        }
    }

    /// Like [`Params::string`] but with a length limit in bytes and a
    /// minimum of one character.
    pub fn required_string_with_len(&self, key: &str, max_len: usize) -> Result<&'a str> {
        self.string_with_len(key, max_len)?
            .ok_or_else(|| missing(key))
    }

    fn string_with_len(&self, key: &str, max_len: usize) -> Result<Option<&'a str>> {
        match self.string(key)? {
            Some(s) if s.is_empty() || s.len() > max_len => {
                Err(invalid(key, format!("must be 1 to {max_len} bytes long")))
            }
            other => Ok(other),
        }
    }
}

fn is_integer_literal(value: &Value) -> bool {
    match value {
        Value::Number(n) => {
            let text = n.to_string();
            let digits = text.strip_prefix('-').unwrap_or(&text);
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

pub fn missing(key: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("{key} is required"))
}

pub fn invalid(key: &str, what: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, format!("{key} {what}"))
}
