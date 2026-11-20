mod common;

use common::{fail, ok, script};
use serde_json::json;

const CREATE_Q: &str = r#"{"Command":"CreateQueue","QueueName":"q"}"#;
const CREATE_F: &str =
    r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#;
const RECEIVE_Q: &str = r#"{"Command":"ReceiveMessage","QueueName":"q"}"#;

fn counts(visible: u32, in_flight: u32, delayed: u32) -> serde_json::Value {
    json!({ "Visible": visible, "InFlight": in_flight, "Delayed": delayed })
}

fn get_counts(queue: &str) -> String {
    format!(r#"{{"Command":"GetQueueAttributes","QueueName":"{queue}"}}"#)
}

#[test]
fn send_and_receive_in_order_with_counts() {
    script(&[
        (CREATE_Q, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            json!({"ok": true, "MessageId": "m-1"}),
        ),
        (
            r#"{"Command":"Tick","Seconds":4}"#,
            json!({"ok": true, "Now": 4}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
            json!({"ok": true, "MessageId": "m-2"}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4},
                {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1, "SentAt": 4, "FirstReceivedAt": 4},
            ]}),
        ),
        (RECEIVE_Q, json!({"ok": true, "Messages": []})),
    ]);
}

#[test]
fn state_counts_follow_the_clock() {
    let get = get_counts("q");
    let responses = common::run_lines(
        &[
            CREATE_Q,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DelaySeconds":10}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":20}"#,
            &get,
            r#"{"Command":"Tick","Seconds":10}"#,
            &get,
            r#"{"Command":"Tick","Seconds":10}"#,
            &get,
        ]
        .join("\n"),
    );
    let at = |i: usize| responses[i]["Counts"].clone();
    assert_eq!(at(4), counts(0, 1, 1));
    assert_eq!(at(6), counts(1, 1, 0));
    assert_eq!(at(8), counts(2, 0, 0));
}

#[test]
fn send_rules_for_standard_queues() {
    script(&[
        (CREATE_Q, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","GroupId":"g"}"#,
            fail("UnsupportedOperation"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DeduplicationId":"d"}"#,
            fail("UnsupportedOperation"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"nope","Body":"a"}"#,
            fail("QueueDoesNotExist"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            json!({"ok": true, "MessageId": "m-1"}),
        ),
    ]);
}

#[test]
fn fifo_send_rules_and_deduplication_window() {
    script(&[
        (CREATE_F, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d","DelaySeconds":0}"#,
            fail("UnsupportedOperation"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DeduplicationId":"d"}"#,
            fail("MissingParameter"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
            fail("MissingParameter"),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (
            r#"{"Command":"Tick","Seconds":299}"#,
            json!({"ok": true, "Now": 299}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"h","DeduplicationId":"d"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (
            r#"{"Command":"Tick","Seconds":1}"#,
            json!({"ok": true, "Now": 300}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"c","GroupId":"g","DeduplicationId":"d"}"#,
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ),
    ]);
}

#[test]
fn deduplication_outlives_the_message_and_content_based_uses_the_body() {
    script(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            ok(),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (r#"{"Command":"PurgeQueue","QueueName":"c.fifo"}"#, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"other","GroupId":"g"}"#,
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ),
        (
            &get_counts("c.fifo"),
            json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600, "Fifo": true, "ContentBasedDeduplication": true},
            "Counts": counts(1, 0, 0)}),
        ),
    ]);
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_behind_in_flight() {
    script(&[
        (CREATE_F, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a1","GroupId":"a","DeduplicationId":"1"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a2","GroupId":"a","DeduplicationId":"2"}"#,
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b1","GroupId":"b","DeduplicationId":"3"}"#,
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1},
                {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3},
            ]}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            json!({"ok": true, "Messages": []}),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
            ok(),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            json!({"ok": true, "Messages": [
                {"MessageId": "m-2", "ReceiptHandle": "r-3", "Body": "a2", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 2},
            ]}),
        ),
    ]);
}

