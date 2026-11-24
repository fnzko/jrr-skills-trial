//! Queue tag commands (feature spec section 3).

use serde_json::{Map, Value};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{self, TagKey};

const MAX_TAGS: usize = 50;

fn not_a_string(what: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{what} must be a string"),
    )
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.require_string("QueueName")?;
        let given = params.require_object("Tags")?;
        let mut new_tags = Vec::new();
        for (key, value) in given.members() {
            let key = TagKey::parse(key)?;
            let value = value
                .as_str()
                .ok_or_else(|| not_a_string(&format!("value of tag `{}`", key.as_str())))?;
            tags::check_value(key.as_str(), value)?;
            new_tags.push((key, value));
        }

        let queue = self.queue_mut(name)?;
        let added = new_tags
            .iter()
            .filter(|(key, _)| !queue.tags.contains_key(key))
            .count();
        if queue.tags.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        queue.tags.extend(
            new_tags
                .into_iter()
                .map(|(key, value)| (key, value.to_owned())),
        );
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.require_string("QueueName")?;
        let keys = params
            .require_array("TagKeys")?
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| not_a_string("a tag key"))
                    .and_then(TagKey::parse)
            })
            .collect::<Result<Vec<_>>>()?;

        let queue = self.queue_mut(name)?;
        for key in &keys {
            queue.tags.remove(key);
        }
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.require_string("QueueName")?;
        let tags: Map<String, Value> = self
            .queue_mut(name)?
            .tags
            .iter()
            .map(|(key, value)| (key.as_str().to_owned(), Value::String(value.clone())))
            .collect();
        Ok(members([("Tags", Value::Object(tags))]))
    }
}
