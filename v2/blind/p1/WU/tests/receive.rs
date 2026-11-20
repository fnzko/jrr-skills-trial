mod common;

use common::Miniq;
use serde_json::{Value, json};

fn create(miniq: &mut Miniq, name: &str, attributes: Value) {
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(miniq: &mut Miniq, queue: &str, body: &str) {
    miniq.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn send_fifo(miniq: &mut Miniq, queue: &str, body: &str, group: &str) {
    miniq.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": body}));
}

fn tick(miniq: &mut Miniq, seconds: u64) {
    miniq.ok(json!({"Command": "Tick", "Seconds": seconds}));
}

fn receive(miniq: &mut Miniq, queue: &str, max: u64) -> Vec<Value> {
    let response =
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    response["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(miniq: &mut Miniq, queue: &str) -> Value {
    miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn a_delivery_carries_all_listed_members() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    tick(&mut miniq, 4);
    send(&mut miniq, "q", "hello");
    tick(&mut miniq, 6);
    assert_eq!(
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"})),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 4,
            "FirstReceivedAt": 10,
        }]})
    );
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(receive(&mut miniq, "q", 1), Vec::<Value>::new());
}

#[test]
fn messages_come_in_arrival_order_up_to_max_messages() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    for body in ["a", "b", "c", "d"] {
        send(&mut miniq, "q", body);
    }
    let first = receive(&mut miniq, "q", 3);
    assert_eq!(bodies(&first), ["a", "b", "c"]);
    let handles: Vec<&str> = first
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["r-1", "r-2", "r-3"]);
    let rest = miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10, "VisibilityTimeout": null}));
    assert_eq!(bodies(rest["Messages"].as_array().unwrap()), ["d"]);
    assert_eq!(rest["Messages"][0]["ReceiptHandle"], "r-4");
}

#[test]
fn handles_count_deliveries_over_all_queues() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "a", json!({}));
    create(&mut miniq, "b", json!({}));
    send(&mut miniq, "a", "x");
    send(&mut miniq, "b", "y");
    assert_eq!(receive(&mut miniq, "b", 1)[0]["ReceiptHandle"], "r-1");
    assert_eq!(receive(&mut miniq, "a", 1)[0]["ReceiptHandle"], "r-2");
    assert_eq!(receive(&mut miniq, "a", 1), Vec::<Value>::new());
    send(&mut miniq, "a", "z");
    assert_eq!(receive(&mut miniq, "a", 1)[0]["ReceiptHandle"], "r-3");
}

#[test]
fn an_unreceived_message_returns_in_its_place_after_the_visibility_timeout() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 5}));
    send(&mut miniq, "q", "a");
    send(&mut miniq, "q", "b");
    assert_eq!(bodies(&receive(&mut miniq, "q", 1)), ["a"]);
    assert_eq!(bodies(&receive(&mut miniq, "q", 1)), ["b"]);
    tick(&mut miniq, 4);
    assert_eq!(receive(&mut miniq, "q", 10), Vec::<Value>::new());
    tick(&mut miniq, 1);
    let again = receive(&mut miniq, "q", 10);
    assert_eq!(bodies(&again), ["a", "b"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
}

#[test]
fn first_received_at_stays_at_the_first_delivery() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 1}));
    send(&mut miniq, "q", "a");
    tick(&mut miniq, 3);
    receive(&mut miniq, "q", 1);
    tick(&mut miniq, 3);
    let again = receive(&mut miniq, "q", 1);
    assert_eq!(again[0]["FirstReceivedAt"], 3);
    assert_eq!(again[0]["ReceiveCount"], 2);
}

#[test]
fn request_visibility_timeout_overrides_the_queue_and_zero_means_visible_at_once() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 100}));
    send(&mut miniq, "q", "a");
    let first =
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(first["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let second =
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 2}));
    assert_eq!(second["Messages"][0]["ReceiveCount"], 2);
    tick(&mut miniq, 2);
    assert_eq!(receive(&mut miniq, "q", 1)[0]["ReceiveCount"], 3);
}

