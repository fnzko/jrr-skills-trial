use std::ops::RangeInclusive;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{GroupId, is_token};

/// The members of a JSON object, where `null` counts as absent.
pub(crate) struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub(crate) fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    pub(crate) fn names(&self) -> impl Iterator<Item = &'a str> + use<'a, '_> {
        self.members
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, _)| name.as_str())
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    pub(crate) fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    pub(crate) fn command(&self) -> Result<&'a str, Error> {
        self.get("Command").and_then(Value::as_str).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidRequest,
                "the line has no string member Command",
            )
        })
    }

    pub(crate) fn string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| Error::invalid(name, "must be a string"))
            })
            .transpose()
    }

    pub(crate) fn required_string(&self, name: &str) -> Result<&'a str, Error> {
        self.string(name)?.ok_or_else(|| Error::missing(name))
    }

    pub(crate) fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::invalid(name, "must be a boolean"))
            })
            .transpose()
    }

    /// An integer written without fraction or exponent, within `range`.
    pub(crate) fn integer(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<u64>, Error> {
        // `as_u64` is `None` for floats and negative numbers, which are
        // invalid for every parameter in the protocol.
        self.get(name)
            .map(|value| {
                value.as_u64().filter(|n| range.contains(n)).ok_or_else(|| {
                    Error::invalid(
                        name,
                        &format!(
                            "must be an integer from {} to {}",
                            range.start(),
                            range.end()
                        ),
                    )
                })
            })
            .transpose()
    }

    /// A whole number of seconds within `range`.
    pub(crate) fn duration(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<Duration>, Error> {
        Ok(self.integer(name, range)?.map(Duration::from_secs))
    }

    pub(crate) fn required_duration(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Duration, Error> {
        self.duration(name, range)?
            .ok_or_else(|| Error::missing(name))
    }

    pub(crate) fn object(&self, name: &str) -> Result<Option<Params<'a>>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_object()
                    .map(Params::new)
                    .ok_or_else(|| Error::invalid(name, "must be an object"))
            })
            .transpose()
    }

    pub(crate) fn required_object(&self, name: &str) -> Result<Params<'a>, Error> {
        self.object(name)?.ok_or_else(|| Error::missing(name))
    }

    /// A string of 1 to 128 characters from `A-Z a-z 0-9 _ -`.
    pub(crate) fn token(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.string(name)?
            .map(|s| {
                is_token(s).then_some(s).ok_or_else(|| {
                    Error::invalid(name, "must be 1 to 128 characters of A-Z a-z 0-9 _ -")
                })
            })
            .transpose()
    }

    pub(crate) fn group_id(&self, name: &str) -> Result<Option<GroupId>, Error> {
        Ok(self
            .token(name)?
            .map(|s| GroupId::parse(s).expect("token() accepts exactly the group id syntax")))
    }
}
