mod common;

use common::{assert_error, assert_ok, last, run, session};
use serde_json::json;

const STANDARD: &str = r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#;
const FIFO: &str =
    r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#;

fn send(queue: &str, body: &str) -> String {
    format!(r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "{body}"}}"#)
}

fn tick(seconds: u32) -> String {
    format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#)
}

fn receive(queue: &str) -> String {
    format!(r#"{{"Command": "ReceiveMessage", "QueueName": "{queue}"}}"#)
}

#[test]
fn send_to_a_standard_queue_returns_a_message_id_and_no_sequence_number() {
    let responses = session(&[STANDARD, &send("jobs", "hello"), &send("jobs", "again")]);

    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-2"}));
}

#[test]
fn send_to_a_missing_queue_fails() {
    let response = last(&[&send("nope", "hello")]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn failed_send_does_not_use_up_a_message_id() {
    let responses = session(&[&send("nope", "lost"), STANDARD, &send("jobs", "kept")]);

    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn body_must_be_1_to_1024_utf8_bytes() {
    let lines = [
        STANDARD.to_owned(),
        send("jobs", ""),
        send("jobs", &"a".repeat(1024)),
        send("jobs", &"a".repeat(1025)),
        // Each é is two bytes, so 512 of them are exactly 1024 bytes.
        send("jobs", &"é".repeat(512)),
        send("jobs", &"é".repeat(513)),
        // Each € is three bytes: 341 of them plus an `a` is exactly 1024 bytes.
        send("jobs", &format!("{}a", "€".repeat(341))),
        send("jobs", &"€".repeat(342)),
        r#"{"Command": "SendMessage", "QueueName": "jobs"}"#.to_owned(),
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": 3}"#.to_owned(),
    ];

    let responses = session(&lines.iter().map(String::as_str).collect::<Vec<_>>());

    assert_error(&responses[1], "InvalidParameterValue");
    assert_ok(&responses[2]);
    assert_error(&responses[3], "InvalidParameterValue");
    assert_ok(&responses[4]);
    assert_error(&responses[5], "InvalidParameterValue");
    assert_ok(&responses[6]);
    assert_error(&responses[7], "InvalidParameterValue");
    assert_error(&responses[8], "MissingParameter");
    assert_error(&responses[9], "InvalidParameterValue");
}

#[test]
fn delay_seconds_on_send_is_bounded() {
    let responses = session(&[
        STANDARD,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "DelaySeconds": 900}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "DelaySeconds": 901}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "DelaySeconds": "5"}"#,
    ]);

    assert_ok(&responses[1]);
    assert_error(&responses[2], "InvalidParameterValue");
    assert_error(&responses[3], "InvalidParameterValue");
}

#[test]
fn group_and_deduplication_ids_are_rejected_on_a_standard_queue() {
    let responses = session(&[
        STANDARD,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "DeduplicationId": "d"}"#,
    ]);

    assert_error(&responses[1], "UnsupportedOperation");
    assert_error(&responses[2], "UnsupportedOperation");
}

#[test]
fn identifiers_are_1_to_128_characters_of_the_token_set() {
    let responses = session(&[
        FIFO,
        &format!(
            r#"{{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "{}", "DeduplicationId": "d"}}"#,
            "g".repeat(128)
        ),
        &format!(
            r#"{{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "{}", "DeduplicationId": "d2"}}"#,
            "g".repeat(129)
        ),
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "c", "GroupId": "a.b", "DeduplicationId": "d3"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "d", "GroupId": "g", "DeduplicationId": ""}"#,
    ]);

    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_error(&responses[2], "InvalidParameterValue");
    assert_error(&responses[3], "InvalidParameterValue");
    assert_error(&responses[4], "InvalidParameterValue");
}

#[test]
fn fifo_send_needs_a_group_and_a_deduplication_id_unless_content_based() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DelaySeconds": 0}"#,
    ]);

    assert_error(&responses[1], "MissingParameter");
    assert_error(&responses[2], "MissingParameter");
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_error(&responses[4], "UnsupportedOperation");
}

