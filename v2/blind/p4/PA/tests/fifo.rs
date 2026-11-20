//! Section 6 on FIFO queues: deduplication, sequence numbers, groups and their dead-letter moves.

mod support;

use serde_json::json;
use support::{refused, replies};

#[test]
fn a_duplicate_within_the_window_returns_the_original_and_the_window_is_not_extended() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"Tick","Seconds":299}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"c","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"c","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"f.fifo"}"#,
        ])[1..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 299}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": true, "ContentBasedDeduplication": false
            }, "Counts": {"Visible": 2, "InFlight": 0, "Delayed": 0}}),
        ]
    );
}

#[test]
fn a_fifo_send_without_group_or_deduplication_id_or_with_a_delay_is_refused() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d","DelaySeconds":5}"#,
            r#"{"Command":"SendMessage","QueueName":"nosuch","Body":"a","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
        ])[1..],
        [
            refused("MissingParameter"),
            refused("MissingParameter"),
            refused("UnsupportedOperation"),
            refused("QueueDoesNotExist"),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ]
    );
}

#[test]
fn group_and_deduplication_ids_are_refused_on_a_standard_queue() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DeduplicationId":"d"}"#,
        ])[1..],
        [
            refused("UnsupportedOperation"),
            refused("UnsupportedOperation")
        ]
    );
}

#[test]
fn content_based_deduplication_uses_the_body_and_shares_its_namespace_with_explicit_ids() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"h.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"h.fifo","Body":"same","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"h.fifo","Body":"same","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"h.fifo","Body":"zzz","GroupId":"g","DeduplicationId":"same"}"#,
            r#"{"Command":"SendMessage","QueueName":"h.fifo","Body":"other","GroupId":"g"}"#,
        ])[1..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ]
    );
}

#[test]
fn one_receive_delivers_at_most_one_message_per_group_and_a_group_waits_for_its_earliest_message() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a1","GroupId":"g1","DeduplicationId":"d1"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b1","GroupId":"g2","DeduplicationId":"d2"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a2","GroupId":"g1","DeduplicationId":"d3"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b2","GroupId":"g2","DeduplicationId":"d4"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            r#"{"Command":"Tick","Seconds":30}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        ])[5..],
        [
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
                {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 2}
            ]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 30}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a1", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
                {"MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "b1", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 2}
            ]}),
        ]
    );
}

#[test]
fn a_message_moved_to_a_fifo_dead_letter_queue_does_not_hold_back_its_group() {
    assert_eq!(
        replies(&[
            r#"{"Command":"CreateQueue","QueueName":"dlq.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"src.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"dlq.fifo","MaxReceiveCount":1}}"#,
            r#"{"Command":"SendMessage","QueueName":"src.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"SendMessage","QueueName":"src.fifo","Body":"b","GroupId":"g","DeduplicationId":"d2"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src.fifo"}"#,
            r#"{"Command":"Tick","Seconds":30}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"src.fifo"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"dlq.fifo"}"#,
        ])[4..],
        [
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1
            }]}),
            json!({"ok": true, "Now": 30}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 30, "GroupId": "g", "SequenceNumber": 2
            }]}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 30, "GroupId": "g", "SequenceNumber": 1
            }]}),
        ]
    );
}
