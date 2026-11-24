use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{Identifier, Seconds};

/// The members of a command object. A member whose value is `null` counts as
/// absent.
#[derive(Clone, Copy)]
pub struct Members<'a>(&'a Map<String, Value>);

impl<'a> Members<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn optional(&self, name: &'static str) -> Option<Param<'a>> {
        self.0
            .get(name)
            .filter(|value| !value.is_null())
            .map(|value| Param { name, value })
    }

    pub fn required(&self, name: &'static str) -> Result<Param<'a>, Error> {
        self.optional(name).ok_or_else(|| Error::missing(name))
    }
}

/// One present parameter or attribute, waiting for its type check.
#[derive(Clone, Copy)]
pub struct Param<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> Param<'a> {
    pub fn new(name: &'a str, value: &'a Value) -> Self {
        Self { name, value }
    }

    fn invalid(&self, expected: &str) -> Error {
        Error::new(
            ErrorCode::InvalidParameterValue,
            format!("{} must be {expected}", self.name),
        )
    }

    pub fn string(self) -> Result<&'a str, Error> {
        self.value.as_str().ok_or_else(|| self.invalid("a string"))
    }

    pub fn boolean(self) -> Result<bool, Error> {
        self.value
            .as_bool()
            .ok_or_else(|| self.invalid("a boolean"))
    }

    pub fn object(self) -> Result<&'a Map<String, Value>, Error> {
        self.value
            .as_object()
            .ok_or_else(|| self.invalid("an object"))
    }

    /// A string whose UTF-8 encoding is `bytes` long.
    pub fn text(self, bytes: RangeInclusive<usize>) -> Result<&'a str, Error> {
        self.string().and_then(|text| {
            if bytes.contains(&text.len()) {
                Ok(text)
            } else {
                Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    format!(
                        "{} must be {} to {} bytes long",
                        self.name,
                        bytes.start(),
                        bytes.end()
                    ),
                ))
            }
        })
    }

    pub fn identifier(self) -> Result<Identifier<'a>, Error> {
        self.string().and_then(|text| {
            Identifier::parse(text)
                .ok_or_else(|| self.invalid("1 to 128 characters from A-Z a-z 0-9 _ -"))
        })
    }

    /// An integer written without fraction or exponent, inside `range`.
    pub fn bounded<T: TryFrom<i64>>(self, range: RangeInclusive<i64>) -> Result<T, Error> {
        self.value
            .as_i64()
            .filter(|n| range.contains(n))
            .and_then(|n| T::try_from(n).ok())
            .ok_or_else(|| {
                self.invalid(&format!(
                    "an integer from {} to {}",
                    range.start(),
                    range.end()
                ))
            })
    }

    pub fn seconds(self, range: RangeInclusive<i64>) -> Result<Seconds, Error> {
        self.bounded::<u32>(range).map(Seconds)
    }
}
