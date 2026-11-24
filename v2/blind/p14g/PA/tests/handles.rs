mod common;

use common::{Session, code};
use serde_json::{Value, json};

fn handle_command(command: &str, queue: &str, handle: &str) -> Value {
    json!({"Command": command, "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": 10})
}

fn send(session: &mut Session, queue: &str, body: &str) {
    session.command(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn receive_handles(session: &mut Session, queue: &str, max: u64) -> Vec<String> {
    let response = session
        .command(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    response["Messages"]
        .as_array()
        .expect("a successful receive has Messages")
        .iter()
        .map(|m| {
            m["ReceiptHandle"]
                .as_str()
                .expect("handle is a string")
                .to_owned()
        })
        .collect()
}

fn counts(session: &mut Session, queue: &str) -> Value {
    let response = session.command(json!({"Command": "GetQueueAttributes", "QueueName": queue}));
    response["Counts"].clone()
}

#[test]
fn delete_removes_a_message_in_flight() {
    let mut session = Session::new();
    session.create("q", json!({}));
    send(&mut session, "q", "a");
    let handles = receive_handles(&mut session, "q", 1);
    session.expect_ok(handle_command("DeleteMessage", "q", &handles[0]));
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let again = session.command(handle_command("DeleteMessage", "q", &handles[0]));
    assert_eq!(code(&again), "ReceiptHandleExpired");
}

#[test]
fn handles_are_compared_as_exact_strings() {
    let mut session = Session::new();
    session.create("q", json!({}));
    send(&mut session, "q", "a");
    receive_handles(&mut session, "q", 1);
    for handle in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-2",
        "r-0",
        "",
        "1",
        "r--1",
        "r-99999999999999999999999",
    ] {
        for command in ["DeleteMessage", "ChangeMessageVisibility"] {
            let response = session.command(handle_command(command, "q", handle));
            assert_eq!(
                code(&response),
                "ReceiptHandleInvalid",
                "{command} {handle:?}"
            );
        }
    }
    assert_eq!(counts(&mut session, "q")["InFlight"], json!(1));
}

#[test]
fn a_redelivered_message_expires_its_older_handles() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    send(&mut session, "q", "a");
    let old = receive_handles(&mut session, "q", 1);
    session.tick(10);
    let new = receive_handles(&mut session, "q", 1);
    assert_eq!((old[0].as_str(), new[0].as_str()), ("r-1", "r-2"));
    let stale = session.command(handle_command("DeleteMessage", "q", &old[0]));
    assert_eq!(code(&stale), "ReceiptHandleExpired");
    session.expect_ok(handle_command("DeleteMessage", "q", &new[0]));
}

#[test]
fn a_handle_stops_working_when_its_timeout_ends() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    send(&mut session, "q", "a");
    let handles = receive_handles(&mut session, "q", 1);
    session.tick(9);
    session.expect_ok(handle_command("ChangeMessageVisibility", "q", &handles[0]));
    session.tick(9);
    // ChangeMessageVisibility moved the end to 19, so the handle still works.
    session.expect_ok(handle_command("ChangeMessageVisibility", "q", &handles[0]));
    session.tick(10);
    let late = session.command(handle_command("DeleteMessage", "q", &handles[0]));
    assert_eq!(code(&late), "ReceiptHandleExpired");
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable_at_once() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 0}));
    send(&mut session, "q", "a");
    let handles = receive_handles(&mut session, "q", 1);
    let response = session.command(handle_command("DeleteMessage", "q", &handles[0]));
    assert_eq!(code(&response), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_moves_the_end_of_the_timeout_and_keeps_the_receive_count() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 100}));
    send(&mut session, "q", "a");
    let handles = receive_handles(&mut session, "q", 1);
    session.tick(5);
    let mut shorten = handle_command("ChangeMessageVisibility", "q", &handles[0]);
    shorten["VisibilityTimeout"] = json!(2);
    session.expect_ok(shorten);
    session.tick(1);
    assert_eq!(counts(&mut session, "q")["InFlight"], json!(1));
    session.tick(1);
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
    let response = session.command(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(response["Messages"][0]["ReceiveCount"], json!(2));
    let mut release = handle_command("ChangeMessageVisibility", "q", "r-2");
    release["VisibilityTimeout"] = json!(0);
    session.expect_ok(release);
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
}

#[test]
fn a_handle_of_another_queue_or_a_vanished_message_is_expired() {
    let mut session = Session::new();
    session.create("a", json!({}));
    session.create("b", json!({}));
    send(&mut session, "a", "x");
    send(&mut session, "a", "y");
    let handles = receive_handles(&mut session, "a", 2);
    let other = session.command(handle_command("DeleteMessage", "b", &handles[0]));
    assert_eq!(code(&other), "ReceiptHandleExpired");
    session.expect_ok(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    for handle in &handles {
        let purged = session.command(handle_command("ChangeMessageVisibility", "a", handle));
        assert_eq!(code(&purged), "ReceiptHandleExpired");
    }
}

#[test]
fn a_handle_of_a_dead_lettered_or_expired_message_is_expired() {
    let mut session = Session::new();
    session.create("dead", json!({}));
    session.create("q", json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0, "RetentionSeconds": 60}));
    send(&mut session, "q", "a");
    let first = receive_handles(&mut session, "q", 1);
    receive_handles(&mut session, "q", 1);
    let moved = session.command(handle_command("DeleteMessage", "q", &first[0]));
    assert_eq!(code(&moved), "ReceiptHandleExpired");
    let on_dead = session.command(handle_command("DeleteMessage", "dead", &first[0]));
    assert_eq!(code(&on_dead), "ReceiptHandleExpired");

    send(&mut session, "dead", "b");
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "dead", "Attributes": {"VisibilityTimeout": 1000}}));
    let held = receive_handles(&mut session, "dead", 1);
    session.tick(345_600);
    let gone = session.command(handle_command("DeleteMessage", "dead", &held[0]));
    assert_eq!(code(&gone), "ReceiptHandleExpired");
}

#[test]
fn handle_commands_check_parameters_before_handles() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let cases = [
        (
            json!({"Command": "DeleteMessage", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "DeleteMessage", "ReceiptHandle": "r-1"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "junk"}),
            "ReceiptHandleInvalid",
        ),
        (
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "junk"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "ChangeMessageVisibility", "QueueName": "nope", "ReceiptHandle": "junk", "VisibilityTimeout": 43_201}),
            "InvalidParameterValue",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(
            code(&session.command(command.clone())),
            expected,
            "{command}"
        );
    }
}
