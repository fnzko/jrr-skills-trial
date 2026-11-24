mod common;

use common::{error_code, exec};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn fifo_send(queue: &str, body: &str, group: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn receive_many(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, timeout: u32) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a ReceiveMessage success has Messages")
        .iter()
        .map(|m| m["Body"].as_str().expect("Body is a string"))
        .collect()
}

#[test]
fn receive_returns_all_fields_of_a_standard_message() {
    let responses = exec(&[
        create("q", json!({})),
        tick(7),
        send("q", "hello"),
        tick(3),
        receive("q"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 7, "FirstReceivedAt": 10
        }]})
    );
}

#[test]
fn receive_returns_fifo_fields_and_one_message_per_group() {
    let responses = exec(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a1", "A"),
        fifo_send("f.fifo", "a2", "A"),
        fifo_send("f.fifo", "b1", "B"),
        receive_many("f.fifo", 10),
        receive_many("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_many("f.fifo", 10),
    ]);
    assert_eq!(
        responses[4]["Messages"][0],
        json!({
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "A", "SequenceNumber": 1
        })
    );
    assert_eq!(bodies(&responses[4]), ["a1", "b1"]);
    assert_eq!(
        bodies(&responses[5]),
        Vec::<&str>::new(),
        "both groups have a message in flight"
    );
    assert_eq!(responses[6], json!({"ok": true}));
    assert_eq!(bodies(&responses[7]), ["a2"]);
}

#[test]
fn max_messages_limits_a_receive_and_defaults_to_one() {
    let responses = exec(&[
        create("q", json!({})),
        send("q", "1"),
        send("q", "2"),
        send("q", "3"),
        receive("q"),
        receive_many("q", 10),
        receive_many("q", 10),
    ]);
    assert_eq!(bodies(&responses[4]), ["1"]);
    assert_eq!(bodies(&responses[5]), ["2", "3"]);
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
}

#[test]
fn receive_checks_its_parameters() {
    let cases: Vec<(Value, &str)> = vec![
        (json!({"Command": "ReceiveMessage"}), "MissingParameter"),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 10}),
            "QueueDoesNotExist",
        ),
    ];
    for (command, code) in cases {
        assert_eq!(
            error_code(&exec(std::slice::from_ref(&command))[0]),
            Some(code),
            "{command}"
        );
    }
}

#[test]
fn a_message_becomes_visible_again_in_its_old_place_when_its_timeout_ends() {
    let responses = exec(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "first"),
        send("q", "second"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(bodies(&responses[3]), ["first"]);
    assert_eq!(bodies(&responses[5]), ["second"]);
    let again = &responses[7]["Messages"];
    assert_eq!(again[0]["Body"], json!("first"));
    assert_eq!(again[0]["ReceiveCount"], json!(2));
    assert_eq!(again[0]["FirstReceivedAt"], json!(0));
    assert_eq!(again[0]["ReceiptHandle"], json!("r-3"));
}

#[test]
fn receive_visibility_timeout_overrides_the_queue_and_zero_never_leaves_a_usable_handle() {
    let responses = exec(&[
        create("q", json!({"VisibilityTimeout": 100})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}),
        tick(5),
        receive("q"),
    ]);
    assert_eq!(error_code(&responses[3]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], json!(3));
}

#[test]
fn delay_hides_a_message_until_it_has_passed() {
    let responses = exec(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own-delay", "DelaySeconds": 2}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no-delay", "DelaySeconds": 0}),
        counts("q"),
        receive_many("q", 10),
        tick(2),
        receive_many("q", 10),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}),
        tick(8),
        receive_many("q", 10),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(bodies(&responses[5]), ["no-delay"]);
    assert_eq!(bodies(&responses[7]), ["own-delay"]);
    assert_eq!(bodies(&responses[10]), ["queue-delay"]);
}

