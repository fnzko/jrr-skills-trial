use serde_json::{Value, json};

use super::{Fields, Outcome, Service, fields};
use crate::error::{Code, Failure};
use crate::params::{Params, is_token};

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid_key(key: &str) -> Failure {
    Failure::invalid_value(format!("{key:?} is not a tag key"))
}

impl Service {
    pub(super) fn tag_queue(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "Tags"])?;
        let name = params.required_string("QueueName")?;
        let Some(Value::Object(tags)) = params.get("Tags") else {
            return Err(Failure::invalid_value("Tags must be an object"));
        };
        let mut new_tags = Vec::new();
        for (key, value) in tags {
            if !is_token(key, MAX_KEY_CHARS) {
                return Err(invalid_key(key));
            }
            match value.as_str() {
                Some(value) if value.len() <= MAX_VALUE_BYTES => new_tags.push((key, value)),
                _ => {
                    return Err(Failure::invalid_value(format!(
                        "the value of tag {key} must be a string of at most {MAX_VALUE_BYTES} bytes"
                    )));
                }
            }
        }
        let queue = self.queue_mut(name)?;
        let added = new_tags
            .iter()
            .filter(|(key, _)| !queue.tags.contains_key(*key))
            .count();
        if queue.tags.len() + added > MAX_TAGS {
            return Err(Failure::new(Code::LimitExceeded, "too many tags"));
        }
        for (key, value) in new_tags {
            queue.tags.insert(key.clone(), value.to_owned());
        }
        Ok(Fields::new())
    }

    pub(super) fn untag_queue(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.required_string("QueueName")?;
        let keys = params
            .required_array("TagKeys")?
            .iter()
            .map(|key| match key.as_str() {
                Some(key) if is_token(key, MAX_KEY_CHARS) => Ok(key),
                _ => Err(Failure::invalid_value("TagKeys must hold tag keys")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let queue = self.queue_mut(name)?;
        for key in keys {
            queue.tags.remove(key);
        }
        Ok(Fields::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName"])?;
        let queue = self.queue(params.required_string("QueueName")?)?;
        Ok(fields([("Tags", json!(queue.tags))]))
    }
}
