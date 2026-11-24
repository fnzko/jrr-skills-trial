//! Queue tag commands (SPEC-feature section 3).

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{TagKey, TagValue};

fn invalid_element(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must hold only {expected}"),
    )
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let tags = params
            .object("Tags")?
            .expect("required")
            .iter()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .ok_or_else(|| invalid_element("Tags", "string values"))?;
                Ok((TagKey::parse(key)?, TagValue::parse(value)?))
            })
            .collect::<Result<Vec<_>>>()?;

        self.queue_mut(name)?.tags.set_all(tags)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = params
            .array("TagKeys")?
            .expect("required")
            .iter()
            .map(|key| {
                let key = key
                    .as_str()
                    .ok_or_else(|| invalid_element("TagKeys", "strings"))?;
                TagKey::parse(key)
            })
            .collect::<Result<Vec<_>>>()?;

        self.queue_mut(name)?.tags.remove_all(&keys);
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let queue = self.queue_mut(name)?;
        Ok(members([("Tags", queue.tags.to_json())]))
    }
}
