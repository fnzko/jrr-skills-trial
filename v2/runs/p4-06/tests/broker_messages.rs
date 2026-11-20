//! Behaviour of the message commands, driven through the same parse-and-execute
//! path the stdin loop uses. Each test names the SPEC rule it pins down.

use miniq::command::parse_line;
use miniq::response::{failure, success};
use miniq::Broker;
use serde_json::{json, Value};

/// One command line through the protocol path, returning the response object.
fn call(broker: &mut Broker, command: Value) -> Value {
    let line = command.to_string();
    match parse_line(line.as_bytes()).and_then(|c| broker.execute(c)) {
        Ok(result) => success(result),
        Err(error) => failure(&error),
    }
}

fn ok(broker: &mut Broker, command: Value) -> Value {
    let response = call(broker, command.clone());
    assert_eq!(response["ok"], json!(true), "{command} failed: {response}");
    response
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "expected failure: {response}");
    response["error"]["code"].as_str().unwrap()
}

fn create(broker: &mut Broker, name: &str, attributes: Value) {
    ok(
        broker,
        json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}),
    );
}

fn send(broker: &mut Broker, queue: &str, body: &str) -> Value {
    ok(
        broker,
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body}),
    )
}

fn receive(broker: &mut Broker, queue: &str) -> Vec<Value> {
    let response = ok(
        broker,
        json!({"Command": "ReceiveMessage", "QueueName": queue}),
    );
    response["Messages"].as_array().unwrap().clone()
}

fn receive_max(broker: &mut Broker, queue: &str, max: u64) -> Vec<Value> {
    let response = ok(
        broker,
        json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}),
    );
    response["Messages"].as_array().unwrap().clone()
}

fn tick(broker: &mut Broker, seconds: u64) {
    ok(broker, json!({"Command": "Tick", "Seconds": seconds}));
}

fn counts(broker: &mut Broker, queue: &str) -> Value {
    let response = ok(
        broker,
        json!({"Command": "GetQueueAttributes", "QueueName": queue}),
    );
    response["Counts"].clone()
}

fn delete(broker: &mut Broker, queue: &str, handle: &str) -> Value {
    call(
        broker,
        json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle}),
    )
}

#[test]
fn a_sent_message_comes_back_with_its_first_delivery_recorded() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    assert_eq!(send(&mut broker, "q", "hello")["MessageId"], json!("m-1"));
    tick(&mut broker, 7);

    let messages = receive(&mut broker, "q");
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message["MessageId"], json!("m-1"));
    assert_eq!(message["ReceiptHandle"], json!("r-1"));
    assert_eq!(message["Body"], json!("hello"));
    assert_eq!(message["ReceiveCount"], json!(1));
    assert_eq!(message["SentAt"], json!(0));
    assert_eq!(message["FirstReceivedAt"], json!(7));
    assert!(
        message.get("GroupId").is_none(),
        "standard queues have no GroupId"
    );
    assert!(message.get("SequenceNumber").is_none());
}

#[test]
fn a_delivered_message_is_invisible_until_its_timeout_ends() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({"VisibilityTimeout": 30}));
    send(&mut broker, "q", "a");
    assert_eq!(receive(&mut broker, "q").len(), 1);
    assert!(receive(&mut broker, "q").is_empty());

    tick(&mut broker, 29);
    assert!(receive(&mut broker, "q").is_empty());
    tick(&mut broker, 1);
    let again = receive(&mut broker, "q");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0]["ReceiveCount"], json!(2));
    assert_eq!(again[0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn an_old_handle_is_expired_after_redelivery() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({"VisibilityTimeout": 1}));
    send(&mut broker, "q", "a");
    receive(&mut broker, "q");
    tick(&mut broker, 1);
    receive(&mut broker, "q");

    assert_eq!(
        code(&delete(&mut broker, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut broker, "q", "r-2")["ok"], json!(true));
}

#[test]
fn handles_that_were_never_issued_are_invalid_and_only_the_canonical_form_counts() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    send(&mut broker, "q", "a");
    receive(&mut broker, "q");

    for never in ["r-01", "R-1", "r-1 ", "r-0", "r-2", "r-", "1", "r-+1"] {
        assert_eq!(
            code(&delete(&mut broker, "q", never)),
            "ReceiptHandleInvalid",
            "{never:?}"
        );
    }
    assert_eq!(delete(&mut broker, "q", "r-1")["ok"], json!(true));
}

#[test]
fn a_handle_issued_for_another_queue_is_expired_here() {
    let mut broker = Broker::new();
    create(&mut broker, "a", json!({}));
    create(&mut broker, "b", json!({}));
    send(&mut broker, "a", "x");
    receive(&mut broker, "a");
    assert_eq!(
        code(&delete(&mut broker, "b", "r-1")),
        "ReceiptHandleExpired"
    );
}

