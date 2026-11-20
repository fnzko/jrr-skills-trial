//! End-to-end tests: feed command lines in, check the response lines out.

use std::io::Cursor;

use serde_json::{Value, json};

/// Runs `lines` through a fresh miniq and returns one response per line.
fn session(lines: &[&str]) -> Vec<Value> {
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn ok(response: &Value) -> &Value {
    assert_eq!(response["ok"], true, "{response}");
    response
}

#[test]
fn every_line_gets_one_response_and_bad_lines_do_not_stop_the_session() {
    let responses = session(&["", "not json", "[1]", r#"{"Command":"Tick","Seconds":5}"#]);
    assert_eq!(responses.len(), 4);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(error_code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 5}));
}

#[test]
fn successful_commands_carry_exactly_their_result_members() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        r#"{"Command":"SendMessage","QueueName":"jobs","Body":"hello"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"jobs"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"jobs"}"#,
        r#"{"Command":"ListQueues"}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[2],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0,
        }]})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 0},
        })
    );
    assert_eq!(responses[4], json!({"ok": true, "QueueNames": ["jobs"]}));
}

#[test]
fn create_queue_is_idempotent_only_for_identical_attributes() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":5}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":5,"VisibilityTimeout":30}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"f2","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"bad name"}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"MaxReceiveCount":2}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s","Attributes":{"MaxReceiveCount":2,"DeadLetterQueue":"nope"}}"#,
    ]);
    let codes: Vec<&str> = responses
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "ok",
            "ok",
            "QueueAlreadyExists",
            "QueueNameInvalid",
            "ok",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn error_precedence_follows_the_spec() {
    let responses = session(&[
        // missing parameter beats invalid value
        r#"{"Command":"SendMessage","QueueName":5}"#,
        // invalid value beats unknown queue
        r#"{"Command":"SendMessage","QueueName":"nope","Body":""}"#,
        // unknown queue beats a rule of the command (and names need no syntax)
        r#"{"Command":"GetQueueAttributes","QueueName":"not a valid name"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"nope","ReceiptHandle":"junk","VisibilityTimeout":1}"#,
        // null is absent
        r#"{"Command":"CreateQueue","QueueName":null}"#,
    ]);
    let codes: Vec<&str> = responses.iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "MissingParameter",
        ]
    );
}

#[test]
fn visibility_timeout_hides_a_message_and_expires_its_handle() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":9}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-02"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-3"}"#,
    ]);
    assert_eq!(responses[3]["Messages"], json!([]));
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(error_code(&responses[7]), "ReceiptHandleExpired");
    let second = &responses[8]["Messages"][0];
    assert_eq!(second["ReceiptHandle"], "r-2");
    assert_eq!(second["ReceiveCount"], 2);
    assert_eq!(second["FirstReceivedAt"], 0);
    assert_eq!(error_code(&responses[9]), "ReceiptHandleExpired");
    ok(&responses[10]);
    assert_eq!(error_code(&responses[11]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&responses[12]), "ReceiptHandleInvalid");
}

#[test]
fn handles_of_other_queues_are_expired_not_invalid() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"a"}"#,
        r#"{"Command":"CreateQueue","QueueName":"b"}"#,
        r#"{"Command":"SendMessage","QueueName":"a","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"a"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"b","ReceiptHandle":"r-1"}"#,
    ]);
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":5}"#,
        r#"{"Command":"Tick","Seconds":4}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":10}"#,
        r#"{"Command":"Tick","Seconds":9}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-2","VisibilityTimeout":0}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    ok(&responses[4]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[9]["Messages"][0]["ReceiveCount"], 2);
    ok(&responses[10]);
    assert_eq!(
        responses[11]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delay_hides_a_message_until_it_ends() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"queue delay"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"own delay","DelaySeconds":0}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":10}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
    ]);
    assert_eq!(
        responses[3]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    let bodies: Vec<&str> = responses[6]["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect();
    // Arrival order, not the order in which the messages became visible.
    assert_eq!(bodies, ["queue delay", "own delay"]);
}

#[test]
fn retention_removes_messages_at_once_and_for_good() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"Tick","Seconds":99}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
        r#"{"Command":"Tick","Seconds":50}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[3]["Counts"]["Visible"], 1);
    assert_eq!(responses[5]["Counts"]["Visible"], 0);
    // "b" is 50 seconds old, younger than the lowered retention of 60.
    assert_eq!(responses[10]["Counts"]["Visible"], 1);

    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_everything_but_keeps_sequence_numbers_and_deduplication() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"q.fifo"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"b","GroupId":"g","DeduplicationId":"e"}"#,
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn fifo_send_rules() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"s"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DelaySeconds":1,"GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DeduplicationId":"d"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"s","Body":"a","DeduplicationId":"d"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"s","Attributes":{"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"s","Attributes":{"Fifo":false}}"#,
    ]);
    assert_eq!(error_code(&responses[3]), "UnsupportedOperation");
    assert_eq!(error_code(&responses[4]), "MissingParameter"); // GroupId
    assert_eq!(error_code(&responses[5]), "MissingParameter"); // DeduplicationId
    // content-based deduplication makes DeduplicationId optional
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(error_code(&responses[7]), "UnsupportedOperation");
    assert_eq!(error_code(&responses[8]), "UnsupportedOperation");
    assert_eq!(error_code(&responses[9]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[10]), "UnsupportedOperation");
}

