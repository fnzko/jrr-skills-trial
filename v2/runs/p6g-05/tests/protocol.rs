//! End-to-end tests: lines in, response lines out, through `miniq::run`.

use serde_json::{Value, json};

/// Runs the lines through a fresh process and returns the parsed responses.
fn session(lines: &[&str]) -> Vec<Value> {
    let input = lines.join("\n");
    let mut output = Vec::new();
    miniq::run(input.as_bytes(), &mut output).expect("in-memory I/O does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    text.lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn failure(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code, "message": "m"}})
}

/// Compares responses, ignoring the free-text error message.
fn assert_responses(actual: Vec<Value>, expected: Vec<Value>) {
    let normalize = |mut value: Value| {
        if let Some(error) = value.get_mut("error") {
            error["message"] = json!("m");
        }
        value
    };
    let actual: Vec<Value> = actual.into_iter().map(normalize).collect();
    assert_eq!(actual, expected);
}

fn delivery(id: u64, handle: u64, body: &str, count: u64, sent: u64, first: u64) -> Value {
    json!({"MessageId": format!("m-{id}"), "ReceiptHandle": format!("r-{handle}"), "Body": body,
           "ReceiveCount": count, "SentAt": sent, "FirstReceivedAt": first})
}

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = session(&[
        r#"{"Command":"Tick","Seconds":2}"#,
        "",
        "[1]",
        r#"{"Command":"Tick","Seconds":3}"#,
    ]);
    assert_responses(
        responses,
        vec![
            json!({"ok": true, "Now": 2}),
            failure("InvalidRequest"),
            failure("InvalidRequest"),
            json!({"ok": true, "Now": 5}),
        ],
    );
}

#[test]
fn a_final_empty_line_is_not_a_command() {
    assert_eq!(session(&[r#"{"Command":"Tick","Seconds":1}"#, ""]).len(), 1);
    assert!(session(&[]).is_empty());
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_input_carries_on() {
    let mut input = b"{\"Command\":\"Tick\",\"Seconds\":\xff}\n".to_vec();
    input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    let mut output = Vec::new();
    miniq::run(input.as_slice(), &mut output).expect("in-memory I/O does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    let responses: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn queue_lifecycle_and_attributes() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"b"}"#,
        r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"VisibilityTimeout":5,"DelaySeconds":null}}"#,
        r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"VisibilityTimeout":5}}"#,
        r#"{"Command":"CreateQueue","QueueName":"a"}"#,
        r#"{"Command":"CreateQueue","QueueName":"B.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"ListQueues"}"#,
        r#"{"Command":"ListQueues","Prefix":"a"}"#,
        r#"{"Command":"ListQueues","Prefix":"zz"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"a"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"nope"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"a"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"a"}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            ok(),
            ok(),
            failure("QueueAlreadyExists"),
            ok(),
            json!({"ok": true, "QueueNames": ["B.fifo", "a", "b"]}),
            json!({"ok": true, "QueueNames": ["a"]}),
            json!({"ok": true, "QueueNames": []}),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": 0, "RetentionSeconds": 345600,
                                  "Fifo": false, "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            failure("QueueDoesNotExist"),
            ok(),
            failure("QueueDoesNotExist"),
        ],
    );
}

#[test]
fn create_queue_error_precedence() {
    let responses = session(&[
        r#"{"Command":"CreateQueue"}"#,
        r#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":[]}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"MaxReceiveCount":2,"DeadLetterQueue":"gone"}}"#,
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"MaxReceiveCount":2,"DeadLetterQueue":"d.fifo"}}"#,
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true,"MaxReceiveCount":2}}"#,
    ]);
    assert_responses(
        responses,
        vec![
            failure("MissingParameter"),
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            ok(),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
        ],
    );
}

#[test]
fn creating_with_a_dead_letter_queue_is_idempotent_only_when_equal() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":4}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            ok(),
            ok(),
            failure("QueueAlreadyExists"),
            failure("QueueAlreadyExists"),
            ok(),
            ok(),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                                  "Fifo": false, "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn set_queue_attributes_rules() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"other"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3,"DeadLetterQueue":"gone"}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3,"DeadLetterQueue":"q"}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3,"DeadLetterQueue":"f.fifo"}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3,"DeadLetterQueue":"other","DelaySeconds":7}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":1}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"missing","Attributes":{}}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            ok(),
            ok(),
            failure("MissingParameter"),
            failure("UnsupportedOperation"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            ok(),
            ok(),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 1, "DelaySeconds": 7, "RetentionSeconds": 345600,
                                  "Fifo": false, "ContentBasedDeduplication": false,
                                  "DeadLetterQueue": "other", "MaxReceiveCount": 3},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            failure("QueueDoesNotExist"),
        ],
    );
}

#[test]
fn send_and_receive_standard_queue() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"Tick","Seconds":4}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":5}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":26}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":5}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Now": 4}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "Messages": [delivery(1, 1, "a", 1, 0, 4)]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 30}),
            json!({"ok": true, "Messages": [delivery(2, 2, "b", 1, 4, 30)]}),
        ],
    );
}

