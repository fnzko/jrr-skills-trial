//! End-to-end tests: command lines in, response lines out.

use std::io::Cursor;

use serde_json::{Value, json};

/// Runs `lines` through miniq and returns the parsed responses, one per line.
fn session(lines: &[&str]) -> Vec<Value> {
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory I/O does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn error_codes(responses: &[Value]) -> Vec<&str> {
    responses
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("-"))
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = session(&[
        "",
        "not json",
        r#"{"Command":"Tick","Seconds":5}"#,
        r#"{"Command":"Tick","Seconds":5,"Ignored":[1]}"#,
    ]);
    assert_eq!(
        error_codes(&responses),
        ["InvalidRequest", "InvalidRequest", "-", "-"]
    );
    assert_eq!(responses[2], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[3], json!({"ok": true, "Now": 10}));
}

#[test]
fn a_final_line_without_newline_counts() {
    let mut output = Vec::new();
    miniq::run(
        Cursor::new(r#"{"Command":"Tick","Seconds":1}"#),
        &mut output,
    )
    .unwrap();
    assert_eq!(output, b"{\"ok\":true,\"Now\":1}\n");
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_processing_continues() {
    let mut input = b"{\"Command\":\"Tick\",\"Seconds\":\xff}\n".to_vec();
    input.extend_from_slice(br#"{"Command":"Tick","Seconds":2}"#);
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let responses: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(error_codes(&responses), ["InvalidRequest", "-"]);
    assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"VisibilityTimeout":30,"Fifo":false}}"#,
        r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"VisibilityTimeout":31}}"#,
        r#"{"Command":"CreateQueue","QueueName":"a.fifo"}"#,
        r#"{"Command":"CreateQueue","QueueName":"b","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"c","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"d","Attributes":{"MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"e","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"jobs","MaxReceiveCount":3}}"#,
    ]);
    assert_eq!(
        error_codes(&responses),
        [
            "-",
            "-",
            "QueueAlreadyExists",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"bad name"}"#,
    ]);
    assert_eq!(
        responses[1],
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
        })
    );
    assert_eq!(error_codes(&responses)[2], "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut lines: Vec<String> = ["a", "B", "_x", "a-b", "a.fifo", "1"]
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            format!(
                r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{{"Fifo":{fifo}}}}}"#
            )
        })
        .collect();
    lines.push(r#"{"Command":"ListQueues"}"#.into());
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.into());
    lines.push(r#"{"Command":"ListQueues","Prefix":""}"#.into());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&refs);
    assert_eq!(
        responses[6],
        json!({"ok": true, "QueueNames": ["1", "B", "_x", "a", "a-b", "a.fifo"]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["a", "a-b", "a.fifo"]})
    );
    assert_eq!(responses[8], responses[6]);
}

#[test]
fn receiving_hides_the_message_until_its_visibility_timeout_ends() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"one"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":29}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
    ]);
    let first = json!({"MessageId":"m-1","ReceiptHandle":"r-1","Body":"one","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0});
    let second = json!({"MessageId":"m-1","ReceiptHandle":"r-2","Body":"one","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":0});
    assert_eq!(responses[2], json!({"ok": true, "Messages": [first]}));
    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7], json!({"ok": true, "Messages": [second]}));
    assert_eq!(error_codes(&responses)[8], "ReceiptHandleExpired");
    assert_eq!(responses[9], ok());
    assert_eq!(responses[10], json!({"ok": true, "Messages": []}));
}

#[test]
fn receipt_handles_are_checked_in_order_invalid_then_expired() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"other"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1 "}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"missing","ReceiptHandle":"zzz"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":100}"#,
        r#"{"Command":"DeleteMessage","QueueName":"other","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-2","VisibilityTimeout":0}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
    ]);
    assert_eq!(
        error_codes(&responses)[4..13],
        [
            "ReceiptHandleExpired",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "QueueDoesNotExist",
            "-",
            "ReceiptHandleExpired",
            "-",
        ]
    );
    assert_eq!(responses[13]["Messages"][0]["ReceiveCount"], 3);
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":10}"#,
        r#"{"Command":"Tick","Seconds":5}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":20}"#,
        r#"{"Command":"Tick","Seconds":19}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delayed_messages_become_visible_after_the_delay() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"queue delay"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"no delay","DelaySeconds":0}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DelaySeconds":900}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":10}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_and_cannot_be_undone_by_raising_it() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"Tick","Seconds":59}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1209600}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"y"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1209600}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[3]["Counts"]["Visible"], 1);
    assert_eq!(responses[6]["Counts"]["Visible"], 0);
    assert_eq!(responses[11]["Counts"]["Visible"], 0);
}

#[test]
fn set_queue_attributes_applies_the_rule_order() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"zz","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"nope","Attributes":{"Fifo":false}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Colour":null}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{}}"#,
    ]);
    assert_eq!(
        error_codes(&responses)[2..],
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "-",
        ]
    );
}