#[test]
fn fifo_deduplication_window_outlives_the_message() {
    let send = r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#;
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#,
        send,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q.fifo","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"Tick","Seconds":299}"#,
        send, // still a duplicate, and does not extend the window
        r#"{"Command":"Tick","Seconds":1}"#,
        send, // t = 300: accepted, opens a new window
        r#"{"Command":"Tick","Seconds":299}"#,
        send, // t = 599 < 600: duplicate of the second
        r#"{"Command":"Tick","Seconds":1}"#,
        send,
    ]);
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    let second = json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2});
    let third = json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3});
    assert_eq!(responses[1], first);
    assert_eq!(responses[5], first);
    assert_eq!(responses[7], second);
    assert_eq!(responses[9], second);
    assert_eq!(responses[11], third);
}

#[test]
fn content_based_deduplication_uses_the_body_and_ignores_the_group() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"same","GroupId":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"same","GroupId":"b"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"other","GroupId":"a","DeduplicationId":"same"}"#,
    ]);
    assert_eq!(responses[2], responses[1]);
    // An explicit id shares the namespace with bodies.
    assert_eq!(responses[3], responses[1]);
}

#[test]
fn fifo_receive_delivers_one_message_per_group_in_order() {
    let mut lines = vec![
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#.to_owned(),
    ];
    for (body, group) in [
        ("a1", "a"),
        ("a2", "a"),
        ("b1", "b"),
        ("a3", "a"),
        ("c1", "c"),
    ] {
        lines.push(format!(
            r#"{{"Command":"SendMessage","QueueName":"q.fifo","Body":"{body}","GroupId":"{group}","DeduplicationId":"{body}"}}"#
        ));
    }
    lines.push(r#"{"Command":"ReceiveMessage","QueueName":"q.fifo","MaxMessages":10}"#.into());
    lines.push(r#"{"Command":"ReceiveMessage","QueueName":"q.fifo","MaxMessages":10}"#.into());
    lines.push(r#"{"Command":"DeleteMessage","QueueName":"q.fifo","ReceiptHandle":"r-1"}"#.into());
    lines.push(r#"{"Command":"ReceiveMessage","QueueName":"q.fifo","MaxMessages":10}"#.into());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);

    let bodies = |response: &Value| -> Vec<String> {
        response["Messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["Body"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(bodies(&responses[6]), ["a1", "b1", "c1"]);
    assert_eq!(responses[6]["Messages"][0]["GroupId"], "a");
    assert_eq!(responses[6]["Messages"][0]["SequenceNumber"], 1);
    // Everything of every group is held back while the heads are in flight.
    assert_eq!(bodies(&responses[7]), Vec::<String>::new());
    // Deleting the head of group a releases its next message only.
    assert_eq!(bodies(&responses[9]), ["a2"]);
}

fn dead_letter_setup() -> Vec<&'static str> {
    vec![
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":2,"VisibilityTimeout":0}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"poison"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
    ]
}

#[test]
fn messages_move_to_the_dead_letter_queue_on_the_receive_after_the_last_allowed_one() {
    let mut lines = dead_letter_setup();
    lines.extend([
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
    ]);
    let responses = session(&lines);
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[5]["Counts"]["Visible"], 0);
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(responses[7]["Counts"]["Visible"], 0);
    assert_eq!(
        responses[8]["Messages"],
        json!([{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0,
        }])
    );
}

#[test]
fn dead_lettered_messages_keep_their_age_for_the_dead_letter_retention() {
    let mut lines = dead_letter_setup();
    lines.extend([
        r#"{"Command":"Tick","Seconds":100}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
    ]);
    let responses = session(&lines);
    // Sent at 0, retention 100 in the dead-letter queue: gone on arrival.
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_lettered_fifo_messages_take_new_sequence_numbers_and_free_their_group() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"d.fifo","MaxReceiveCount":1,"VisibilityTimeout":0}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"b","GroupId":"g","DeduplicationId":"2"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        // "a" is dead-lettered, which makes "b" eligible within the same call.
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"d.fifo"}"#,
    ]);
    assert_eq!(responses[4]["Messages"][0]["Body"], "a");
    assert_eq!(responses[5]["Messages"][0]["Body"], "b");
    let moved = &responses[6]["Messages"][0];
    assert_eq!(moved["Body"], "a");
    assert_eq!(moved["SequenceNumber"], 1);
    assert_eq!(moved["GroupId"], "g");
    assert_eq!(moved["ReceiveCount"], 1);
}