#[test]
fn send_error_precedence() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"s"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"s"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":""}"#,
        r#"{"Command":"SendMessage","QueueName":"nope","Body":"x","GroupId":"bad id"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"x","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DelaySeconds":1}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"ok"}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            ok(),
            ok(),
            failure("MissingParameter"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("UnsupportedOperation"),
            failure("UnsupportedOperation"),
            failure("UnsupportedOperation"),
            failure("MissingParameter"),
            failure("MissingParameter"),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2"}),
        ],
    );
}

#[test]
fn fifo_deduplication_window() {
    let send = r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d"}"#;
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        send,
        r#"{"Command":"Tick","Seconds":299}"#,
        send,
        r#"{"Command":"Tick","Seconds":1}"#,
        send,
        send,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 299}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ],
    );
}

#[test]
fn content_based_deduplication_survives_purge() {
    let send = r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"same","GroupId":"g"}"#;
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        send,
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#,
        send,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"other","GroupId":"h"}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ],
    );
}

#[test]
fn fifo_groups_are_delivered_one_message_at_a_time() {
    let fifo = |id: u64, handle: u64, body: &str, group: &str, seq: u64, count: u64, first: u64| {
        let mut value = delivery(id, handle, body, count, 0, first);
        value["GroupId"] = json!(group);
        value["SequenceNumber"] = json!(seq);
        value
    };
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a1","GroupId":"a","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a2","GroupId":"a","DeduplicationId":"2"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b1","GroupId":"b","DeduplicationId":"3"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
            json!({"ok": true, "Messages": [fifo(1, 1, "a1", "a", 1, 1, 0), fifo(3, 2, "b1", "b", 3, 1, 0)]}),
            json!({"ok": true, "Messages": []}),
            ok(),
            json!({"ok": true, "Messages": [fifo(2, 3, "a2", "a", 2, 1, 0)]}),
        ],
    );
}

#[test]
fn receipt_handles_are_validated_in_order() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"p"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"p","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"gone","ReceiptHandle":"junk"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":43201}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
    ]);
    assert_responses(
        responses,
        vec![
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [delivery(1, 1, "x", 1, 0, 0)]}),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleExpired"),
            failure("QueueDoesNotExist"),
            failure("MissingParameter"),
            failure("InvalidParameterValue"),
            ok(),
            failure("ReceiptHandleExpired"),
        ],
    );
}

#[test]
fn visibility_timeout_and_change_visibility() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":10}"#,
        r#"{"Command":"Tick","Seconds":9}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":10}"#,
        r#"{"Command":"Tick","Seconds":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":5}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-2","VisibilityTimeout":0}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-3","VisibilityTimeout":5}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    let counts = |v: u64, f: u64, d: u64| json!({"Visible": v, "InFlight": f, "Delayed": d});
    assert_eq!(
        responses[2],
        json!({"ok": true, "Messages": [delivery(1, 1, "x", 1, 0, 0)]})
    );
    assert_eq!(responses[4], ok());
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [delivery(1, 2, "x", 2, 0, 0)]})
    );
    assert_eq!(responses[8]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(responses[9], ok());
    assert_eq!(
        responses[10],
        json!({"ok": true, "Messages": [delivery(1, 3, "x", 3, 0, 0)]})
    );
    assert_eq!(responses[11]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(
        responses[12],
        json!({"ok": true, "Messages": [delivery(1, 4, "x", 4, 0, 0)]})
    );
    assert_eq!(responses[13]["Counts"], counts(0, 1, 0));
}

#[test]
fn retention_removes_messages_at_the_deadline_and_stays_removed() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"Tick","Seconds":59}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":600}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"y"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[3]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[6]["Counts"]["Visible"], json!(0));
    assert_eq!(responses[9]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[12]["Counts"]["Visible"], json!(0));
}

#[test]
fn purge_empties_the_queue_but_keeps_sequence_numbers() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"1"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"2"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"missing"}"#,
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(responses[4]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[6]["error"]["code"], "QueueDoesNotExist");
}

#[test]
fn dead_letter_moves_after_max_receive_count() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
    ]);
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [delivery(1, 3, "x", 1, 0, 0)]})
    );
}

#[test]
fn dead_lettered_message_that_is_too_old_is_dropped_on_arrival() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        responses[3]["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
}

#[test]
fn queue_limits() {
    let mut lines = Vec::new();
    let creates: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    lines.extend(creates.iter().map(String::as_str));
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#);
    let responses = session(&lines);
    assert_eq!(responses[999], ok());
    assert_eq!(responses[1000]["error"]["code"], "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#; 1];
    lines.extend(vec![
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#;
        100_001
    ]);
    lines.push(r#"{"Command":"PurgeQueue","QueueName":"q"}"#);
    lines.push(r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#);
    let responses = session(&lines);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001]["error"]["code"], "LimitExceeded");
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}
