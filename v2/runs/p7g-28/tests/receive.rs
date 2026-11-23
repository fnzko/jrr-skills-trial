mod common;

use common::{code, codes, counts, create, run_cmds, send, tick};
use serde_json::{Value, json};

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn receive_n(queue: &str, n: i64) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": n})
}

fn receive_vt(queue: &str, vt: i64) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "VisibilityTimeout": vt})
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, vt: i64) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle,
           "VisibilityTimeout": vt})
}

fn fifo_send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group,
           "DeduplicationId": dedup})
}

fn bodies(r: &Value) -> Vec<&str> {
    r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn delivers_in_arrival_order_with_metadata() {
    let r = run_cmds(vec![
        create("q", json!({})),
        send("q", "a"),
        tick(5),
        send("q", "b"),
        send("q", "c"),
        receive_n("q", 2),
        receive("q"),
        receive("q"),
        counts("q"),
    ]);
    assert_eq!(
        r[5]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 5},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1, "SentAt": 5, "FirstReceivedAt": 5},
        ])
    );
    assert_eq!(r[6]["Messages"][0]["ReceiptHandle"], "r-3");
    assert_eq!(r[7]["Messages"], json!([]));
    assert_eq!(
        r[8]["Counts"],
        json!({"Visible": 0, "InFlight": 3, "Delayed": 0})
    );
}

#[test]
fn visibility_timeout_ends_and_keeps_arrival_position() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        receive("q"), // a
        tick(9),
        receive("q"), // b
        tick(1),
        receive("q"), // a again, before b
        receive("q"), // b is in flight until 19, a until 20
    ]);
    assert_eq!(bodies(&r[5]), ["b"]);
    assert_eq!(bodies(&r[7]), ["a"]);
    assert_eq!(r[7]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(r[7]["Messages"][0]["FirstReceivedAt"], 0);
    assert_eq!(bodies(&r[8]), Vec::<&str>::new());
}

#[test]
fn parameter_validation() {
    let r = run_cmds(vec![
        create("q", json!({})),
        json!({"Command": "ReceiveMessage"}),
        receive_n("q", 0),
        receive_n("q", 11),
        receive_vt("q", -1),
        receive_vt("q", 43201),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1.5}),
        receive("ghost"),
        receive_n("q", 10),
    ]);
    assert_eq!(
        codes(&r),
        [
            "ok",
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ok",
        ]
    );
}

#[test]
fn delayed_messages_are_not_delivered() {
    let r = run_cmds(vec![
        create("q", json!({"DelaySeconds": 5})),
        send("q", "a"),
        receive("q"),
        tick(5),
        receive("q"),
    ]);
    assert_eq!(bodies(&r[2]), Vec::<&str>::new());
    assert_eq!(bodies(&r[4]), ["a"]);
}

#[test]
fn delete_message_and_handle_errors() {
    let r = run_cmds(vec![
        create("q", json!({})),
        create("other", json!({})),
        send("q", "a"),
        send("other", "b"),
        receive("q"),     // r-1
        receive("other"), // r-2
        delete("q", "r-9"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-"),
        delete("q", "r-0"),
        delete("q", "r--1"),
        delete("q", "r-+1"),
        delete("q", "r-99999999999999999999999"),
        delete("q", "garbage"),
        delete("q", "r-2"), // issued for another queue
        delete("ghost", "r-9"),
        json!({"Command": "DeleteMessage", "QueueName": "q"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
        delete("q", "r-1"),
        delete("q", "r-1"),
        counts("q"),
    ]);
    assert_eq!(
        codes(&r)[6..],
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired",
            "QueueDoesNotExist",
            "MissingParameter",
            "InvalidParameterValue",
            "ok",
            "ReceiptHandleExpired",
            "ok",
        ]
    );
    assert_eq!(
        r[22]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn only_the_latest_handle_works() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 5})),
        send("q", "a"),
        receive("q"), // r-1
        tick(5),
        receive("q"), // r-2
        delete("q", "r-1"),
        change("q", "r-1", 10),
        delete("q", "r-2"),
    ]);
    assert_eq!(
        codes(&r)[5..],
        ["ReceiptHandleExpired", "ReceiptHandleExpired", "ok"]
    );
}

#[test]
fn expired_visibility_makes_the_handle_unusable() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 5})),
        send("q", "a"),
        receive("q"),
        tick(5),
        delete("q", "r-1"),
        receive_vt("q", 0), // r-2, visible at once
        delete("q", "r-2"),
    ]);
    assert_eq!(
        codes(&r)[4..],
        ["ReceiptHandleExpired", "ok", "ReceiptHandleExpired"]
    );
}

