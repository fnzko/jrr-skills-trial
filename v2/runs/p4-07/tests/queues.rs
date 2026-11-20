//! Section 5: queue commands and the rules of their parameters.

mod support;

use serde_json::json;
use support::{refused, replies};

#[test]
fn create_queue_accepts_identical_repeats_and_refuses_other_attributes() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"x"}"#,
            r#"{"Command":"CreateQueue","QueueName":"x"}"#,
            r#"{"Command":"CreateQueue","QueueName":"x","Attributes":{"VisibilityTimeout":31}}"#,
            r#"{"Command":"CreateQueue","QueueName":"bad name"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"g.fifo"}"#,
            r#"{"Command":"CreateQueue","QueueName":"g.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t","Attributes":{"DeadLetterQueue":"x"}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t","Attributes":{"DeadLetterQueue":"g.fifo","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t","Attributes":{"DeadLetterQueue":"x","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t","Attributes":{"DeadLetterQueue":"x","MaxReceiveCount":4}}"#,
            r#"{"Command":"CreateQueue","QueueName":"t"}"#,
            r#"{"Command":"CreateQueue","QueueName":"h.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        ]),
        vec![
            json!({"ok": true}),
            json!({"ok": true}),
            refused("QueueAlreadyExists"),
            refused("QueueNameInvalid"),
            refused("QueueNameInvalid"),
            refused("QueueNameInvalid"),
            json!({"ok": true}),
            refused("InvalidParameterValue"),
            refused("InvalidParameterValue"),
            refused("QueueDoesNotExist"),
            refused("InvalidParameterValue"),
            json!({"ok": true}),
            refused("QueueAlreadyExists"),
            refused("QueueAlreadyExists"),
            json!({"ok": true}),
        ]
    );
}

#[test]
fn set_queue_attributes_applies_the_rules_of_section_5_in_order() {
    let setup = [
        r#"{"Command":"CreateQueue","QueueName":"src"}"#,
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
    ];
    let mut lines = setup.to_vec();
    lines.extend([
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"Fifo":false}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"nosuch","Attributes":{}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq"}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"src","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2,"VisibilityTimeout":45}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":null,"MaxReceiveCount":null}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
    ]);
    let got = replies(&lines);
    let attributes = json!({
        "ok": true,
        "Attributes": {
            "VisibilityTimeout": 45,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dlq",
            "MaxReceiveCount": 2
        },
        "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
    });
    assert_eq!(
        got[setup.len()..],
        [
            refused("UnsupportedOperation"),
            refused("QueueDoesNotExist"),
            refused("InvalidParameterValue"),
            refused("InvalidParameterValue"),
            refused("QueueDoesNotExist"),
            refused("InvalidParameterValue"),
            refused("InvalidParameterValue"),
            json!({"ok": true}),
            attributes.clone(),
            json!({"ok": true}),
            json!({"ok": true}),
            attributes,
        ]
    );
}

#[test]
fn get_queue_attributes_reports_defaults_for_standard_and_fifo_queues() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"h.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"h.fifo"}"#,
        ])[1],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": true,
                "ContentBasedDeduplication": true
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn list_queues_returns_names_in_byte_order_filtered_by_prefix() {
    let names = ["b", "B", "_x", "a", "-z", "0", "a-b"];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| format!(r#"{{"Command":"CreateQueue","QueueName":"{name}"}}"#))
        .collect();
    lines.push(r#"{"Command":"ListQueues"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"zz"}"#.to_owned());
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let got = replies(&line_refs);
    assert_eq!(
        got[names.len()..],
        [
            json!({"ok": true, "QueueNames": ["-z", "0", "B", "_x", "a", "a-b", "b"]}),
            json!({"ok": true, "QueueNames": ["a", "a-b"]}),
            json!({"ok": true, "QueueNames": []}),
        ]
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_references_and_a_new_queue_starts_empty() {
    let setup = [
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"SendMessage","QueueName":"dlq","Body":"kept"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq","VisibilityTimeout":100}"#,
    ];
    let mut lines = setup.to_vec();
    lines.extend([
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"dlq","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
    ]);
    let got = replies(&lines);
    assert_eq!(
        got[setup.len()..],
        [
            json!({"ok": true}),
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30,
                    "DelaySeconds": 0,
                    "RetentionSeconds": 345600,
                    "Fifo": false,
                    "ContentBasedDeduplication": false
                },
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
            }),
            refused("QueueDoesNotExist"),
            refused("QueueDoesNotExist"),
            json!({"ok": true}),
            refused("ReceiptHandleExpired"),
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30,
                    "DelaySeconds": 0,
                    "RetentionSeconds": 345600,
                    "Fifo": false,
                    "ContentBasedDeduplication": false
                },
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
            }),
        ]
    );
}

#[test]
fn purge_empties_the_queue_but_message_ids_keep_counting() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
            r#"{"Command":"PurgeQueue","QueueName":"q"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"c"}"#,
        ])[2..],
        [
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true}),
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30,
                    "DelaySeconds": 0,
                    "RetentionSeconds": 345600,
                    "Fifo": false,
                    "ContentBasedDeduplication": false
                },
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
            }),
            json!({"ok": true, "MessageId": "m-3"}),
        ]
    );
}

#[test]
fn tick_advances_the_clock_and_stops_at_the_documented_limit() {
    // 3306 full Ticks reach 3998937600, which leaves 1062400 seconds below 4000000000.
    let mut lines: Vec<String> = (0..3306)
        .map(|_| r#"{"Command":"Tick","Seconds":1209600}"#.to_owned())
        .collect();
    lines.push(r#"{"Command":"Tick","Seconds":1209600}"#.to_owned());
    lines.push(r#"{"Command":"Tick","Seconds":1062400}"#.to_owned());
    lines.push(r#"{"Command":"Tick","Seconds":1}"#.to_owned());
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let got = replies(&line_refs);
    assert_eq!(got[3305], json!({"ok": true, "Now": 3998937600u64}));
    assert_eq!(got[3306], refused("InvalidParameterValue"));
    assert_eq!(got[3307], json!({"ok": true, "Now": 4000000000u64}));
    assert_eq!(got[3308], refused("InvalidParameterValue"));
}
