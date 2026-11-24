//! Queue tag commands (SPEC-feature section 3).

use super::queue_commands::queue_name;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{TagKey, TagValue};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.required_string("QueueName")?;
        let tags = params
            .required_object("Tags")?
            .members()
            .map(|(key, value)| Ok((TagKey::parse(key)?, TagValue::parse(value)?)))
            .collect::<Result<Vec<_>>>()?;

        self.queue_mut(name)?.tags.set_all(tags)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.required_string("QueueName")?;
        let keys = params
            .required_array("TagKeys")?
            .iter()
            .map(|key| {
                key.as_str()
                    .ok_or_else(|| {
                        Error::new(ErrorCode::InvalidParameterValue, "tag keys must be strings")
                    })
                    .and_then(TagKey::parse)
            })
            .collect::<Result<Vec<_>>>()?;

        self.queue_mut(name)?.tags.remove_all(&keys);
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        let queue = self.queue_mut(name)?;
        Ok(members([("Tags", queue.tags.to_json())]))
    }
}
