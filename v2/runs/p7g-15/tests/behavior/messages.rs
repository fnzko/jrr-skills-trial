use serde_json::{Value, json};

use crate::common::{error_code, session};

const CREATE_Q: &str = r#"{"Command": "CreateQueue", "QueueName": "q"}"#;
const CREATE_FIFO: &str =
    r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#;

fn send_fifo(group: &str, dedup: &str, body: &str) -> String {
    format!(
        r#"{{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "{body}", "GroupId": "{group}", "DeduplicationId": "{dedup}"}}"#
    )
}

fn tick(seconds: u32) -> String {
    format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#)
}

fn counts(queue: &str) -> String {
    format!(r#"{{"Command": "GetQueueAttributes", "QueueName": "{queue}"}}"#)
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a successful receive has Messages")
        .iter()
        .map(|message| message["Body"].as_str().expect("Body is a string"))
        .collect()
}

#[test]
fn a_sent_message_is_received_once_with_all_fields() {
    let responses = session(&[
        CREATE_Q,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "hello"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &counts("q"),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[2],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 0,
        }]})
    );
    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delayed_messages_become_visible_when_the_delay_ends() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "slow"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "fast", "DelaySeconds": 0}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}"#,
        &counts("q"),
        &tick(5),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}"#,
    ]);
    assert_eq!(bodies(&responses[3]), ["fast"]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 1})
    );
    assert_eq!(bodies(&responses[6]), ["slow"]);
}

#[test]
fn a_message_whose_visibility_timeout_ends_keeps_its_place_and_gets_a_new_handle() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 10}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &tick(5),
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &tick(5),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-3"}"#,
        &tick(5),
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
    ]);
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "a",
            "ReceiveCount": 2,
            "SentAt": 0,
            "FirstReceivedAt": 0,
        }]})
    );
    assert_eq!(error_code(&responses[8]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[9], json!({"ok": true}));
    assert_eq!(responses[11]["Messages"][0]["MessageId"], json!("m-2"));
    assert_eq!(responses[11]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[11]["Messages"][0]["FirstReceivedAt"], json!(5));
}

#[test]
fn receipt_handles_are_invalid_unless_issued_and_expired_unless_current() {
    let responses = session(&[
        CREATE_Q,
        r#"{"Command": "CreateQueue", "QueueName": "other"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        r#"{"Command": "SendMessage", "QueueName": "other", "Body": "b"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "other"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "R-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1 "}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-3"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "bad"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q"}"#,
    ]);
    let codes: Vec<_> = responses[6..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleExpired"),
            None,
            Some("ReceiptHandleExpired"),
            Some("QueueDoesNotExist"),
            Some("MissingParameter"),
        ]
    );
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = session(&[
        CREATE_Q,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        &counts("q"),
    ]);
    assert_eq!(error_code(&responses[3]), Some("ReceiptHandleExpired"));
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_message_visibility_moves_the_end_of_the_timeout() {
    let responses = session(&[
        CREATE_Q,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &tick(20),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 20}"#,
        &tick(15),
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &tick(5),
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 0}"#,
        &counts("q"),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 5}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 43201}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
    ]);
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[9], json!({"ok": true}));
    assert_eq!(responses[10]["Counts"]["Visible"], json!(1));
    assert_eq!(error_code(&responses[11]), Some("ReceiptHandleExpired"));
    assert_eq!(error_code(&responses[12]), Some("InvalidParameterValue"));
    assert_eq!(error_code(&responses[13]), Some("MissingParameter"));
}

#[test]
fn send_and_receive_parameters_are_checked_in_the_specified_order() {
    let long_body = "a".repeat(1025);
    let too_long_body = "é".repeat(513);
    let exact_body = "é".repeat(512);
    let too_long_command =
        format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{long_body}"}}"#);
    let multibyte_command =
        format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{too_long_body}"}}"#);
    let exact_command =
        format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{exact_body}"}}"#);
    let responses = session(&[
        CREATE_Q,
        &too_long_command,
        &multibyte_command,
        &exact_command,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": 5}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "DelaySeconds": 901}"#,
        r#"{"Command": "SendMessage", "QueueName": "nope", "Body": ""}"#,
        r#"{"Command": "SendMessage", "QueueName": "nope", "Body": "x"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": "1"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#,
    ]);
    let codes: Vec<_> = responses[1..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            None,
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("MissingParameter"),
            Some("InvalidParameterValue"),
            Some("QueueDoesNotExist"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("UnsupportedOperation"),
            Some("UnsupportedOperation"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
        ]
    );
}

#[test]
fn fifo_sends_number_messages_and_duplicates_return_the_original() {
    let responses = session(&[
        CREATE_FIFO,
        &send_fifo("g1", "d1", "a"),
        &send_fifo("g1", "d2", "b"),
        &send_fifo("g1", "d1", "ignored"),
        &send_fifo("g2", "d3", "c"),
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[3], responses[1]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "c", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 3},
        ]})
    );
}

#[test]
fn fifo_send_rules_in_order() {
    let responses = session(&[
        CREATE_FIFO,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}"#,
    ]);
    let codes: Vec<_> = responses[1..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("UnsupportedOperation"),
            Some("MissingParameter"),
            Some("MissingParameter"),
        ]
    );
}