#[test]
fn a_zero_visibility_timeout_makes_the_message_visible_and_its_handle_unusable() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    send(&mut broker, "q", "a");
    receive(&mut broker, "q");
    ok(
        &mut broker,
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}),
    );
    assert_eq!(counts(&mut broker, "q")["Visible"], json!(1));
    assert_eq!(
        code(&delete(&mut broker, "q", "r-1")),
        "ReceiptHandleExpired"
    );
}

#[test]
fn a_delayed_message_is_invisible_until_its_delay_ends() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    ok(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "later", "DelaySeconds": 10}),
    );
    assert!(receive(&mut broker, "q").is_empty());
    assert_eq!(counts(&mut broker, "q")["Delayed"], json!(1));
    tick(&mut broker, 10);
    assert_eq!(receive(&mut broker, "q").len(), 1);
}

#[test]
fn a_queue_default_delay_applies_to_sends_without_their_own() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({"DelaySeconds": 5}));
    send(&mut broker, "q", "a");
    assert!(receive(&mut broker, "q").is_empty());
    tick(&mut broker, 5);
    assert_eq!(receive(&mut broker, "q").len(), 1);
}

#[test]
fn a_message_expires_at_its_retention_even_while_in_flight() {
    let mut broker = Broker::new();
    create(
        &mut broker,
        "q",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
    );
    send(&mut broker, "q", "a");
    receive(&mut broker, "q");
    tick(&mut broker, 60);
    assert_eq!(
        counts(&mut broker, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        code(&delete(&mut broker, "q", "r-1")),
        "ReceiptHandleExpired"
    );
}

#[test]
fn lowering_retention_removes_messages_at_once() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    send(&mut broker, "q", "a");
    tick(&mut broker, 100);
    ok(
        &mut broker,
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
    );
    assert_eq!(counts(&mut broker, "q")["Visible"], json!(0));
    ok(
        &mut broker,
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 100000}}),
    );
    assert_eq!(
        counts(&mut broker, "q")["Visible"],
        json!(0),
        "raising it does not bring messages back"
    );
}

#[test]
fn a_fifo_group_delivers_one_message_at_a_time_in_order() {
    let mut broker = Broker::new();
    create(&mut broker, "f.fifo", json!({"Fifo": true}));
    for (group, body, dedup) in [("A", "a1", "d1"), ("A", "a2", "d2"), ("B", "b1", "d3")] {
        ok(
            &mut broker,
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body,
                   "GroupId": group, "DeduplicationId": dedup}),
        );
    }

    let first = receive_max(&mut broker, "f.fifo", 10);
    let bodies: Vec<&str> = first.iter().map(|m| m["Body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["a1", "b1"], "a2 waits behind a1 in group A");
    assert_eq!(first[0]["SequenceNumber"], json!(1));
    assert_eq!(first[0]["GroupId"], json!("A"));
    assert_eq!(first[1]["SequenceNumber"], json!(3));

    assert!(receive_max(&mut broker, "f.fifo", 10).is_empty());
    assert_eq!(delete(&mut broker, "f.fifo", "r-1")["ok"], json!(true));
    let next = receive(&mut broker, "f.fifo");
    assert_eq!(next[0]["Body"], json!("a2"));
}

#[test]
fn a_fifo_duplicate_within_the_window_returns_the_original_numbers() {
    let mut broker = Broker::new();
    create(&mut broker, "f.fifo", json!({"Fifo": true}));
    let send_one = |broker: &mut Broker, body: &str| {
        ok(
            broker,
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body,
                   "GroupId": "g", "DeduplicationId": "same"}),
        )
    };

    let original = send_one(&mut broker, "one");
    assert_eq!(original["MessageId"], json!("m-1"));
    assert_eq!(original["SequenceNumber"], json!(1));

    let duplicate = send_one(&mut broker, "two");
    assert_eq!(duplicate["MessageId"], json!("m-1"));
    assert_eq!(duplicate["SequenceNumber"], json!(1));
    assert_eq!(counts(&mut broker, "f.fifo")["Visible"], json!(1));

    // Deleting the original does not free its deduplication id.
    let delivered = receive(&mut broker, "f.fifo");
    assert_eq!(delivered[0]["Body"], json!("one"));
    assert_eq!(delete(&mut broker, "f.fifo", "r-1")["ok"], json!(true));
    assert_eq!(send_one(&mut broker, "three")["MessageId"], json!("m-1"));

    // The window closes 300 seconds after the send that opened it, not after the duplicate.
    tick(&mut broker, 299);
    assert_eq!(
        send_one(&mut broker, "still a duplicate")["MessageId"],
        json!("m-1")
    );
    tick(&mut broker, 1);
    let fresh = send_one(&mut broker, "fresh");
    assert_eq!(fresh["MessageId"], json!("m-2"));
    assert_eq!(fresh["SequenceNumber"], json!(2));
}