#[test]
fn dead_letter_configuration_rules() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#,
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":1}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":1}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq"}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":3}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":5}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
    ]);
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[3]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[4]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[5]), "InvalidParameterValue");
    ok(&responses[6]);
    ok(&responses[7]);
    assert_eq!(
        responses[8]["Attributes"],
        json!({
            "VisibilityTimeout": 5, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dlq", "MaxReceiveCount": 3,
        })
    );
    ok(&responses[9]);
    assert_eq!(
        responses[10]["Attributes"],
        json!({
            "VisibilityTimeout": 5, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
        })
    );
    assert_eq!(error_code(&responses[11]), "QueueDoesNotExist");
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"q"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(error_code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a_", "a-", "aB", "a1", "ab", "a.fifo"];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| {
            let attributes = if name.ends_with(".fifo") {
                r#","Attributes":{"Fifo":true}"#
            } else {
                ""
            };
            format!(r#"{{"Command":"CreateQueue","QueueName":"{name}"{attributes}}}"#)
        })
        .collect();
    lines.push(r#"{"Command":"ListQueues"}"#.into());
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.into());
    lines.push(r#"{"Command":"ListQueues","Prefix":"zz"}"#.into());
    lines.push(r#"{"Command":"ListQueues","Prefix":""}"#.into());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    let all = json!(["a-", "a.fifo", "a1", "aB", "a_", "ab", "b"]);
    assert_eq!(responses[7]["QueueNames"], all);
    assert_eq!(
        responses[8]["QueueNames"],
        json!(["a-", "a.fifo", "a1", "aB", "a_", "ab"])
    );
    assert_eq!(responses[9]["QueueNames"], json!([]));
    assert_eq!(responses[10]["QueueNames"], all);
}

#[test]
fn at_most_one_thousand_queues_may_exist() {
    let mut lines: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#.into());
    lines.push(r#"{"Command":"DeleteQueue","QueueName":"q5"}"#.into());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q1000"}"#.into());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    assert!(responses[..1000].iter().all(|r| r["ok"] == true));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    // Re-creating an existing queue is not creating one.
    ok(&responses[1001]);
    ok(&responses[1002]);
    ok(&responses[1003]);
}

#[test]
fn a_queue_holds_at_most_one_hundred_thousand_messages() {
    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#.to_owned()];
    lines.extend(
        (0..100_001).map(|_| r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#.to_owned()),
    );
    lines.push(r#"{"Command":"ReceiveMessage","QueueName":"q"}"#.into());
    lines.push(r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#.into());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    assert_eq!(responses[100_000]["MessageId"], "m-100000");
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
    // The failed send used no id, and the full queue still serves receives.
    assert_eq!(responses[100_002]["Messages"][0]["MessageId"], "m-1");
    assert_eq!(error_code(&responses[100_003]), "LimitExceeded");
}

#[test]
fn receiving_many_messages_is_not_quadratic() {
    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#.to_owned()];
    lines.extend(
        (0..20_000).map(|_| r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#.to_owned()),
    );
    lines.extend((0..20_000).map(|_| r#"{"Command":"ReceiveMessage","QueueName":"q"}"#.to_owned()));
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    assert_eq!(responses[40_000]["Messages"][0]["MessageId"], "m-20000");
}

#[test]
fn validation_rejects_numbers_that_are_not_plain_integers() {
    let responses = session(&[
        r#"{"Command":"Tick","Seconds":1.0}"#,
        r#"{"Command":"Tick","Seconds":1e0}"#,
        r#"{"Command":"Tick","Seconds":"1"}"#,
        r#"{"Command":"Tick","Seconds":-0}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":true}}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":0}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[2]), "InvalidParameterValue");
    assert_eq!(responses[3], json!({"ok": true, "Now": 0}));
    assert_eq!(error_code(&responses[4]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[5]), "InvalidParameterValue");
}

#[test]
fn a_failed_command_changes_nothing() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":5,"DeadLetterQueue":"nope","MaxReceiveCount":1}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(error_code(&responses[1]), "QueueDoesNotExist");
    assert_eq!(responses[2]["Attributes"]["VisibilityTimeout"], 30);
}
