use serde_json::{Value, json};

use crate::support::{Session, counts};

fn ok() -> Value {
    json!({"ok": true})
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, timeout: u32) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle,
           "VisibilityTimeout": timeout})
}

#[test]
fn get_queue_attributes_reports_defaults_and_changes() {
    let mut session = Session::default();
    session.create("dlq", json!({}));
    session.create("q", json!({}));
    assert_eq!(
        session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        json!({"ok": true,
               "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                              "Fifo": false, "ContentBasedDeduplication": false},
               "Counts": counts(0, 0, 0)})
    );
    let set = json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {
        "VisibilityTimeout": 5, "DelaySeconds": 7, "RetentionSeconds": 60,
        "DeadLetterQueue": "dlq", "MaxReceiveCount": 4}});
    assert_eq!(session.run(set), ok());
    assert_eq!(
        session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        json!({"ok": true,
               "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": 7, "RetentionSeconds": 60,
                              "Fifo": false, "ContentBasedDeduplication": false,
                              "DeadLetterQueue": "dlq", "MaxReceiveCount": 4},
               "Counts": counts(0, 0, 0)})
    );
    session.create("f.fifo", json!({"Fifo": true}));
    let set = json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo",
                     "Attributes": {"ContentBasedDeduplication": true}});
    assert_eq!(session.run(set), ok());
    assert_eq!(
        session.run(json!({"Command": "GetQueueAttributes", "QueueName": "f.fifo"}))["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": true, "ContentBasedDeduplication": true})
    );
    session.fails(
        json!({"Command": "GetQueueAttributes", "QueueName": "nope"}),
        "QueueDoesNotExist",
    );
}

#[test]
fn send_and_receive_on_a_standard_queue() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.create("other", json!({}));
    assert_eq!(
        session.send("q", "first"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(
        session.send("other", "elsewhere"),
        json!({"ok": true, "MessageId": "m-2"})
    );
    session.tick(3);
    assert_eq!(
        session.send("q", "second"),
        json!({"ok": true, "MessageId": "m-3"})
    );
    session.tick(2);
    assert_eq!(
        session.receive("q"),
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "first", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 5},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "second", "ReceiveCount": 1,
             "SentAt": 3, "FirstReceivedAt": 5},
        ]})
    );
    assert_eq!(session.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(session.counts("q"), counts(0, 2, 0));
    // The visibility timeout ends at 5 + 30; arrival order is kept.
    session.tick(29);
    assert_eq!(session.receive("q"), json!({"ok": true, "Messages": []}));
    session.tick(1);
    assert_eq!(
        session.receive("q"),
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "first", "ReceiveCount": 2,
             "SentAt": 0, "FirstReceivedAt": 5},
            {"MessageId": "m-3", "ReceiptHandle": "r-4", "Body": "second", "ReceiveCount": 2,
             "SentAt": 3, "FirstReceivedAt": 5},
        ]})
    );
}

#[test]
fn max_messages_and_visibility_timeout_parameters() {
    let mut session = Session::default();
    session.create("q", json!({"VisibilityTimeout": 100}));
    for body in ["a", "b", "c"] {
        session.send("q", body);
    }
    let receive = |max: Value, timeout: Value| {
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": max,
               "VisibilityTimeout": timeout})
    };
    let reply = session.run(receive(Value::Null, json!(10)));
    assert_eq!(reply["Messages"].as_array().map(Vec::len), Some(1));
    let reply = session.run(receive(json!(2), Value::Null));
    assert_eq!(reply["Messages"].as_array().map(Vec::len), Some(2));
    assert_eq!(session.counts("q"), counts(0, 3, 0));
    session.tick(10);
    assert_eq!(session.receive_ids("q"), ["m-1"]);
    session.fails(receive(json!(0), Value::Null), "InvalidParameterValue");
    session.fails(receive(json!(11), Value::Null), "InvalidParameterValue");
    session.fails(receive(Value::Null, json!(43201)), "InvalidParameterValue");
    session.fails(receive(Value::Null, json!(-1)), "InvalidParameterValue");
}

