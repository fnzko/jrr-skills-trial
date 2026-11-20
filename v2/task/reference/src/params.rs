use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::Failure;

#[derive(Clone, Copy)]
pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    pub fn present(&self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        self.members
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, value)| (name.as_str(), value))
    }

    pub fn require(&self, names: &[&str]) -> Result<(), Failure> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Failure::missing(name)),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>, Failure> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(Failure::invalid_value(format!("{name} must be a string"))),
        }
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str, Failure> {
        self.string(name)?.ok_or_else(|| Failure::missing(name))
    }

    pub fn token(&self, name: &str, max_len: usize) -> Result<Option<&'a str>, Failure> {
        match self.string(name)? {
            Some(text) if !is_token(text, max_len) => Err(Failure::invalid_value(format!(
                "{name} must be 1 to {max_len} characters from A-Z a-z 0-9 _ -"
            ))),
            other => Ok(other),
        }
    }

    pub fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>, Failure> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        match value.as_u64().filter(|number| range.contains(number)) {
            Some(number) => Ok(Some(number)),
            None => Err(Failure::invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))),
        }
    }

    pub fn required_integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<u64, Failure> {
        self.integer(name, range)?
            .ok_or_else(|| Failure::missing(name))
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>, Failure> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(Failure::invalid_value(format!("{name} must be a boolean"))),
        }
    }

    pub fn object(&self, name: &str) -> Result<Option<Params<'a>>, Failure> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(Params::new(members))),
            Some(_) => Err(Failure::invalid_value(format!("{name} must be an object"))),
        }
    }

    pub fn required_object(&self, name: &str) -> Result<Params<'a>, Failure> {
        self.object(name)?.ok_or_else(|| Failure::missing(name))
    }

    #[cfg(feature = "extension")]
    pub fn required_array(&self, name: &str) -> Result<&'a [Value], Failure> {
        match self.get(name) {
            None => Err(Failure::missing(name)),
            Some(Value::Array(items)) => Ok(items),
            Some(_) => Err(Failure::invalid_value(format!("{name} must be an array"))),
        }
    }
}

pub fn is_token(text: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