#[test]
fn receipt_handles_are_exact_and_only_the_latest_works() {
    script(&[
        (CREATE_Q, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            json!({"ok": true, "MessageId": "m-1"}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            fail("ReceiptHandleExpired"),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a", "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            fail("ReceiptHandleExpired"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-02"}"#,
            fail("ReceiptHandleInvalid"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-2"}"#,
            fail("ReceiptHandleInvalid"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2 "}"#,
            fail("ReceiptHandleInvalid"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-3"}"#,
            fail("ReceiptHandleInvalid"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-0"}"#,
            fail("ReceiptHandleInvalid"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"nope","ReceiptHandle":"r-2"}"#,
            fail("QueueDoesNotExist"),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            ok(),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            fail("ReceiptHandleExpired"),
        ),
    ]);
}

#[test]
fn a_handle_of_another_queue_or_an_earlier_queue_of_the_same_name_is_expired() {
    script(&[
        (CREATE_Q, ok()),
        (r#"{"Command":"CreateQueue","QueueName":"p"}"#, ok()),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            json!({"ok": true, "MessageId": "m-1"}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"p","ReceiptHandle":"r-1"}"#,
            fail("ReceiptHandleExpired"),
        ),
        (r#"{"Command":"DeleteQueue","QueueName":"q"}"#, ok()),
        (CREATE_Q, ok()),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            fail("ReceiptHandleExpired"),
        ),
    ]);
}

#[test]
fn change_visibility_moves_the_end_of_the_timeout() {
    let get = get_counts("q");
    let responses = common::run_lines(&[
        CREATE_Q,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        RECEIVE_Q,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":100}"#,
        r#"{"Command":"Tick","Seconds":99}"#,
        &get,
        r#"{"Command":"Tick","Seconds":1}"#,
        &get,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":5}"#,
    ].join("\n"));
    assert_eq!(responses[3], ok());
    assert_eq!(responses[5]["Counts"], counts(0, 1, 0));
    assert_eq!(responses[7]["Counts"], counts(1, 0, 0));
    assert_eq!(responses[8], fail("ReceiptHandleExpired"));
}

#[test]
fn retention_removes_messages_and_lowering_then_raising_does_not_restore_them() {
    let get = get_counts("q");
    let responses = common::run_lines(&[
        CREATE_Q,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"Tick","Seconds":100}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
        &get,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        &get,
    ].join("\n"));
    assert_eq!(responses[4]["Counts"], counts(0, 0, 0));
    assert_eq!(responses[6]["Counts"], counts(0, 0, 0));
}

#[test]
fn messages_are_dead_lettered_after_max_receive_count() {
    script(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2,"VisibilityTimeout":0}}"#,
            ok(),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            json!({"ok": true, "MessageId": "m-1"}),
        ),
        (
            RECEIVE_Q,
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
        (
            RECEIVE_Q,
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a", "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
        (RECEIVE_Q, json!({"ok": true, "Messages": []})),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
        ),
    ]);
}

#[test]
fn a_dead_lettered_message_that_is_too_old_for_the_target_is_dropped() {
    let responses = common::run_lines(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":1,"VisibilityTimeout":0}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        RECEIVE_Q,
        r#"{"Command":"Tick","Seconds":60}"#,
        RECEIVE_Q,
        &get_counts("dlq"),
    ].join("\n"));
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[6]["Counts"], counts(0, 0, 0));
}

#[test]
fn dead_lettering_frees_the_group_for_later_messages() {
    script(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
            ok(),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"d.fifo","MaxReceiveCount":1,"VisibilityTimeout":0}}"#,
            ok(),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"1"}"#,
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1}]}),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"y","GroupId":"g","DeduplicationId":"2"}"#,
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "y", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 2}]}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d.fifo"}"#,
            json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "x", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1}]}),
        ),
    ]);
}

#[test]
fn receive_validates_its_parameters() {
    script(&[
        (CREATE_Q, ok()),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":0}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":11}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":43201}"#,
            fail("InvalidParameterValue"),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":null}"#,
            json!({"ok": true, "Messages": []}),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":5,"MaxMessages":0}"#,
            fail("InvalidParameterValue"),
        ),
    ]);
}

#[test]
fn a_full_queue_rejects_sends_quickly() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    let responses = common::run_lines(&input);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001], fail("LimitExceeded"));
    assert_eq!(responses[100_002], fail("LimitExceeded"));
}
