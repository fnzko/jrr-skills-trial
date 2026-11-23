use serde_json::{Value, json};

use crate::common::{assert_responses, err, ok, run};

const STANDARD: &str = r#"{"Command":"CreateQueue","QueueName":"q"}"#;
const FIFO: &str = r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#;

fn tick(seconds: u32) -> String {
    format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#)
}

fn send(queue: &str, body: &str) -> String {
    format!(r#"{{"Command":"SendMessage","QueueName":"{queue}","Body":"{body}"}}"#)
}

fn send_to_group(body: &str, group: &str) -> String {
    format!(
        r#"{{"Command":"SendMessage","QueueName":"f.fifo","Body":"{body}","GroupId":"{group}"}}"#
    )
}

fn receive(queue: &str, extra: &str) -> String {
    format!(r#"{{"Command":"ReceiveMessage","QueueName":"{queue}"{extra}}}"#)
}

fn delete(queue: &str, handle: &str) -> String {
    format!(r#"{{"Command":"DeleteMessage","QueueName":"{queue}","ReceiptHandle":"{handle}"}}"#)
}

fn change(queue: &str, handle: &str, seconds: u32) -> String {
    format!(
        r#"{{"Command":"ChangeMessageVisibility","QueueName":"{queue}","ReceiptHandle":"{handle}","VisibilityTimeout":{seconds}}}"#
    )
}

fn counts(queue: &str) -> String {
    format!(r#"{{"Command":"GetQueueAttributes","QueueName":"{queue}"}}"#)
}

fn run_owned(lines: &[String]) -> Vec<Value> {
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&lines)
}

/// A delivered standard message.
fn delivered(id: u32, handle: u32, body: &str, count: u32, sent: u32, first: u32) -> Value {
    json!({
        "MessageId": format!("m-{id}"), "ReceiptHandle": format!("r-{handle}"), "Body": body,
        "ReceiveCount": count, "SentAt": sent, "FirstReceivedAt": first,
    })
}

fn messages(items: Vec<Value>) -> Value {
    json!({"ok": true, "Messages": items})
}

fn counts_of(response: &Value) -> Value {
    response["Counts"].clone()
}

#[test]
fn receive_delivers_in_arrival_order_up_to_max_messages() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        tick(5),
        send("q", "b"),
        send("q", "c"),
        receive("q", r#","MaxMessages":2"#),
        receive("q", ""),
        receive("q", ""),
        receive("q", r#","MaxMessages":null"#),
    ]);
    assert_eq!(
        responses[5..],
        [
            messages(vec![
                delivered(1, 1, "a", 1, 0, 5),
                delivered(2, 2, "b", 1, 5, 5)
            ]),
            messages(vec![delivered(3, 3, "c", 1, 5, 5)]),
            messages(vec![]),
            messages(vec![]),
        ]
    );
}

#[test]
fn receive_validates_parameters_before_the_queue() {
    assert_responses(
        run_owned(&[
            STANDARD.to_owned(),
            r#"{"Command":"ReceiveMessage"}"#.to_owned(),
            receive("nope", r#","MaxMessages":0"#),
            receive("nope", r#","MaxMessages":11"#),
            receive("nope", r#","MaxMessages":1.5"#),
            receive("nope", r#","VisibilityTimeout":43201"#),
            receive("nope", ""),
            receive("q", r#","MaxMessages":10,"VisibilityTimeout":43200"#),
        ]),
        vec![
            ok(),
            err("MissingParameter"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("QueueDoesNotExist"),
            json!({"ok": true, "Messages": []}),
        ],
    );
}

#[test]
fn a_delivered_message_is_hidden_until_its_visibility_timeout_ends_and_keeps_its_place() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        send("q", "b"),
        receive("q", ""),
        counts("q"),
        receive("q", ""),
        tick(29),
        receive("q", ""),
        tick(1),
        counts("q"),
        receive("q", ""),
    ]);
    assert_eq!(responses[3], messages(vec![delivered(1, 1, "a", 1, 0, 0)]));
    assert_eq!(
        counts_of(&responses[4]),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(responses[5], messages(vec![delivered(2, 2, "b", 1, 0, 0)]));
    assert_eq!(responses[7], messages(vec![]));
    assert_eq!(
        counts_of(&responses[9]),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[10], messages(vec![delivered(1, 3, "a", 2, 0, 0)]));
}

#[test]
fn the_receive_timeout_overrides_the_queue_attribute_for_that_delivery_only() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        receive("q", r#","VisibilityTimeout":5"#),
        tick(5),
        receive("q", ""),
        tick(29),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(responses[4], messages(vec![delivered(1, 2, "a", 2, 0, 0)]));
    assert_eq!(
        counts_of(&responses[6]),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        counts_of(&responses[8]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_queue_visibility_timeout_applies_to_later_deliveries_only() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        send("q", "b"),
        receive("q", ""),
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":100}}"#
            .to_owned(),
        receive("q", ""),
        tick(30),
        counts("q"),
        tick(70),
        counts("q"),
    ]);
    assert_eq!(
        counts_of(&responses[7]),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        counts_of(&responses[9]),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delayed_messages_are_not_delivered() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        r#"{"Command":"SendMessage","QueueName":"q","Body":"late","DelaySeconds":10}"#.to_owned(),
        send("q", "now"),
        receive("q", r#","MaxMessages":10"#),
        tick(10),
        receive("q", r#","MaxMessages":10"#),
    ]);
    assert_eq!(
        responses[3],
        messages(vec![delivered(2, 1, "now", 1, 0, 0)])
    );
    assert_eq!(
        responses[5],
        messages(vec![delivered(1, 2, "late", 1, 0, 10)])
    );
}

#[test]
fn a_zero_visibility_timeout_leaves_the_message_visible_with_an_unusable_handle() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        receive("q", r#","VisibilityTimeout":0"#),
        delete("q", "r-1"),
        receive("q", r#","VisibilityTimeout":0"#),
        change("q", "r-2", 10),
    ]);
    assert_eq!(responses[2], messages(vec![delivered(1, 1, "a", 1, 0, 0)]));
    assert_eq!(responses[3]["error"]["code"], json!("ReceiptHandleExpired"));
    assert_eq!(responses[4], messages(vec![delivered(1, 2, "a", 2, 0, 0)]));
    assert_eq!(responses[5]["error"]["code"], json!("ReceiptHandleExpired"));
}

#[test]
fn receipt_handles_count_deliveries_over_all_queues() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"r"}"#.to_owned(),
        send("q", "a"),
        send("r", "b"),
        receive("r", ""),
        receive("q", ""),
    ]);
    assert_eq!(responses[4], messages(vec![delivered(2, 1, "b", 1, 0, 0)]));
    assert_eq!(responses[5], messages(vec![delivered(1, 2, "a", 1, 0, 0)]));
}

#[test]
fn delete_message_needs_the_latest_handle_of_an_in_flight_message() {
    assert_responses(
        run_owned(&[
            STANDARD.to_owned(),
            r#"{"Command":"CreateQueue","QueueName":"r"}"#.to_owned(),
            send("q", "a"),
            receive("q", r#","VisibilityTimeout":10"#),
            tick(10),
            receive("q", ""),
            delete("q", "r-1"),
            delete("r", "r-2"),
            delete("q", "r-2"),
            delete("q", "r-2"),
            counts("q"),
        ]),
        vec![
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            messages(vec![delivered(1, 1, "a", 1, 0, 0)]),
            json!({"ok": true, "Now": 10}),
            messages(vec![delivered(1, 2, "a", 2, 0, 0)]),
            err("ReceiptHandleExpired"),
            err("ReceiptHandleExpired"),
            ok(),
            err("ReceiptHandleExpired"),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn handles_must_be_the_exact_text_of_an_issued_handle() {
    let mut lines = vec![STANDARD.to_owned(), send("q", "a"), receive("q", "")];
    let wrong = [
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r-2",
        "r-0",
        "r-",
        "m-1",
        "",
        "r-+1",
        "r--1",
        "r-1e0",
        "r-99999999999999999999",
    ];
    lines.extend(wrong.iter().map(|handle| delete("q", handle)));
    lines.push(delete("q", "r-1"));
    let responses = run_owned(&lines);
    let mut expected = vec![ok(), json!({"ok": true, "MessageId": "m-1"})];
    expected.push(messages(vec![delivered(1, 1, "a", 1, 0, 0)]));
    expected.extend(wrong.iter().map(|_| err("ReceiptHandleInvalid")));
    expected.push(ok());
    assert_responses(responses, expected);
}

#[test]
fn delete_and_change_check_parameters_then_the_queue_then_the_handle() {
    assert_responses(
        run(&[
            STANDARD,
            r#"{"Command":"DeleteMessage","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":5}"#,
            r#"{"Command":"DeleteMessage","QueueName":"nope","ReceiptHandle":"junk"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"junk"}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"nope","ReceiptHandle":"r-1","VisibilityTimeout":43201}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"nope","ReceiptHandle":"r-1","VisibilityTimeout":1}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":1}"#,
        ]),
        vec![
            ok(),
            err("MissingParameter"),
            err("MissingParameter"),
            err("InvalidParameterValue"),
            err("QueueDoesNotExist"),
            err("ReceiptHandleInvalid"),
            err("MissingParameter"),
            err("InvalidParameterValue"),
            err("QueueDoesNotExist"),
            err("ReceiptHandleInvalid"),
        ],
    );
}

#[test]
fn handles_of_vanished_messages_are_expired_not_invalid() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#
            .to_owned(),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        receive("q", r#","MaxMessages":3,"VisibilityTimeout":1000"#),
        delete("q", "r-1"),
        r#"{"Command":"PurgeQueue","QueueName":"q"}"#.to_owned(),
        delete("q", "r-2"),
        send("q", "d"),
        receive("q", r#","VisibilityTimeout":1000"#),
        tick(60),
        change("q", "r-4", 5),
        r#"{"Command":"DeleteQueue","QueueName":"q"}"#.to_owned(),
    ]);
    assert_eq!(responses[5], ok());
    assert_eq!(responses[7]["error"]["code"], json!("ReceiptHandleExpired"));
    assert_eq!(
        responses[11]["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        receive("q", ""),
        tick(10),
        change("q", "r-1", 100),
        tick(99),
        counts("q"),
        tick(1),
        counts("q"),
        receive("q", ""),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(
        counts_of(&responses[6]),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        counts_of(&responses[8]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[9], messages(vec![delivered(1, 2, "a", 2, 0, 0)]));
}

#[test]
fn change_message_visibility_can_shorten_to_zero_and_leaves_receive_count_alone() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", "a"),
        receive("q", ""),
        change("q", "r-1", 0),
        counts("q"),
        change("q", "r-1", 0),
        receive("q", ""),
        change("q", "r-2", 7),
        change("q", "r-2", 1),
        tick(1),
        receive("q", ""),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(
        counts_of(&responses[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[5]["error"]["code"], json!("ReceiptHandleExpired"));
    assert_eq!(responses[6], messages(vec![delivered(1, 2, "a", 2, 0, 0)]));
    assert_eq!(responses[10], messages(vec![delivered(1, 3, "a", 3, 0, 0)]));
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_a_group_while_its_head_is_busy() {
    let fifo = |id: u32, handle: u32, body: &str, group: &str, sequence: u32| {
        let mut message = delivered(id, handle, body, 1, 0, 0);
        message["GroupId"] = json!(group);
        message["SequenceNumber"] = json!(sequence);
        message
    };
    let responses = run_owned(&[
        FIFO.to_owned(),
        send_to_group("a1", "A"),
        send_to_group("a2", "A"),
        send_to_group("b1", "B"),
        receive("f.fifo", r#","MaxMessages":10"#),
        receive("f.fifo", r#","MaxMessages":10"#),
        delete("f.fifo", "r-1"),
        receive("f.fifo", r#","MaxMessages":10"#),
        tick(30),
        receive("f.fifo", r#","MaxMessages":10"#),
    ]);
    assert_eq!(
        responses[4],
        messages(vec![fifo(1, 1, "a1", "A", 1), fifo(3, 2, "b1", "B", 3)])
    );
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(responses[6], ok());
    assert_eq!(responses[7], messages(vec![fifo(2, 3, "a2", "A", 2)]));
    let again = |id: u32, handle: u32, body: &str, group: &str, sequence: u32| {
        let mut message = fifo(id, handle, body, group, sequence);
        message["ReceiveCount"] = json!(2);
        message["FirstReceivedAt"] = json!(0);
        message
    };
    assert_eq!(
        responses[9],
        messages(vec![again(2, 4, "a2", "A", 2), again(3, 5, "b1", "B", 3)])
    );
}

#[test]
fn a_group_waits_for_every_earlier_message_in_any_state() {
    let responses = run_owned(&[
        FIFO.to_owned(),
        send_to_group("a1", "A"),
        send_to_group("a2", "A"),
        receive("f.fifo", r#","VisibilityTimeout":0"#),
        receive("f.fifo", r#","MaxMessages":10,"VisibilityTimeout":0"#),
    ]);
    let bodies = |response: &Value| -> Vec<Value> {
        response["Messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .map(|message| message["Body"].clone())
            .collect()
    };
    assert_eq!(bodies(&responses[3]), vec![json!("a1")]);
    assert_eq!(bodies(&responses[4]), vec![json!("a1")]);
}

#[test]
fn a_dead_letter_move_resets_the_message_and_does_not_count_as_a_delivery() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"VisibilityTimeout":0}}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2,"VisibilityTimeout":0}}"#.to_owned(),
        send("q", "a"),
        tick(3),
        send("q", "b"),
        receive("q", ""),
        receive("q", ""),
        receive("q", ""),
        counts("q"),
        counts("dead"),
        receive("dead", ""),
        receive("q", ""),
    ]);
    assert_eq!(responses[5], messages(vec![delivered(1, 1, "a", 1, 0, 3)]));
    assert_eq!(responses[6], messages(vec![delivered(1, 2, "a", 2, 0, 3)]));
    // The third receive moves "a" away and delivers "b" instead.
    assert_eq!(responses[7], messages(vec![delivered(2, 3, "b", 1, 3, 3)]));
    assert_eq!(
        counts_of(&responses[8]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts_of(&responses[9]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[10], messages(vec![delivered(1, 4, "a", 1, 0, 3)]));
    assert_eq!(responses[11], messages(vec![delivered(2, 5, "b", 2, 3, 3)]));
}

#[test]
fn dead_letter_moves_keep_group_ids_take_new_sequence_numbers_and_ignore_deduplication() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"dead.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true,"VisibilityTimeout":0,"DeadLetterQueue":"dead.fifo","MaxReceiveCount":1}}"#.to_owned(),
        r#"{"Command":"SendMessage","QueueName":"dead.fifo","Body":"first","GroupId":"X"}"#.to_owned(),
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"first","GroupId":"G"}"#.to_owned(),
        receive("f.fifo", ""),
        receive("f.fifo", ""),
        receive("dead.fifo", r#","MaxMessages":10"#),
        r#"{"Command":"SendMessage","QueueName":"dead.fifo","Body":"first","GroupId":"X"}"#.to_owned(),
    ]);
    assert_eq!(responses[3]["SequenceNumber"], json!(1));
    assert_eq!(responses[5], messages(vec![]));
    let moved = &responses[6]["Messages"];
    assert_eq!(moved[0]["Body"], json!("first"));
    assert_eq!(moved[0]["MessageId"], json!("m-1"));
    assert_eq!(moved[0]["GroupId"], json!("X"));
    assert_eq!(moved[0]["SequenceNumber"], json!(1));
    assert_eq!(moved[1]["MessageId"], json!("m-2"));
    assert_eq!(moved[1]["GroupId"], json!("G"));
    assert_eq!(moved[1]["SequenceNumber"], json!(2));
    assert_eq!(moved[1]["ReceiveCount"], json!(1));
    // The move recorded no deduplication id, so only the earlier plain send is remembered.
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn a_moved_message_that_is_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"RetentionSeconds":60}}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":1,"VisibilityTimeout":0}}"#.to_owned(),
        send("q", "a"),
        receive("q", ""),
        tick(60),
        receive("q", ""),
        counts("dead"),
    ]);
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(
        counts_of(&responses[6]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_changed_dead_letter_configuration_applies_from_the_next_receive_and_deletion_removes_it() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"dead"}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0}}"#.to_owned(),
        send("q", "a"),
        receive("q", ""),
        receive("q", ""),
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#.to_owned(),
        receive("q", ""),
        counts("dead"),
        r#"{"Command":"CreateQueue","QueueName":"q2","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dead","MaxReceiveCount":1}}"#.to_owned(),
        send("q2", "b"),
        receive("q2", ""),
        r#"{"Command":"DeleteQueue","QueueName":"dead"}"#.to_owned(),
        receive("q2", ""),
    ]);
    assert_eq!(
        counts_of(&responses[7]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[6], messages(vec![]));
    assert_eq!(responses[12]["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn a_dead_letter_queue_receives_messages_from_several_sources() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"dead"}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dead","MaxReceiveCount":1}}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"b","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dead","MaxReceiveCount":1}}"#.to_owned(),
        send("a", "from-a"),
        send("b", "from-b"),
        receive("b", ""),
        receive("a", ""),
        receive("b", ""),
        receive("a", ""),
        receive("dead", r#","MaxMessages":10"#),
    ]);
    let bodies: Vec<&str> = responses[9]["Messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter_map(|message| message["Body"].as_str())
        .collect();
    assert_eq!(bodies, vec!["from-b", "from-a"]);
}