#[test]
fn content_based_fifo_queue_derives_the_deduplication_id_from_the_body() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "c.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "y", "GroupId": "g"}"#,
    ]);

    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn duplicate_within_the_window_returns_the_original_and_changes_nothing() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
        &tick(299),
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "d1"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "f.fifo"}"#,
    ]);

    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[4]["Counts"]["Visible"], 1);
}

#[test]
fn duplicates_do_not_extend_the_window() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
        &tick(200),
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
        &tick(100),
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
    ]);

    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_ignores_the_group_and_is_per_queue() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "CreateQueue", "QueueName": "other.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g1", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g2", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "other.fifo", "Body": "a", "GroupId": "g1", "DeduplicationId": "d"}"#,
    ]);

    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn purge_keeps_sequence_numbers_and_deduplication_records() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d1"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "d2"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "f.fifo"}"#,
    ]);

    assert_ok(&responses[2]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_of_a_missing_queue_fails() {
    let response = last(&[r#"{"Command": "PurgeQueue", "QueueName": "nope"}"#]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn queue_default_delay_holds_messages_until_it_has_passed() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 10}}"#,
        &send("jobs", "a"),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
        &tick(9),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
        &tick(1),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(
        responses[2]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn send_delay_overrides_the_queue_default() {
    let responses = session(&[
        STANDARD,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "a", "DelaySeconds": 5}"#,
        &tick(4),
        receive("jobs").as_str(),
        &tick(1),
        receive("jobs").as_str(),
    ]);

    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[5]["Messages"][0]["MessageId"], "m-1");
}

#[test]
fn retention_removes_messages_at_exactly_their_retention_end() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}"#,
        &send("jobs", "a"),
        &tick(59),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
        &tick(1),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(responses[3]["Counts"]["Visible"], 1);
    assert_eq!(responses[5]["Counts"]["Visible"], 0);
}

#[test]
fn lowering_retention_removes_messages_at_once() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &tick(100),
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(responses[4]["Counts"]["Visible"], 0);
}

#[test]
fn raising_retention_does_not_bring_removed_messages_back() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}"#,
        &send("jobs", "a"),
        &tick(60),
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 1000}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(responses[4]["Counts"]["Visible"], 0);
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_the_next_send_is_refused() {
    let send_line = send("jobs", "a");
    let mut lines = vec![STANDARD.to_owned()];
    lines.extend((0..100_000).map(|_| send_line.clone()));
    lines.push(send("jobs", "one too many"));
    lines.push(r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#.to_owned());

    let responses = run(&lines.join("\n"));

    assert_eq!(responses.len(), 100_003);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_error(&responses[100_001], "LimitExceeded");
    assert_eq!(responses[100_002]["Counts"]["Visible"], 100_000);
}

#[test]
fn receive_from_an_empty_queue_returns_no_messages() {
    let responses = session(&[STANDARD, &receive("jobs")]);

    assert_eq!(responses[1], json!({"ok": true, "Messages": []}));
}

#[test]
fn receive_returns_the_message_fields_of_a_standard_queue() {
    let responses = session(&[STANDARD, &tick(5), &send("jobs", "hello"), &receive("jobs")]);

    assert_eq!(
        responses[3],
        json!({
            "ok": true,
            "Messages": [{
                "MessageId": "m-1",
                "ReceiptHandle": "r-1",
                "Body": "hello",
                "ReceiveCount": 1,
                "SentAt": 5,
                "FirstReceivedAt": 5,
            }],
        })
    );
}

#[test]
fn receive_delivers_one_message_by_default_and_up_to_ten_on_request() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &send("jobs", "b"),
        &send("jobs", "c"),
        &receive("jobs"),
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "MaxMessages": 10}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "MaxMessages": 0}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "MaxMessages": 11}"#,
    ]);

    assert_eq!(responses[4]["Messages"].as_array().unwrap().len(), 1);
    assert_eq!(responses[4]["Messages"][0]["Body"], "a");
    let rest: Vec<&str> = responses[5]["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect();
    assert_eq!(rest, ["b", "c"]);
    assert_error(&responses[6], "InvalidParameterValue");
    assert_error(&responses[7], "InvalidParameterValue");
}

