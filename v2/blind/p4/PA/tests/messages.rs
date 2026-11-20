//! Section 6 on a standard queue, and the retention and visibility rules of section 4.

mod support;

use serde_json::json;
use support::{refused, replies};

#[test]
fn a_message_is_hidden_while_in_flight_and_returns_with_a_new_handle() {
    let got = replies(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        r#"{"Command":"SendMessage","QueueName":"jobs","Body":"a"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs"}"#,
        r#"{"Command":"Tick","Seconds":29}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"jobs","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"jobs","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"jobs"}"#,
    ]);
    assert_eq!(
        got,
        vec![
            json!({"ok": true}),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 29}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 30}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a",
                "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            refused("ReceiptHandleExpired"),
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
        ]
    );
}

#[test]
fn a_bad_or_expired_receipt_handle_is_refused_after_the_queue_lookup() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"other"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-1"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"nosuch","ReceiptHandle":"r-2"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"other","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":0}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        ])[2..],
        [
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            refused("ReceiptHandleInvalid"),
            refused("ReceiptHandleInvalid"),
            refused("ReceiptHandleInvalid"),
            refused("QueueDoesNotExist"),
            refused("ReceiptHandleExpired"),
            json!({"ok": true}),
            refused("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn change_message_visibility_moves_the_end_of_the_timeout_and_zero_makes_it_visible() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":20}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":100}"#,
            r#"{"Command":"Tick","Seconds":99}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":0}"#,
        ])[4..],
        [
            json!({"ok": true}),
            json!({"ok": true, "Now": 119}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 120}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a",
                "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            refused("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn a_delayed_message_is_invisible_until_its_delay_ends_and_a_send_can_override_the_delay() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":9}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":0}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":5}"#,
        ])[1..],
        [
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 10, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 1}}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 9}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 10}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 10
            }]}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b",
                "ReceiveCount": 1, "SentAt": 10, "FirstReceivedAt": 10
            }]}),
        ]
    );
}

#[test]
fn receive_returns_at_most_max_messages_in_arrival_order() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"c"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":2}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        ])[4..],
        [
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0},
                {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}
            ]}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-3", "ReceiptHandle": "r-3", "Body": "c", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}
            ]}),
        ]
    );
}

#[test]
fn a_message_that_reached_max_receive_count_moves_to_the_dead_letter_queue() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
            r#"{"Command":"CreateQueue","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            r#"{"Command":"SendMessage","QueueName":"src","Body":"poison"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src","VisibilityTimeout":0}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src","VisibilityTimeout":0}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src","VisibilityTimeout":0}"#,
            r#"{"Command":"Tick","Seconds":5}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"src","ReceiptHandle":"r-2"}"#,
        ])[2..],
        [
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "poison",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "poison",
                "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 5}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "poison",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 5
            }]}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dlq", "MaxReceiveCount": 2
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            refused("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn lowering_retention_removes_messages_at_once_and_raising_it_brings_nothing_back() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
            r#"{"Command":"Tick","Seconds":100}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        ])[4..],
        [
            json!({"ok": true}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 100,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            json!({"ok": true}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 1000,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ]
    );
}

#[test]
fn a_handle_of_an_expired_message_is_expired_and_the_message_is_not_counted() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":59}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        ])[4..],
        [
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}}),
            json!({"ok": true, "Now": 60}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            refused("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn a_refused_send_does_not_use_up_a_message_id() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"nosuch","Body":"a"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        ])[1..],
        [
            refused("UnsupportedOperation"),
            refused("UnsupportedOperation"),
            refused("InvalidParameterValue"),
            refused("QueueDoesNotExist"),
            json!({"ok": true, "MessageId": "m-1"}),
        ]
    );
}

#[test]
fn a_dead_lettered_message_already_past_the_dead_letter_retention_is_dropped_on_arrival() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
            r#"{"Command":"CreateQueue","QueueName":"src","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
            r#"{"Command":"SendMessage","QueueName":"src","Body":"old"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            r#"{"Command":"Tick","Seconds":100}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
        ])[2..],
        [
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "old",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            json!({"ok": true, "Now": 100}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
                "Fifo": false, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            json!({"ok": true, "Messages": []}),
        ]
    );
}
