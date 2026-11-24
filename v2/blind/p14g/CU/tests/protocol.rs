//! End-to-end scenarios: input lines in, response lines out.

use std::io::Cursor;

use serde_json::{Value, json};

fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::serve(Cursor::new(input), &mut output).expect("in-memory I/O does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn run(lines: &[&str]) -> Vec<Value> {
    run_raw(lines.join("\n").as_bytes())
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

#[test]
fn every_line_gets_exactly_one_response() {
    let responses = run_raw(
        b"\n{\"Command\":\"Tick\",\"Seconds\":2}\r\nnot json\n{\"Command\":\"ListQueues\"}",
    );
    assert_eq!(responses.len(), 4);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
    assert_eq!(error_code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn invalid_utf8_line_is_rejected_and_the_next_line_still_runs() {
    let responses = run_raw(b"{\"Command\":\"\xff\"}\n{\"Command\":\"Tick\",\"Seconds\":0}\n");
    assert_eq!(responses.len(), 2);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 0}));
}

#[test]
fn empty_input_produces_no_output() {
    assert!(run_raw(b"").is_empty());
}

#[test]
fn send_receive_and_delete_round_trip() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        r#"{"Command":"SendMessage","QueueName":"jobs","Body":"one"}"#,
        r#"{"Command":"Tick","Seconds":4}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs","VisibilityTimeout":10}"#,
        r#"{"Command":"DeleteMessage","QueueName":"jobs","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"jobs"}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 4}));
    assert_eq!(
        responses[3],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4
        }]})
    );
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(
        responses[5],
        json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                           "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}})
    );
}

#[test]
fn create_queue_is_idempotent_only_for_identical_attributes() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":5}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":5,"VisibilityTimeout":30}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":3}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":3}}"#,
    ]);
    let codes: Vec<_> = responses
        .iter()
        .map(|r| {
            if r["ok"] == json!(true) {
                "ok"
            } else {
                error_code(r)
            }
        })
        .collect();
    assert_eq!(
        codes,
        [
            "ok",
            "ok",
            "QueueAlreadyExists",
            "QueueNameInvalid",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn queue_lookup_precedes_command_rules() {
    let responses = run(&[
        r#"{"Command":"SendMessage","QueueName":"a b","Body":"x"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"missing","ReceiptHandle":"zzz"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"zzz"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[1]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[3]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&responses[4]), "ReceiptHandleInvalid");
}

#[test]
fn handle_from_another_queue_is_expired_not_invalid() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"a"}"#,
        r#"{"Command":"CreateQueue","QueueName":"b"}"#,
        r#"{"Command":"SendMessage","QueueName":"a","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"a"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"b","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"b","ReceiptHandle":"r-1","VisibilityTimeout":5}"#,
    ]);
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[5]), "ReceiptHandleExpired");
}

#[test]
fn failed_commands_do_not_use_up_message_numbers() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"g"}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[2], responses[1]);
    assert_eq!(error_code(&responses[3]), "InvalidParameterValue");
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn dead_letter_flow_and_queue_deletion() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":1,"VisibilityTimeout":0}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":2}}"#,
    ]);
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[5]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "x",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}])
    );
    assert!(responses[7]["Attributes"].get("DeadLetterQueue").is_none());
    assert!(responses[7]["Attributes"].get("MaxReceiveCount").is_none());
    assert_eq!(error_code(&responses[8]), "UnsupportedOperation");
    assert_eq!(error_code(&responses[9]), "InvalidParameterValue");
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a_", "aB", "a1", "a-", "a.fifo"];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            format!(
                r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{{"Fifo":{fifo}}}}}"#
            )
        })
        .collect();
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = run(&lines);
    assert_eq!(
        responses.last().unwrap(),
        &json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "aB", "a_"]})
    );
}

#[test]
fn retention_removes_messages_when_it_is_lowered() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"Tick","Seconds":100}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn filling_a_queue_to_its_limit_then_refuses_more() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    let responses = run_raw(input.as_bytes());
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
}

#[test]
fn queue_count_is_limited() {
    let mut input = String::new();
    for n in 0..=1000 {
        input.push_str(&format!(
            "{{\"Command\":\"CreateQueue\",\"QueueName\":\"q{n}\"}}\n"
        ));
    }
    input.push_str("{\"Command\":\"CreateQueue\",\"QueueName\":\"q0\"}\n");
    let responses = run_raw(input.as_bytes());
    assert_eq!(responses[999], json!({"ok": true}));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], json!({"ok": true}));
}
