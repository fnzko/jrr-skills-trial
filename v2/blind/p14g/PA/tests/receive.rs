mod common;

use common::{Session, code};
use serde_json::{Value, json};

fn send(session: &mut Session, queue: &str, body: &str) {
    let response =
        session.command(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
    assert_eq!(response["ok"], json!(true), "{response}");
}

fn send_fifo(session: &mut Session, queue: &str, body: &str, group: &str) {
    let response = session.command(
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
        "GroupId": group, "DeduplicationId": body}),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
}

fn receive(session: &mut Session, queue: &str, extra: Value) -> Value {
    let mut command = json!({"Command": "ReceiveMessage", "QueueName": queue});
    for (name, value) in extra.as_object().expect("extra parameters are an object") {
        command[name] = value.clone();
    }
    session.command(command)
}

/// The bodies of a successful receive, in delivery order.
fn bodies(response: &Value) -> Vec<&str> {
    assert_eq!(response["ok"], json!(true), "{response}");
    response["Messages"]
        .as_array()
        .expect("a successful receive has Messages")
        .iter()
        .map(|m| m["Body"].as_str().expect("Body is a string"))
        .collect()
}

fn counts(session: &mut Session, queue: &str) -> Value {
    let response = session.command(json!({"Command": "GetQueueAttributes", "QueueName": queue}));
    response["Counts"].clone()
}

#[test]
fn a_delivery_reports_every_member() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.tick(5);
    send(&mut session, "q", "hello");
    session.tick(7);
    assert_eq!(
        receive(&mut session, "q", json!({})),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 5, "FirstReceivedAt": 12
        }]})
    );
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn an_empty_queue_yields_an_empty_array() {
    let mut session = Session::new();
    session.create("q", json!({}));
    assert_eq!(
        receive(&mut session, "q", json!({})),
        json!({"ok": true, "Messages": []})
    );
}

#[test]
fn messages_come_back_in_arrival_order_after_their_timeout() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    for body in ["a", "b", "c"] {
        send(&mut session, "q", body);
    }
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 1}))),
        ["a"]
    );
    session.tick(3);
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 1}))),
        ["b"]
    );
    session.tick(7);
    // a's timeout ended at 10; b's is still running until 13.
    let again = receive(&mut session, "q", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&again), ["a", "c"]);
    assert_eq!(again["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(again["Messages"][0]["FirstReceivedAt"], json!(0));
    assert_eq!(again["Messages"][0]["ReceiptHandle"], json!("r-3"));
    assert_eq!(again["Messages"][1]["ReceiveCount"], json!(1));
}

#[test]
fn max_messages_limits_one_receive() {
    let mut session = Session::new();
    session.create("q", json!({}));
    for n in 0..12 {
        send(&mut session, "q", &format!("m{n}"));
    }
    assert_eq!(bodies(&receive(&mut session, "q", json!({}))), ["m0"]);
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 3}))),
        ["m1", "m2", "m3"]
    );
    let ten = receive(&mut session, "q", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&ten).len(), 8);
    assert_eq!(ten["Messages"][7]["ReceiptHandle"], json!("r-12"));
}

#[test]
fn receive_validates_its_parameters() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let cases = [
        (json!({"MaxMessages": 0}), "InvalidParameterValue"),
        (json!({"MaxMessages": 11}), "InvalidParameterValue"),
        (json!({"MaxMessages": "2"}), "InvalidParameterValue"),
        (
            json!({"VisibilityTimeout": 43_201}),
            "InvalidParameterValue",
        ),
        (json!({"VisibilityTimeout": -1}), "InvalidParameterValue"),
        (
            json!({"VisibilityTimeout": null, "MaxMessages": null}),
            "<ok>",
        ),
    ];
    for (extra, expected) in cases {
        assert_eq!(
            code(&receive(&mut session, "q", extra.clone())),
            expected,
            "{extra}"
        );
    }
    let missing = session.send(r#"{"Command": "ReceiveMessage"}"#);
    assert_eq!(code(&missing), "MissingParameter");
    let unknown =
        session.send(r#"{"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 0}"#);
    assert_eq!(code(&unknown), "InvalidParameterValue");
}

#[test]
fn the_request_timeout_overrides_the_queue_and_zero_means_visible_at_once() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 100}));
    send(&mut session, "q", "a");
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"VisibilityTimeout": 0}))),
        ["a"]
    );
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"VisibilityTimeout": 5}))),
        ["a"]
    );
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({}))),
        Vec::<&str>::new()
    );
    session.tick(4);
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({}))),
        Vec::<&str>::new()
    );
    session.tick(1);
    assert_eq!(bodies(&receive(&mut session, "q", json!({}))), ["a"]);
}

#[test]
fn a_new_queue_timeout_applies_to_later_deliveries_only() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    send(&mut session, "q", "a");
    receive(&mut session, "q", json!({}));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 1}}));
    session.tick(9);
    assert_eq!(counts(&mut session, "q")["InFlight"], json!(1));
    session.tick(1);
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
}