#[test]
fn queue_visibility_change_applies_to_later_deliveries_only() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 10}));
    send(&mut miniq, "q", "a");
    send(&mut miniq, "q", "b");
    receive(&mut miniq, "q", 1);
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 1}}));
    receive(&mut miniq, "q", 1);
    tick(&mut miniq, 1);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(bodies(&receive(&mut miniq, "q", 5)), ["b"]);
}

#[test]
fn delayed_messages_are_skipped_until_their_time() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    miniq
        .ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 5}));
    send(&mut miniq, "q", "now");
    assert_eq!(bodies(&receive(&mut miniq, "q", 10)), ["now"]);
    tick(&mut miniq, 5);
    assert_eq!(bodies(&receive(&mut miniq, "q", 10)), ["late"]);
}

#[test]
fn receive_parameter_errors() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    miniq.fails(json!({"Command": "ReceiveMessage"}), "MissingParameter");
    miniq.fails(
        json!({"Command": "ReceiveMessage", "QueueName": "nope"}),
        "QueueDoesNotExist",
    );
    miniq.fails(
        json!({"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 0}),
        "InvalidParameterValue",
    );
    for max in [json!(0), json!(11), json!(1.5), json!("1"), json!(-1)] {
        miniq.fails(
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": max}),
            "InvalidParameterValue",
        );
    }
    for timeout in [json!(-1), json!(43201), json!("1"), json!(1.0)] {
        miniq.fails(
            json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": timeout}),
            "InvalidParameterValue",
        );
    }
    miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10, "VisibilityTimeout": 43200}));
}

#[test]
fn fifo_delivers_one_message_per_group_in_group_order() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 10}),
    );
    send_fifo(&mut miniq, "q.fifo", "a1", "A");
    send_fifo(&mut miniq, "q.fifo", "a2", "A");
    send_fifo(&mut miniq, "q.fifo", "b1", "B");
    send_fifo(&mut miniq, "q.fifo", "a3", "A");
    let first = receive(&mut miniq, "q.fifo", 10);
    assert_eq!(bodies(&first), ["a1", "b1"]);
    assert_eq!(first[0]["GroupId"], "A");
    assert_eq!(first[0]["SequenceNumber"], 1);
    assert_eq!(first[1]["GroupId"], "B");
    assert_eq!(first[1]["SequenceNumber"], 3);
    // Both groups have a message in flight.
    assert_eq!(receive(&mut miniq, "q.fifo", 10), Vec::<Value>::new());
    tick(&mut miniq, 10);
    assert_eq!(bodies(&receive(&mut miniq, "q.fifo", 10)), ["a1", "b1"]);
}

#[test]
fn fifo_group_is_blocked_while_its_earliest_message_is_delayed() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "DelaySeconds": 5}),
    );
    send_fifo(&mut miniq, "q.fifo", "a1", "A");
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {"DelaySeconds": 0}}));
    send_fifo(&mut miniq, "q.fifo", "a2", "A");
    send_fifo(&mut miniq, "q.fifo", "b1", "B");
    assert_eq!(bodies(&receive(&mut miniq, "q.fifo", 10)), ["b1"]);
    tick(&mut miniq, 5);
    let first = receive(&mut miniq, "q.fifo", 10);
    assert_eq!(bodies(&first), ["a1"]);
}

fn dead_letter_setup(max_receive_count: u64) -> Miniq {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead", json!({}));
    create(
        &mut miniq,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": max_receive_count}),
    );
    miniq
}