#[test]
fn delete_and_change_reject_handles_that_were_never_issued() {
    let responses = exec(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        delete("q", "r-0"),
        delete("q", "r-+1"),
        delete("q", ""),
        change("q", "x", 1),
        delete("nope", "x"),
    ]);
    let codes: Vec<_> = responses[3..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("ReceiptHandleInvalid"),
            Some("QueueDoesNotExist"),
        ]
    );
}

#[test]
fn handles_expire_when_redelivered_deleted_timed_out_or_used_on_another_queue() {
    let responses = exec(&[
        create("q", json!({"VisibilityTimeout": 5})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        delete("other", "r-1"),
        tick(5),
        delete("q", "r-1"),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
        delete("q", "r-2"),
        change("q", "r-2", 10),
    ]);
    assert_eq!(error_code(&responses[4]), Some("ReceiptHandleExpired"));
    assert_eq!(error_code(&responses[6]), Some("ReceiptHandleExpired"));
    assert_eq!(error_code(&responses[8]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[9], json!({"ok": true}));
    assert_eq!(error_code(&responses[10]), Some("ReceiptHandleExpired"));
    assert_eq!(error_code(&responses[11]), Some("ReceiptHandleExpired"));
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let responses = exec(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(8),
        change("q", "r-1", 5),
        tick(4),
        receive("q"),
        tick(1),
        change("q", "r-1", 0),
        receive("q"),
    ]);
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(
        bodies(&responses[6]),
        Vec::<&str>::new(),
        "13 is not before 8 + 5"
    );
    assert_eq!(error_code(&responses[8]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[9]["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn change_visibility_zero_makes_the_message_visible_at_once() {
    let responses = exec(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        change("q", "r-1", 0),
        receive("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 43201}),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2"}),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(error_code(&responses[5]), Some("InvalidParameterValue"));
    assert_eq!(error_code(&responses[6]), Some("MissingParameter"));
}

#[test]
fn counts_show_each_state() {
    let responses = exec(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "c", "DelaySeconds": 5}),
        receive("q"),
        counts("q"),
        tick(10),
        counts("q"),
    ]);
    assert_eq!(
        responses[5],
        json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 10, "DelaySeconds": 0, "RetentionSeconds": 345600,
                           "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 1, "InFlight": 1, "Delayed": 1}})
    );
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 3, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_of_every_state_and_raising_it_does_not_restore_them() {
    let responses = exec(&[
        create(
            "q",
            json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000}),
        ),
        send("q", "visible"),
        send("q", "flying"),
        receive_many("q", 1),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
        tick(99),
        counts("q"),
        tick(1),
        counts("q"),
        send("q", "later"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[11]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[13]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_old_messages_at_once() {
    let responses = exec(&[
        create("q", json!({})),
        send("q", "old"),
        tick(100),
        send("q", "young"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        receive_many("q", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["young"]);
}

#[test]
fn purge_removes_every_message_but_keeps_sequence_numbers_and_deduplication() {
    let responses = exec(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g"),
        fifo_send("f.fifo", "b", "g"),
        receive("f.fifo"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        counts("f.fifo"),
        delete("f.fifo", "r-1"),
        fifo_send("f.fifo", "a", "g"),
        fifo_send("f.fifo", "c", "g"),
        json!({"Command": "PurgeQueue", "QueueName": "nope"}),
    ]);
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[6]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[7]["MessageId"], json!("m-1"), "still a duplicate");
    assert_eq!(
        responses[8],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(error_code(&responses[9]), Some("QueueDoesNotExist"));
}

#[test]
fn handles_are_numbered_by_delivery_across_queues() {
    let responses = exec(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "x"),
        send("b", "y"),
        receive("b"),
        receive("a"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], json!("r-1"));
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = exec(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q", json!({})),
        delete("q", "r-1"),
        receive("q"),
    ]);
    assert_eq!(error_code(&responses[5]), Some("ReceiptHandleExpired"));
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
}