#[test]
fn delays_come_from_the_parameter_or_the_queue_at_send_time() {
    let mut session = Session::default();
    session.create("q", json!({"DelaySeconds": 10}));
    session.send("q", "queue delay");
    let send =
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own delay", "DelaySeconds": 0});
    assert_eq!(session.run(send), json!({"ok": true, "MessageId": "m-2"}));
    let set = json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}});
    assert_eq!(session.run(set), ok());
    assert_eq!(session.counts("q"), counts(1, 0, 1));
    assert_eq!(session.receive_ids("q"), ["m-2"]);
    session.tick(9);
    assert_eq!(session.receive_ids("q"), Vec::<String>::new());
    session.tick(1);
    assert_eq!(session.receive_ids("q"), ["m-1"]);
}

#[test]
fn delete_and_change_visibility_with_a_current_handle() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.send("q", "a");
    session.send("q", "b");
    assert_eq!(session.receive_ids("q"), ["m-1", "m-2"]);
    assert_eq!(session.run(delete("q", "r-1")), ok());
    assert_eq!(session.counts("q"), counts(0, 1, 0));
    assert_eq!(session.run(change("q", "r-2", 100)), ok());
    session.tick(99);
    assert_eq!(session.counts("q"), counts(0, 1, 0));
    // The handle stays usable after a change, until the new timeout ends.
    assert_eq!(session.run(change("q", "r-2", 0)), ok());
    assert_eq!(session.counts("q"), counts(1, 0, 0));
    session.fails(change("q", "r-2", 10), "ReceiptHandleExpired");
    let reply = session.receive("q");
    assert_eq!(reply["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(reply["Messages"][0]["ReceiptHandle"], json!("r-3"));
}

#[test]
fn receipt_handle_errors() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.create("other", json!({}));
    session.send("q", "a");
    session.receive_ids("q");
    let cases: &[(Value, &str)] = &[
        (
            json!({"Command": "DeleteMessage", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
            "MissingParameter",
        ),
        (change("q", "r-1", 43201), "InvalidParameterValue"),
        (delete("nope", "r-1"), "QueueDoesNotExist"),
        (delete("q", "r-2"), "ReceiptHandleInvalid"),
        (delete("q", "r-01"), "ReceiptHandleInvalid"),
        (delete("q", "R-1"), "ReceiptHandleInvalid"),
        (delete("q", "r-1 "), "ReceiptHandleInvalid"),
        (delete("q", "m-1"), "ReceiptHandleInvalid"),
        (change("q", "r-2", 5), "ReceiptHandleInvalid"),
        (delete("other", "r-1"), "ReceiptHandleExpired"),
        (change("other", "r-1", 5), "ReceiptHandleExpired"),
    ];
    for (command, code) in cases {
        session.fails(command.clone(), code);
    }
    assert_eq!(session.counts("q"), counts(0, 1, 0));

    // Superseded by a later delivery.
    session.tick(30);
    session.receive_ids("q");
    session.fails(delete("q", "r-1"), "ReceiptHandleExpired");
    // Timed out.
    session.tick(30);
    session.fails(delete("q", "r-2"), "ReceiptHandleExpired");
    // Deleted.
    session.receive_ids("q");
    assert_eq!(session.run(delete("q", "r-3")), ok());
    session.fails(delete("q", "r-3"), "ReceiptHandleExpired");
    // Purged.
    session.send("q", "b");
    session.receive_ids("q");
    assert_eq!(
        session.run(json!({"Command": "PurgeQueue", "QueueName": "q"})),
        ok()
    );
    session.fails(delete("q", "r-4"), "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_leaves_the_message_visible() {
    let mut session = Session::default();
    session.create("q", json!({"VisibilityTimeout": 0}));
    session.send("q", "a");
    assert_eq!(session.receive_ids("q"), ["m-1"]);
    assert_eq!(session.counts("q"), counts(1, 0, 0));
    session.fails(delete("q", "r-1"), "ReceiptHandleExpired");
    let reply = session.receive("q");
    assert_eq!(reply["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn send_message_errors_follow_the_check_order() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let send = |queue: &str, extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "x"});
        for (key, value) in extra.as_object().expect("extra members are an object") {
            command[key] = value.clone();
        }
        command
    };
    let cases: &[(Value, &str)] = &[
        (
            json!({"Command": "SendMessage", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "Body": "x", "GroupId": "bad group"}),
            "MissingParameter",
        ),
        (send("q", json!({"Body": ""})), "InvalidParameterValue"),
        (
            send("q", json!({"Body": "é".repeat(513)})),
            "InvalidParameterValue",
        ),
        (send("q", json!({"Body": 5})), "InvalidParameterValue"),
        (
            send("q", json!({"DelaySeconds": 901})),
            "InvalidParameterValue",
        ),
        (send("q", json!({"GroupId": ""})), "InvalidParameterValue"),
        (
            send("q", json!({"GroupId": "x".repeat(129)})),
            "InvalidParameterValue",
        ),
        (
            send("q", json!({"DeduplicationId": "a.b"})),
            "InvalidParameterValue",
        ),
        (
            send("nope", json!({"GroupId": "bad group"})),
            "InvalidParameterValue",
        ),
        (send("nope", json!({})), "QueueDoesNotExist"),
        (send("q", json!({"GroupId": "g"})), "UnsupportedOperation"),
        (
            send("q", json!({"DeduplicationId": "d"})),
            "UnsupportedOperation",
        ),
        (
            send("f.fifo", json!({"DelaySeconds": 0})),
            "UnsupportedOperation",
        ),
        (
            send("f.fifo", json!({"DeduplicationId": "d"})),
            "MissingParameter",
        ),
        (send("f.fifo", json!({"GroupId": "g"})), "MissingParameter"),
    ];
    for (command, code) in cases {
        session.fails(command.clone(), code);
    }
    assert_eq!(
        session.send("q", &"é".repeat(512)),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(
        session.run(send(
            "f.fifo",
            json!({"GroupId": "g", "DeduplicationId": "d", "DelaySeconds": null})
        )),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

fn fifo_send(group: &str, dedup: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group,
           "DeduplicationId": dedup})
}

#[test]
fn fifo_receive_delivers_one_message_per_free_group() {
    let mut session = Session::default();
    session.create("f.fifo", json!({"Fifo": true}));
    for (n, group) in ["a", "a", "b", "c", "b"].iter().enumerate() {
        session.run(fifo_send(group, &format!("d{n}"), &format!("body{n}")));
    }
    let reply = session.receive("f.fifo");
    assert_eq!(
        reply["Messages"][0],
        json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "body0", "ReceiveCount": 1,
               "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1})
    );
    let ids = |reply: &Value| -> Vec<Value> {
        reply["Messages"]
            .as_array()
            .expect("Messages")
            .iter()
            .map(|m| m["MessageId"].clone())
            .collect()
    };
    assert_eq!(ids(&reply), [json!("m-1"), json!("m-3"), json!("m-4")]);
    // Every group's head is in flight.
    assert_eq!(session.receive_ids("f.fifo"), Vec::<String>::new());
    assert_eq!(session.run(delete("f.fifo", "r-2")), ok());
    assert_eq!(session.receive_ids("f.fifo"), ["m-5"]);
    // Every head times out at 30; m-1 comes back before m-2 can go.
    session.tick(30);
    assert_eq!(session.receive_ids("f.fifo"), ["m-1", "m-4", "m-5"]);
}

#[test]
fn fifo_deduplication_window() {
    let mut session = Session::default();
    session.create("f.fifo", json!({"Fifo": true}));
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    assert_eq!(session.run(fifo_send("g", "d", "one")), first);
    session.tick(299);
    // Duplicates ignore the group and body and do not extend the window.
    assert_eq!(session.run(fifo_send("other", "d", "two")), first);
    assert_eq!(session.receive_ids("f.fifo"), ["m-1"]);
    assert_eq!(session.run(delete("f.fifo", "r-1")), ok());
    assert_eq!(session.run(fifo_send("g", "d", "three")), first);
    session.tick(1);
    assert_eq!(
        session.run(fifo_send("g", "d", "four")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    // Purging keeps the window and the sequence.
    assert_eq!(
        session.run(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"})),
        ok()
    );
    assert_eq!(
        session.run(fifo_send("g", "d", "five")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        session.run(fifo_send("g", "e", "six")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(session.counts("f.fifo"), counts(1, 0, 0));
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let mut session = Session::default();
    session.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str, dedup: Option<&str>| {
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g",
               "DeduplicationId": dedup})
    };
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    assert_eq!(session.run(send("same body", None)), first);
    assert_eq!(session.run(send("same body", None)), first);
    // An explicit id with the same text shares the key space.
    assert_eq!(
        session.run(send("other", Some("x"))),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        session.run(send("x", None)),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn retention_removes_messages_in_every_state() {
    let mut session = Session::default();
    session.create(
        "q",
        json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000}),
    );
    session.send("q", "in flight");
    assert_eq!(session.receive_ids("q"), ["m-1"]);
    session.send("q", "visible");
    let send =
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900});
    session.run(send);
    session.tick(10);
    session.send("q", "younger");
    session.tick(89);
    assert_eq!(session.counts("q"), counts(2, 1, 1));
    session.tick(1);
    assert_eq!(session.counts("q"), counts(1, 0, 0));
    session.fails(delete("q", "r-1"), "ReceiptHandleExpired");
}

#[test]
fn lowering_retention_removes_at_once_and_raising_does_not_restore() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.send("q", "old");
    session.tick(100);
    session.send("q", "new");
    let set = |retention: u32| {
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"RetentionSeconds": retention}})
    };
    assert_eq!(session.run(set(100)), ok());
    assert_eq!(session.counts("q"), counts(1, 0, 0));
    assert_eq!(session.run(set(1_209_600)), ok());
    assert_eq!(session.receive_ids("q"), ["m-2"]);
}

#[test]
fn purge_removes_every_message() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.send("q", "a");
    session.send("q", "b");
    session.receive_ids("q");
    session.send("q", "c");
    assert_eq!(
        session.run(json!({"Command": "PurgeQueue", "QueueName": "q"})),
        ok()
    );
    assert_eq!(session.counts("q"), counts(0, 0, 0));
    session.fails(
        json!({"Command": "PurgeQueue", "QueueName": "nope"}),
        "QueueDoesNotExist",
    );
    assert_eq!(
        session.send("q", "d"),
        json!({"ok": true, "MessageId": "m-4"})
    );
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut session = Session::default();
    session.create("f.fifo", json!({"Fifo": true}));
    session.run(fifo_send("g", "d", "a"));
    session.receive_ids("f.fifo");
    assert_eq!(
        session.run(json!({"Command": "DeleteQueue", "QueueName": "f.fifo"})),
        ok()
    );
    session.create("f.fifo", json!({"Fifo": true}));
    session.fails(delete("f.fifo", "r-1"), "ReceiptHandleExpired");
    assert_eq!(session.counts("f.fifo"), counts(0, 0, 0));
    assert_eq!(
        session.run(fifo_send("g", "d", "a")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut session = Session::default();
    session.create("q", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let line = br#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#;
    for _ in 0..100_000 {
        assert_eq!(session.line(line)["ok"], json!(true));
    }
    session.fails(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}),
        "LimitExceeded",
    );
    assert_eq!(session.counts("q"), counts(100_000, 0, 0));
    assert_eq!(
        session.send("other-queue-missing", "x")["error"]["code"],
        json!("QueueDoesNotExist")
    );
    assert_eq!(
        session.run(fifo_send("g", "d", "x")),
        json!({"ok": true, "MessageId": "m-100001", "SequenceNumber": 1})
    );
}