#[test]
fn a_message_over_the_receive_limit_moves_to_the_dead_letter_queue() {
    let mut miniq = dead_letter_setup(2);
    tick(&mut miniq, 7);
    send(&mut miniq, "q", "poison");
    send(&mut miniq, "q", "fine");
    assert_eq!(bodies(&receive(&mut miniq, "q", 1)), ["poison"]);
    assert_eq!(bodies(&receive(&mut miniq, "q", 1)), ["poison"]);
    tick(&mut miniq, 3);
    // The move does not count toward MaxMessages: "fine" is delivered in the same call.
    let third = receive(&mut miniq, "q", 1);
    assert_eq!(bodies(&third), ["fine"]);
    assert_eq!(third[0]["ReceiveCount"], 1);
    assert_eq!(
        counts(&mut miniq, "dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let dead = receive(&mut miniq, "dead", 1);
    assert_eq!(
        dead[0],
        json!({
            "MessageId": "m-1",
            "ReceiptHandle": "r-4",
            "Body": "poison",
            "ReceiveCount": 1,
            "SentAt": 7,
            "FirstReceivedAt": 10,
        })
    );
}

#[test]
fn moved_messages_start_over_and_ignore_delay_and_the_message_limit() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead", json!({"DelaySeconds": 60}));
    create(
        &mut miniq,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    send(&mut miniq, "q", "x");
    receive(&mut miniq, "q", 1);
    assert_eq!(receive(&mut miniq, "q", 1), Vec::<Value>::new());
    assert_eq!(
        counts(&mut miniq, "dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_dead_message_older_than_the_dead_letter_retention_is_removed_on_arrival() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead", json!({"RetentionSeconds": 60}));
    create(
        &mut miniq,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    send(&mut miniq, "q", "x");
    send(&mut miniq, "q", "y");
    receive(&mut miniq, "q", 1);
    tick(&mut miniq, 59);
    // "x" moves (age 59, kept); "y" is delivered.
    assert_eq!(bodies(&receive(&mut miniq, "q", 1)), ["y"]);
    assert_eq!(counts(&mut miniq, "dead")["Visible"], 1);
    tick(&mut miniq, 1);
    assert_eq!(counts(&mut miniq, "dead")["Visible"], 0);

    send(&mut miniq, "q", "z");
    send(&mut miniq, "q", "w");
    receive(&mut miniq, "q", 2);
    tick(&mut miniq, 60);
    receive(&mut miniq, "q", 1);
    assert_eq!(
        counts(&mut miniq, "dead"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut miniq = dead_letter_setup(5);
    send(&mut miniq, "q", "x");
    receive(&mut miniq, "q", 1);
    receive(&mut miniq, "q", 1);
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}));
    assert_eq!(receive(&mut miniq, "q", 1), Vec::<Value>::new());
    assert_eq!(counts(&mut miniq, "dead")["Visible"], 1);
}

#[test]
fn fifo_dead_letters_take_new_sequence_numbers_and_free_their_group() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead.fifo", json!({"Fifo": true}));
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
    );
    send_fifo(&mut miniq, "dead.fifo", "earlier", "Z");
    send_fifo(&mut miniq, "q.fifo", "a1", "A");
    send_fifo(&mut miniq, "q.fifo", "a2", "A");
    assert_eq!(bodies(&receive(&mut miniq, "q.fifo", 1)), ["a1"]);
    // a1 moves out of the queue, so a2 is the earliest message of group A.
    let next = receive(&mut miniq, "q.fifo", 1);
    assert_eq!(bodies(&next), ["a2"]);
    let dead = receive(&mut miniq, "dead.fifo", 10);
    assert_eq!(bodies(&dead), ["earlier", "a1"]);
    assert_eq!(dead[1]["SequenceNumber"], 2);
    assert_eq!(dead[1]["GroupId"], "A");
    assert_eq!(dead[1]["MessageId"], "m-2");
}

#[test]
fn moves_do_not_record_deduplication_ids() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead.fifo", json!({"Fifo": true}));
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
    );
    send_fifo(&mut miniq, "q.fifo", "a1", "A");
    receive(&mut miniq, "q.fifo", 1);
    receive(&mut miniq, "q.fifo", 1);
    let sent = miniq.ok(json!({"Command": "SendMessage", "QueueName": "dead.fifo", "Body": "x", "GroupId": "A", "DeduplicationId": "a1"}));
    assert_eq!(sent["MessageId"], "m-2");
    assert_eq!(sent["SequenceNumber"], 2);
}
