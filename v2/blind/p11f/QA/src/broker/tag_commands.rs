//! Queue tag commands (SPEC-feature section 3).

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{TagKey, parse_tags};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.required_string("QueueName")?;
        let tags = parse_tags(params.required_members("Tags")?)?;
        self.queue_mut(name)?.tags.set(tags)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.required_string("QueueName")?;
        let keys = params
            .required_array("TagKeys")?
            .iter()
            .map(|key| {
                let key = key.as_str().ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidParameterValue,
                        "`TagKeys` must hold strings",
                    )
                })?;
                TagKey::parse(key)
            })
            .collect::<Result<Vec<_>>>()?;
        self.queue_mut(name)?.tags.remove(&keys);
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let queue = self.queue_mut(name)?;
        Ok(members([("Tags", queue.tags.to_json())]))
    }
}