#[test]
fn content_based_deduplication_uses_the_body_as_the_id() {
    let mut broker = Broker::new();
    create(
        &mut broker,
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let first = ok(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"}),
    );
    let second = ok(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"}),
    );
    assert_eq!(first["MessageId"], second["MessageId"]);
}

#[test]
fn fifo_and_standard_send_rules_are_enforced_in_order() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    create(&mut broker, "f.fifo", json!({"Fifo": true}));

    let standard_with_group = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
    );
    assert_eq!(code(&standard_with_group), "UnsupportedOperation");

    let fifo_without_group = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}),
    );
    assert_eq!(code(&fifo_without_group), "MissingParameter");

    let fifo_without_dedup = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
    );
    assert_eq!(code(&fifo_without_dedup), "MissingParameter");

    let fifo_with_delay = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g",
               "DeduplicationId": "d", "DelaySeconds": 1}),
    );
    assert_eq!(code(&fifo_with_delay), "UnsupportedOperation");
}

#[test]
fn a_failed_command_consumes_no_message_id() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    let failed = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
    );
    assert_eq!(code(&failed), "UnsupportedOperation");
    assert_eq!(send(&mut broker, "q", "y")["MessageId"], json!("m-1"));
}

#[test]
fn missing_queue_is_reported_before_send_rules_are_checked() {
    let mut broker = Broker::new();
    let response = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "g"}),
    );
    assert_eq!(code(&response), "QueueDoesNotExist");
}

#[test]
fn a_message_that_exceeds_its_dead_letter_count_moves_and_does_not_count_toward_max() {
    let mut broker = Broker::new();
    create(&mut broker, "dlq", json!({}));
    create(
        &mut broker,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    send(&mut broker, "q", "poison");

    for handle in ["r-1", "r-2"] {
        assert_eq!(receive(&mut broker, "q").len(), 1);
        ok(
            &mut broker,
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": 0}),
        );
    }
    assert!(
        receive(&mut broker, "q").is_empty(),
        "the third receive moves it"
    );
    assert_eq!(counts(&mut broker, "q")["Visible"], json!(0));

    let moved = receive(&mut broker, "dlq");
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0]["MessageId"], json!("m-1"));
    assert_eq!(moved[0]["Body"], json!("poison"));
    assert_eq!(
        moved[0]["ReceiveCount"],
        json!(1),
        "the move reset the count to 0"
    );
}

#[test]
fn a_moved_message_past_the_dead_letter_retention_is_dropped_on_arrival() {
    let mut broker = Broker::new();
    create(&mut broker, "dlq", json!({"RetentionSeconds": 60}));
    create(
        &mut broker,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut broker, "q", "old");
    receive(&mut broker, "q");
    ok(
        &mut broker,
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}),
    );
    tick(&mut broker, 100);
    assert!(receive(&mut broker, "q").is_empty());
    assert_eq!(
        counts(&mut broker, "dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_messages_but_keeps_the_sequence_numbers() {
    let mut broker = Broker::new();
    create(&mut broker, "f.fifo", json!({"Fifo": true}));
    let send_fifo = |broker: &mut Broker, dedup: &str| {
        ok(
            broker,
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": dedup}),
        )
    };
    send_fifo(&mut broker, "one");
    ok(
        &mut broker,
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
    );
    assert_eq!(counts(&mut broker, "f.fifo")["Visible"], json!(0));
    assert_eq!(send_fifo(&mut broker, "two")["SequenceNumber"], json!(2));
}

#[test]
fn messages_count_toward_the_per_queue_limit_in_every_state() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    let started = std::time::Instant::now();
    for _ in 0..100_000 {
        send(&mut broker, "q", "x");
    }
    let took = started.elapsed();
    assert!(took.as_secs() < 30, "filling a queue took {took:?}");

    let response = call(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}),
    );
    assert_eq!(code(&response), "LimitExceeded");
    assert_eq!(counts(&mut broker, "q")["Visible"], json!(100_000));
}

#[test]
fn a_batch_receive_stops_at_max_messages() {
    let mut broker = Broker::new();
    create(&mut broker, "q", json!({}));
    for body in ["a", "b", "c"] {
        send(&mut broker, "q", body);
    }
    let batch = receive_max(&mut broker, "q", 2);
    let bodies: Vec<&str> = batch.iter().map(|m| m["Body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["a", "b"]);
    assert_eq!(receive(&mut broker, "q")[0]["Body"], json!("c"));
}