#[test]
fn a_group_is_blocked_until_its_earlier_message_is_deleted() {
    let responses = session(&[
        CREATE_FIFO,
        &send_fifo("g", "d1", "a"),
        &send_fifo("g", "d2", "b"),
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#,
        &tick(30),
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-2"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#,
    ]);
    assert_eq!(bodies(&responses[3]), ["a"]);
    assert_eq!(bodies(&responses[5]), ["a"]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(bodies(&responses[7]), ["b"]);
}

#[test]
fn the_deduplication_window_is_300_seconds_and_a_duplicate_does_not_extend_it() {
    let responses = session(&[
        CREATE_FIFO,
        &send_fifo("g", "d", "a"),
        &tick(299),
        &send_fifo("g", "d", "a"),
        &tick(1),
        &send_fifo("g", "d", "a"),
        &tick(299),
        &send_fifo("g", "d", "a"),
        &tick(1),
        &send_fifo("g", "d", "a"),
    ]);
    let ids: Vec<_> = [1, 3, 5, 7, 9]
        .iter()
        .map(|&n| {
            (
                responses[n]["MessageId"].clone(),
                responses[n]["SequenceNumber"].clone(),
            )
        })
        .collect();
    assert_eq!(
        ids,
        [
            (json!("m-1"), json!(1)),
            (json!("m-1"), json!(1)),
            (json!("m-2"), json!(2)),
            (json!("m-2"), json!(2)),
            (json!("m-3"), json!(3)),
        ]
    );
}

#[test]
fn deduplication_ids_outlive_the_message_and_survive_a_purge() {
    let responses = session(&[
        CREATE_FIFO,
        &send_fifo("g", "d", "a"),
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}"#,
        &send_fifo("g", "d", "a"),
        r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#,
        &send_fifo("g", "d", "a"),
        &send_fifo("g", "e", "a"),
        &counts("f.fifo"),
    ]);
    assert_eq!(responses[4]["MessageId"], json!("m-1"));
    assert_eq!(responses[6]["MessageId"], json!("m-1"));
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[8]["Counts"]["Visible"], json!(1));
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "h"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "k"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "y", "GroupId": "g", "DeduplicationId": "x"}"#,
    ]);
    let ids: Vec<_> = responses[1..]
        .iter()
        .map(|r| r["MessageId"].clone())
        .collect();
    assert_eq!(
        ids,
        [json!("m-1"), json!("m-1"), json!("m-2"), json!("m-1")]
    );
}

#[test]
fn messages_expire_after_the_retention_period_and_lowering_it_expires_at_once() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        &tick(59),
        &counts("q"),
        &tick(1),
        &counts("q"),
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b"}"#,
        &tick(100),
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "c"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#,
        &counts("q"),
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}"#,
        &counts("q"),
    ]);
    assert_eq!(responses[3]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[5]["Counts"]["Visible"], json!(0));
    assert_eq!(responses[11]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[13]["Counts"]["Visible"], json!(1));
}

#[test]
fn a_message_that_expired_in_flight_cannot_be_deleted() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 60, "VisibilityTimeout": 600}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        &tick(60),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        &counts("q"),
    ]);
    assert_eq!(error_code(&responses[4]), Some("ReceiptHandleExpired"));
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn messages_over_the_receive_limit_move_to_the_dead_letter_queue_reset() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SendMessage", "QueueName": "src", "Body": "x"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        &tick(10),
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        &counts("src"),
        r#"{"Command": "ReceiveMessage", "QueueName": "dead"}"#,
    ]);
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "x",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 10,
        }]})
    );
}

#[test]
fn moved_messages_do_not_count_toward_max_messages() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
        r#"{"Command": "SendMessage", "QueueName": "src", "Body": "a"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        r#"{"Command": "SendMessage", "QueueName": "src", "Body": "b"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dead"}"#,
    ]);
    assert_eq!(bodies(&responses[5]), ["b"]);
    assert_eq!(bodies(&responses[6]), ["a"]);
}

#[test]
fn fifo_dead_letters_take_the_next_sequence_number_and_keep_their_group() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}}"#,
        r#"{"Command": "SendMessage", "QueueName": "dead.fifo", "Body": "own", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "src.fifo", "Body": "moved", "GroupId": "h", "DeduplicationId": "d"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dead.fifo", "MaxMessages": 10}"#,
        r#"{"Command": "SendMessage", "QueueName": "dead.fifo", "Body": "z", "GroupId": "g", "DeduplicationId": "d"}"#,
    ]);
    let delivered: Vec<_> = responses[6]["Messages"]
        .as_array()
        .expect("Messages is an array")
        .iter()
        .map(|m| {
            (
                m["Body"].clone(),
                m["GroupId"].clone(),
                m["SequenceNumber"].clone(),
            )
        })
        .collect();
    assert_eq!(
        delivered,
        [
            (json!("own"), json!("g"), json!(1)),
            (json!("moved"), json!("h"), json!(2)),
        ]
    );
    // The move does not record deduplication ids, the earlier send still does.
    assert_eq!(responses[7]["MessageId"], json!("m-1"));
}

#[test]
fn a_dead_letter_arrival_older_than_the_target_retention_is_dropped() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
        r#"{"Command": "SendMessage", "QueueName": "src", "Body": "old"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        &tick(100),
        r#"{"Command": "ReceiveMessage", "QueueName": "src"}"#,
        &counts("dead"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_failed_sends_use_no_id() {
    let mut commands = vec![CREATE_Q.to_owned()];
    commands.extend(
        (0..100_001)
            .map(|n| format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{n}"}}"#)),
    );
    commands.push(r#"{"Command": "PurgeQueue", "QueueName": "q"}"#.to_owned());
    commands.push(r#"{"Command": "SendMessage", "QueueName": "q", "Body": "after"}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = session(&refs);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(error_code(&responses[100_001]), Some("LimitExceeded"));
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn purge_removes_messages_in_every_state() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 10}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "delayed"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "visible", "DelaySeconds": 0}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "q"}"#,
        &counts("q"),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[6]), Some("ReceiptHandleExpired"));
}
