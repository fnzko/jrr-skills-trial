//! Typed access to the members of a command, following the validation
//! order of the spec: required members first, then the value of each.

use serde_json::{Map, Value};

use crate::error::ApiError;
use crate::types::Seconds;

/// The members of one JSON object. A member whose value is `null` counts as
/// absent everywhere.
#[derive(Clone, Copy)]
pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    /// Names of the members that are present.
    pub fn names(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        let members = self.members;
        members
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, _)| name.as_str())
    }

    /// Fails with `MissingParameter` for the first name that is absent.
    pub fn require(&self, names: &[&str]) -> Result<(), ApiError> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(ApiError::missing(name)),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>, ApiError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(ApiError::invalid_value(name, "a string")),
        }
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str, ApiError> {
        self.string(name)?.ok_or_else(|| ApiError::missing(name))
    }

    /// A string accepted by `parse`, which returns `None` for a bad value.
    pub fn parsed_string<T>(
        &self,
        name: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>, ApiError> {
        self.string(name)?
            .map(|text| {
                parse(text).ok_or_else(|| ApiError::invalid_value(name, "in the allowed format"))
            })
            .transpose()
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>, ApiError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(ApiError::invalid_value(name, "a boolean")),
        }
    }

    /// An integer written without fraction or exponent, within `min..=max`.
    pub fn integer(&self, name: &str, min: u32, max: u32) -> Result<Option<u32>, ApiError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        value
            .as_i64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| (min..=max).contains(number))
            .map(Some)
            .ok_or_else(|| {
                ApiError::invalid_value(name, &format!("an integer from {min} to {max}"))
            })
    }

    pub fn required_integer(&self, name: &str, min: u32, max: u32) -> Result<u32, ApiError> {
        self.integer(name, min, max)?
            .ok_or_else(|| ApiError::missing(name))
    }

    pub fn seconds(&self, name: &str, min: u32, max: u32) -> Result<Option<Seconds>, ApiError> {
        Ok(self.integer(name, min, max)?.map(Seconds))
    }

    pub fn required_seconds(&self, name: &str, min: u32, max: u32) -> Result<Seconds, ApiError> {
        self.seconds(name, min, max)?
            .ok_or_else(|| ApiError::missing(name))
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>, ApiError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(members)),
            Some(_) => Err(ApiError::invalid_value(name, "an object")),
        }
    }
}