#[test]
fn delivered_message_is_hidden_for_the_visibility_timeout_then_redelivered() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &receive("jobs"),
        &tick(29),
        &receive("jobs"),
        &tick(1),
        &receive("jobs"),
    ]);

    assert_eq!(responses[2]["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[7]["Messages"],
        json!([{
            "MessageId": "m-1",
            "ReceiptHandle": "r-2",
            "Body": "a",
            "ReceiveCount": 2,
            "SentAt": 0,
            "FirstReceivedAt": 0,
        }])
    );
}

#[test]
fn receive_visibility_timeout_overrides_the_queue_attribute() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "VisibilityTimeout": 5}"#,
        &tick(5),
        &receive("jobs"),
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "VisibilityTimeout": 43201}"#,
    ]);

    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_error(&responses[5], "InvalidParameterValue");
}

#[test]
fn zero_visibility_timeout_makes_the_message_visible_at_once() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "VisibilityTimeout": 0}"#,
        &receive("jobs"),
    ]);

    assert_eq!(responses[3]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn redelivery_keeps_arrival_order() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "first"),
        &send("jobs", "second"),
        &receive("jobs"),
        &receive("jobs"),
        &tick(30),
        &receive("jobs"),
    ]);

    assert_eq!(responses[3]["Messages"][0]["Body"], "first");
    assert_eq!(responses[4]["Messages"][0]["Body"], "second");
    assert_eq!(responses[6]["Messages"][0]["Body"], "first");
}

#[test]
fn expired_messages_are_not_delivered() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}"#,
        &send("jobs", "a"),
        &tick(60),
        &receive("jobs"),
    ]);

    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
}

#[test]
fn counts_report_visible_in_flight_and_delayed_messages() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &send("jobs", "b"),
        &receive("jobs"),
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DelaySeconds": 10}}"#,
        &send("jobs", "c"),
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
}

#[test]
fn receive_on_a_missing_queue_fails() {
    let response = last(&[&receive("nope")]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn fifo_delivers_at_most_one_message_per_group_per_receive() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "CreateQueue", "QueueName": "c.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "a1", "GroupId": "g1"}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "a2", "GroupId": "g1"}"#,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "b1", "GroupId": "g2"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "c.fifo", "MaxMessages": 10}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "c.fifo", "MaxMessages": 10}"#,
        &tick(30),
        r#"{"Command": "ReceiveMessage", "QueueName": "c.fifo", "MaxMessages": 10}"#,
    ]);

    assert_eq!(
        responses[5]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 3},
        ])
    );
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    // Once visibility has ended, a1 and b1 are both visible again; a2 still waits behind a1.
    let bodies: Vec<&str> = responses[8]["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, ["a1", "b1"]);
}

#[test]
fn a_delayed_head_message_blocks_its_group() {
    let responses = session(&[
        FIFO,
        r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 10}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a1", "GroupId": "g", "DeduplicationId": "d1"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a2", "GroupId": "g", "DeduplicationId": "d2"}"#,
        receive("f.fifo").as_str(),
        &tick(10),
        receive("f.fifo").as_str(),
    ]);

    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7]["Messages"][0]["Body"], "a1");
    assert_eq!(responses[7]["Messages"].as_array().unwrap().len(), 1);
}

#[test]
fn draining_a_queue_one_message_at_a_time_delivers_each_message_once_in_order() {
    const COUNT: usize = 10_000;
    let mut lines = vec![STANDARD.to_owned()];
    lines.extend((0..COUNT).map(|n| send("jobs", &format!("m{n}"))));
    lines.extend((0..=COUNT).map(|_| receive("jobs")));

    let responses = run(&lines.join("\n"));

    assert_eq!(responses.len(), 2 + 2 * COUNT);
    for (n, response) in responses[1 + COUNT..1 + 2 * COUNT].iter().enumerate() {
        let message = &response["Messages"][0];
        assert_eq!(message["Body"].as_str(), Some(format!("m{n}").as_str()));
        assert_eq!(
            message["ReceiptHandle"].as_str(),
            Some(format!("r-{}", n + 1).as_str())
        );
    }
    assert_eq!(
        responses[2 * COUNT + 1],
        json!({"ok": true, "Messages": []})
    );
}
