mod common;

use common::{fail, ok, script};
use serde_json::json;

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    script(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, ok()),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":30,"Fifo":false}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":31}}"#,
            fail("QueueAlreadyExists"),
        ),
    ]);
}

#[test]
fn get_attributes_reports_defaults_and_omits_unset_dead_letter() {
    script(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, ok()),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false
                },
                "Counts": { "Visible": 0, "InFlight": 0, "Delayed": 0 }
            }),
        ),
    ]);
}

#[test]
fn name_and_type_must_agree() {
    script(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
            fail("QueueNameInvalid"),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":true}}"#,
            fail("QueueNameInvalid"),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            fail("QueueDoesNotExist"),
        ),
    ]);
}

#[test]
fn dead_letter_configuration_is_checked_on_create() {
    script(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq"}}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            fail("QueueDoesNotExist"),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"dlq.fifo","Attributes":{"Fifo":true}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq.fifo","MaxReceiveCount":2}}"#,
            fail("InvalidParameterValue"),
        ),
        (r#"{"Command":"CreateQueue","QueueName":"dlq"}"#, ok()),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":3}}"#,
            fail("QueueAlreadyExists"),
        ),
    ]);
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_of_its_users() {
    script(&[
        (r#"{"Command":"CreateQueue","QueueName":"dlq"}"#, ok()),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            ok(),
        ),
        (r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#, ok()),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false
                },
                "Counts": { "Visible": 0, "InFlight": 0, "Delayed": 0 }
            }),
        ),
        (
            r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
            fail("QueueDoesNotExist"),
        ),
    ]);
}

#[test]
fn set_attributes_rules_apply_in_order() {
    script(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, ok()),
        (r#"{"Command":"CreateQueue","QueueName":"other"}"#, ok()),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
            fail("MissingParameter"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
            fail("UnsupportedOperation"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
            fail("QueueDoesNotExist"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":2}}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"other","MaxReceiveCount":2,"DelaySeconds":5}}"#,
            ok(),
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 5, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false,
                    "DeadLetterQueue": "other", "MaxReceiveCount": 2
                },
                "Counts": { "Visible": 0, "InFlight": 0, "Delayed": 0 }
            }),
        ),
    ]);
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut steps = Vec::new();
    let creates: Vec<String> = ["b", "a_", "B", "a-", "a0", "a.fifo", "A", "a"]
        .iter()
        .map(|name| match name.strip_suffix(".fifo") {
            Some(_) => format!(
                r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{{"Fifo":true}}}}"#
            ),
            None => format!(r#"{{"Command":"CreateQueue","QueueName":"{name}"}}"#),
        })
        .collect();
    for create in &creates {
        steps.push((create.as_str(), ok()));
    }
    steps.push((
        r#"{"Command":"ListQueues"}"#,
        json!({ "ok": true, "QueueNames": ["A", "B", "a", "a-", "a.fifo", "a0", "a_", "b"] }),
    ));
    steps.push((
        r#"{"Command":"ListQueues","Prefix":"a"}"#,
        json!({ "ok": true, "QueueNames": ["a", "a-", "a.fifo", "a0", "a_"] }),
    ));
    steps.push((
        r#"{"Command":"ListQueues","Prefix":"zz"}"#,
        json!({ "ok": true, "QueueNames": [] }),
    ));
    script(&steps);
}

#[test]
fn queue_count_is_limited() {
    let mut input = String::new();
    for i in 0..1001 {
        input.push_str(&format!(
            "{{\"Command\":\"CreateQueue\",\"QueueName\":\"q{i}\"}}\n"
        ));
    }
    input.push_str("{\"Command\":\"CreateQueue\",\"QueueName\":\"q0\"}\n");
    let responses = common::run_lines(&input);
    assert_eq!(responses[999], ok());
    assert_eq!(responses[1000], fail("LimitExceeded"));
    assert_eq!(responses[1001], ok());
}

#[test]
fn tick_advances_the_clock() {
    script(&[
        (
            r#"{"Command":"Tick","Seconds":5}"#,
            json!({ "ok": true, "Now": 5 }),
        ),
        (
            r#"{"Command":"Tick","Seconds":0}"#,
            json!({ "ok": true, "Now": 5 }),
        ),
        (
            r#"{"Command":"Tick","Seconds":1209600}"#,
            json!({ "ok": true, "Now": 1209605 }),
        ),
        (r#"{"Command":"Tick"}"#, fail("MissingParameter")),
    ]);
}