#[test]
fn change_message_visibility() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(6),
        change("q", "r-1", 10), // now in flight until 16
        tick(9),
        counts("q"),
        tick(1),
        counts("q"),
        receive("q"),
        change("q", "r-2", 0),
        counts("q"),
        change("q", "r-2", 5),
        change("q", "r-1", 44000),
        change("q", "r-9", 5),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2"}),
    ]);
    assert_eq!(r[6]["Counts"]["InFlight"], 1);
    assert_eq!(r[8]["Counts"]["Visible"], 1);
    assert_eq!(r[9]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(r[11]["Counts"]["Visible"], 1);
    assert_eq!(code(&r[12]), "ReceiptHandleExpired");
    assert_eq!(code(&r[13]), "InvalidParameterValue");
    assert_eq!(code(&r[14]), "ReceiptHandleInvalid");
    assert_eq!(code(&r[15]), "MissingParameter");
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let r = run_cmds(vec![
        create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10})),
        fifo_send("f.fifo", "a1", "A", "1"),
        fifo_send("f.fifo", "a2", "A", "2"),
        fifo_send("f.fifo", "b1", "B", "3"),
        fifo_send("f.fifo", "b2", "B", "4"),
        receive_n("f.fifo", 10), // a1, b1
        receive_n("f.fifo", 10), // nothing: both groups blocked
        tick(10),
        receive_n("f.fifo", 10), // a1, b1 again
        delete("f.fifo", "r-3"),
        receive_n("f.fifo", 10), // b1 in flight (blocked), a2 now at head of A but a1 deleted -> a2 only
    ]);
    assert_eq!(bodies(&r[5]), ["a1", "b1"]);
    assert_eq!(r[5]["Messages"][0]["GroupId"], "A");
    assert_eq!(r[5]["Messages"][0]["SequenceNumber"], 1);
    assert_eq!(bodies(&r[6]), Vec::<&str>::new());
    assert_eq!(bodies(&r[8]), ["a1", "b1"]);
    assert_eq!(bodies(&r[10]), ["a2"]);
}

#[test]
fn fifo_group_blocked_by_delayed_or_in_flight_head() {
    let r = run_cmds(vec![
        create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10})),
        fifo_send("f.fifo", "a1", "A", "1"),
        fifo_send("f.fifo", "a2", "A", "2"),
        receive("f.fifo"),
        change("f.fifo", "r-1", 0),
        receive_vt("f.fifo", 100),
        receive("f.fifo"),
    ]);
    assert_eq!(bodies(&r[5]), ["a1"]);
    assert_eq!(bodies(&r[6]), Vec::<&str>::new());
}

#[test]
fn dead_letter_moves_after_max_receive_count() {
    let r = run_cmds(vec![
        create("dlq", json!({"RetentionSeconds": 100, "DelaySeconds": 50})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        send("q", "a"),
        send("q", "b"),
        receive("q"),      // a (1)
        receive("q"),      // a (2)
        receive_n("q", 2), // a moves; b delivered
        counts("dlq"),
        receive("dlq"),
        tick(50),
        receive("dlq"),
        counts("q"),
    ]);
    assert_eq!(bodies(&r[6]), ["b"]);
    assert_eq!(
        r[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(bodies(&r[8]), ["a"]);
    let m = &r[8]["Messages"][0];
    assert_eq!(
        (&m["MessageId"], &m["ReceiveCount"], &m["SentAt"]),
        (&json!("m-1"), &json!(1), &json!(0))
    );
    assert_eq!(m["FirstReceivedAt"], 0);
}

#[test]
fn dead_letter_moves_do_not_count_toward_max_messages() {
    let r = run_cmds(vec![
        create("dlq", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        receive("q"),
        receive("q"),      // a is moved; b delivered
        receive_n("q", 2), // b moved; c delivered
        counts("dlq"),
    ]);
    assert_eq!(bodies(&r[6]), ["b"]);
    assert_eq!(bodies(&r[7]), ["c"]);
    assert_eq!(r[8]["Counts"]["Visible"], 2);
}

#[test]
fn dead_letter_retention_uses_target_queue_and_sent_at() {
    let r = run_cmds(vec![
        create("dlq", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "old"),
        receive("q"),
        tick(60),
        send("q", "new"),
        receive("q"),
        counts("dlq"),
    ]);
    // "old" is 60s old on arrival in dlq and is dropped at once.
    assert_eq!(r[7]["Counts"]["Visible"], 0);
}

#[test]
fn dead_lettered_fifo_message_takes_next_sequence_and_unblocks_group() {
    let r = run_cmds(vec![
        create("d.fifo", json!({"Fifo": true})),
        fifo_send("d.fifo", "x", "G", "x"),
        create(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
        ),
        fifo_send("q.fifo", "a", "G", "a"),
        fifo_send("q.fifo", "b", "G", "b"),
        receive("q.fifo"), // a (count 1)
        receive("q.fifo"), // a moved, then b delivered
        receive_n("d.fifo", 10),
    ]);
    assert_eq!(bodies(&r[6]), ["b"]);
    assert_eq!(bodies(&r[7]), ["x"]); // group G: x is first, a is behind it
    let all = run_cmds(vec![
        create("d.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
        ),
        fifo_send("q.fifo", "a", "G", "a"),
        receive("q.fifo"),
        receive("q.fifo"),
        receive("d.fifo"),
    ]);
    assert_eq!(all[5]["Messages"][0]["SequenceNumber"], 1);
    assert_eq!(all[5]["Messages"][0]["GroupId"], "G");
}

#[test]
fn deleted_dead_letter_queue_disables_moves() {
    let r = run_cmds(vec![
        create("dlq", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        receive("q"),
    ]);
    assert_eq!(bodies(&r[5]), ["a"]);
    assert_eq!(r[5]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn retention_removes_in_flight_messages_and_lowering_is_immediate() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 1000})),
        send("q", "a"),
        tick(100),
        send("q", "b"),
        receive_n("q", 2),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 100}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
    ]);
    assert_eq!(r[6]["Counts"]["InFlight"], 1);
    assert_eq!(r[8]["Counts"]["InFlight"], 1);
    assert_eq!(code(&r[9]), "ReceiptHandleExpired");
    assert_eq!(r[10]["ok"], true);
}