#[test]
fn deleting_a_queue_unsets_dead_letter_configurations_naming_it() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
    ]);
    assert_eq!(responses[2]["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(responses[2]["Attributes"]["MaxReceiveCount"], 2);
    assert_eq!(responses[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[4]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(
        error_codes(&responses)[5..],
        ["QueueDoesNotExist", "QueueAlreadyExists"]
    );
}

#[test]
fn messages_over_the_receive_count_move_to_the_dead_letter_queue() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":1,"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq","MaxMessages":10}"#,
        r#"{"Command":"Tick","Seconds":98}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
    ]);
    assert_eq!(responses[8], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[9]["Counts"]["Visible"], 2);
    let moved = &responses[10]["Messages"];
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["SentAt"], 0);
    assert_eq!(moved[1]["MessageId"], "m-2");
    assert_eq!(
        responses[12]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_dead_letter_move_does_not_count_toward_max_messages() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[5]["Messages"][0]["MessageId"], "m-2");
    assert_eq!(responses[5]["Messages"].as_array().map(Vec::len), Some(1));
}

#[test]
fn fifo_delivers_one_message_per_group_and_keeps_group_order() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"VisibilityTimeout":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a1","GroupId":"A","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a2","GroupId":"A","DeduplicationId":"2"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b1","GroupId":"B","DeduplicationId":"3"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [
            {"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a1","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"A","SequenceNumber":1},
            {"MessageId":"m-3","ReceiptHandle":"r-2","Body":"b1","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"B","SequenceNumber":3}
        ]})
    );
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7]["Messages"][0]["Body"], "a2");
    assert_eq!(responses[7]["Messages"].as_array().map(Vec::len), Some(1));
}

#[test]
fn fifo_send_rules_and_deduplication_window() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d","DelaySeconds":0}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"Tick","Seconds":299}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"y","GroupId":"other","DeduplicationId":"d"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"z","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g","DeduplicationId":"explicit"}"#,
    ]);
    assert_eq!(
        error_codes(&responses)[3..8],
        [
            "MissingParameter",
            "MissingParameter",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "-"
        ]
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[9], responses[7]);
    assert_eq!(
        responses[11],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[12],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 1})
    );
    assert_eq!(responses[13], responses[12]);
    assert_eq!(
        responses[14],
        json!({"ok": true, "MessageId": "m-4", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_outlives_the_message_and_purge_keeps_sequence_numbers() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"e"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"f.fifo"}"#,
    ]);
    assert_eq!(responses[3], responses[1]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[5]["Counts"]["Visible"], 1);
}

#[test]
fn a_dead_lettered_fifo_message_takes_a_new_sequence_number_and_keeps_its_group() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"VisibilityTimeout":0,"DeadLetterQueue":"d.fifo","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"G","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"b","GroupId":"G","DeduplicationId":"2"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"d.fifo"}"#,
    ]);
    assert_eq!(responses[5]["Messages"][0]["Body"], "b");
    assert_eq!(responses[6]["Messages"][0]["MessageId"], "m-1");
    assert_eq!(responses[6]["Messages"][0]["GroupId"], "G");
    assert_eq!(responses[6]["Messages"][0]["SequenceNumber"], 1);
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], 1);
}

#[test]
fn validation_order_is_shape_required_values_lookup_rules() {
    let responses = session(&[
        r#"{"Command":"SendMessage","Body":5}"#,
        r#"{"Command":"SendMessage","QueueName":"nope","Body":5}"#,
        r#"{"Command":"SendMessage","QueueName":"nope","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"nope","MaxMessages":11}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"g","DelaySeconds":1000}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":7}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1.0}"#,
        r#"{"Command":"ListQueues","Prefix":1}"#,
    ]);
    assert_eq!(
        error_codes(&responses),
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "-",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut lines: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#.into());
    lines.push(r#"{"Command":"DeleteQueue","QueueName":"q5"}"#.into());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"again"}"#.into());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&refs);
    assert_eq!(
        error_codes(&responses)[999..],
        ["-", "LimitExceeded", "-", "-", "-"]
    );
}

#[test]
fn a_queue_holds_at_most_one_hundred_thousand_messages_and_fills_quickly() {
    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#.to_owned()];
    lines.extend(
        (0..100_001)
            .map(|n| format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{n}"}}"#)),
    );
    lines.push(r#"{"Command":"PurgeQueue","QueueName":"q"}"#.into());
    lines.push(r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#.into());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&refs);
    assert_eq!(error_codes(&responses)[100_000], "-");
    assert_eq!(error_codes(&responses)[100_001], "LimitExceeded");
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn the_binary_answers_each_line_and_exits_zero() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut reply = String::new();

    // The reply must arrive while stdin is still open: responses are flushed per line.
    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n")
        .unwrap();
    stdout.read_line(&mut reply).unwrap();
    assert_eq!(reply, "{\"ok\":true,\"Now\":3}\n");

    drop(stdin);
    assert_eq!(stdout.fill_buf().unwrap(), b"");
    assert!(child.wait().unwrap().success());
}