#[test]
fn delayed_messages_are_not_delivered_but_do_not_block_later_ones() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.command(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 10}),
    );
    send(&mut session, "q", "now");
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 10}))),
        ["now"]
    );
    session.tick(10);
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 10}))),
        ["late"]
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_none_from_a_blocked_group() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    for (body, group) in [
        ("a1", "a"),
        ("a2", "a"),
        ("b1", "b"),
        ("a3", "a"),
        ("b2", "b"),
    ] {
        send_fifo(&mut session, "f.fifo", body, group);
    }
    let first = receive(&mut session, "f.fifo", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&first), ["a1", "b1"]);
    assert_eq!(
        first["Messages"][1],
        json!({"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3})
    );
    assert_eq!(
        bodies(&receive(&mut session, "f.fifo", json!({"MaxMessages": 10}))),
        Vec::<&str>::new()
    );
    session.tick(10);
    assert_eq!(
        bodies(&receive(&mut session, "f.fifo", json!({"MaxMessages": 1}))),
        ["a1"]
    );
}

#[test]
fn a_delayed_head_of_group_blocks_the_group() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true, "DelaySeconds": 5}));
    send_fifo(&mut session, "f.fifo", "a1", "a");
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}));
    send_fifo(&mut session, "f.fifo", "a2", "a");
    send_fifo(&mut session, "f.fifo", "b1", "b");
    assert_eq!(
        bodies(&receive(&mut session, "f.fifo", json!({"MaxMessages": 10}))),
        ["b1"]
    );
    session.tick(5);
    assert_eq!(
        bodies(&receive(&mut session, "f.fifo", json!({"MaxMessages": 10}))),
        ["a1"]
    );
}

#[test]
fn exhausted_messages_move_to_the_dead_letter_queue_without_using_up_max_messages() {
    let mut session = Session::new();
    session.create("dead", json!({}));
    session.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
    );
    send(&mut session, "q", "poison");
    session.tick(3);
    send(&mut session, "q", "fine");
    // Two deliveries of "poison"; "fine" is not reached while max is 1.
    assert_eq!(bodies(&receive(&mut session, "q", json!({}))), ["poison"]);
    assert_eq!(bodies(&receive(&mut session, "q", json!({}))), ["poison"]);
    // The third receive moves "poison" and still delivers one message.
    session.tick(4);
    let third = receive(&mut session, "q", json!({}));
    assert_eq!(bodies(&third), ["fine"]);
    assert_eq!(
        counts(&mut session, "dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
    let moved = receive(&mut session, "dead", json!({}));
    assert_eq!(
        moved["Messages"][0],
        json!({"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 7})
    );
}

#[test]
fn dead_lettered_messages_are_expired_by_the_target_retention() {
    let mut session = Session::new();
    session.create("dead", json!({"RetentionSeconds": 60}));
    session.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    send(&mut session, "q", "old");
    send(&mut session, "q", "young");
    receive(&mut session, "q", json!({"MaxMessages": 2}));
    session.tick(60);
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({"MaxMessages": 1}))),
        Vec::<&str>::new()
    );
    assert_eq!(
        counts(&mut session, "dead"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(counts(&mut session, "q")["Visible"], json!(0));
}

#[test]
fn a_dead_lettered_fifo_head_unblocks_its_group_and_takes_a_new_sequence_number() {
    let mut session = Session::new();
    session.create("dead.fifo", json!({"Fifo": true}));
    session.create("q.fifo", json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}));
    send_fifo(&mut session, "dead.fifo", "other", "x");
    send_fifo(&mut session, "q.fifo", "a1", "a");
    send_fifo(&mut session, "q.fifo", "a2", "a");
    assert_eq!(bodies(&receive(&mut session, "q.fifo", json!({}))), ["a1"]);
    let second = receive(&mut session, "q.fifo", json!({}));
    assert_eq!(bodies(&second), ["a2"]);
    assert_eq!(second["Messages"][0]["SequenceNumber"], json!(2));
    let dead = receive(&mut session, "dead.fifo", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&dead), ["other", "a1"]);
    assert_eq!(dead["Messages"][1]["SequenceNumber"], json!(2));
    assert_eq!(dead["Messages"][1]["GroupId"], json!("a"));
}

#[test]
fn dead_letter_moves_ignore_deduplication_and_the_message_limit() {
    let mut session = Session::new();
    session.create("dead.fifo", json!({"Fifo": true}));
    session.create("q.fifo", json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}));
    send_fifo(&mut session, "dead.fifo", "same", "g");
    send_fifo(&mut session, "q.fifo", "same", "g");
    receive(&mut session, "q.fifo", json!({}));
    receive(&mut session, "q.fifo", json!({}));
    assert_eq!(counts(&mut session, "dead.fifo")["Visible"], json!(2));
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut session = Session::new();
    session.create("dead", json!({}));
    session.create("q", json!({"VisibilityTimeout": 0}));
    send(&mut session, "q", "a");
    receive(&mut session, "q", json!({}));
    receive(&mut session, "q", json!({}));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}));
    assert_eq!(counts(&mut session, "dead")["Visible"], json!(0));
    assert_eq!(
        bodies(&receive(&mut session, "q", json!({}))),
        Vec::<&str>::new()
    );
    assert_eq!(counts(&mut session, "dead")["Visible"], json!(1));
}
