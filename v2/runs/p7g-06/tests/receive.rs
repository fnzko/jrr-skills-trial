mod common;

use common::{error_code, run};
use serde_json::json;

const STANDARD: &str = r#"{"Command":"CreateQueue","QueueName":"q"}"#;
const FIFO: &str = r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#;

fn send(body: &str) -> String {
    format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{body}"}}"#)
}

fn send_fifo(body: &str, group: &str) -> String {
    format!(
        r#"{{"Command":"SendMessage","QueueName":"f.fifo","Body":"{body}","GroupId":"{group}","DeduplicationId":"{body}"}}"#
    )
}

fn receive(max: u32) -> String {
    format!(r#"{{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":{max}}}"#)
}

fn tick(seconds: u32) -> String {
    format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#)
}

fn run_owned(commands: &[String]) -> Vec<serde_json::Value> {
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    run(&refs)
}

#[test]
fn receive_returns_messages_in_arrival_order_with_numbered_handles() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("a"),
        tick(4),
        send("b"),
        receive(10),
        receive(1),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1, "SentAt": 4, "FirstReceivedAt": 4}
        ]})
    );
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
}

#[test]
fn receive_validates_its_parameters() {
    let cases = [
        (r#"{"Command":"ReceiveMessage"}"#, "MissingParameter"),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":0}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":11}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":43201}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"nope","MaxMessages":99}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"nope"}"#,
            "QueueDoesNotExist",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(error_code(&run(&[STANDARD, line])[1]), code, "{line}");
    }
}

#[test]
fn a_message_returns_in_its_place_when_its_visibility_timeout_ends() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("a"),
        send("b"),
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":10}"#.to_owned(),
        tick(9),
        receive(1),
        tick(1),
        receive(10),
    ]);
    assert_eq!(responses[5]["Messages"][0]["MessageId"], "m-2");
    let ids: Vec<_> = responses[7]["Messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| {
            (
                m["MessageId"].clone(),
                m["ReceiveCount"].clone(),
                m["ReceiptHandle"].clone(),
            )
        })
        .collect();
    assert_eq!(ids, vec![(json!("m-1"), json!(2), json!("r-3"))]);
    assert_eq!(responses[7]["Messages"][0]["FirstReceivedAt"], 0);
}

#[test]
fn delayed_messages_are_skipped_until_their_delay_ends() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DelaySeconds":5}"#.to_owned(),
        send("b"),
        receive(10),
        tick(5),
        receive(10),
    ]);
    assert_eq!(responses[3]["Messages"].as_array().expect("array").len(), 1);
    assert_eq!(responses[5]["Messages"][0]["MessageId"], "m-1");
}

#[test]
fn delete_and_change_visibility_reject_handles_that_were_never_issued() {
    for handle in [
        "r-0",
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r-",
        "1",
        "r-2",
        "r--1",
        "r-+1",
        "",
        "r-99999999999999999999999",
    ] {
        for command in [
            format!(r#"{{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"{handle}"}}"#),
            format!(
                r#"{{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"{handle}","VisibilityTimeout":1}}"#
            ),
        ] {
            let responses =
                run_owned(&[STANDARD.to_owned(), send("a"), receive(1), command.clone()]);
            assert_eq!(
                error_code(&responses[3]),
                "ReceiptHandleInvalid",
                "{command}"
            );
        }
    }
}

#[test]
fn delete_removes_a_message_in_flight_and_the_handle_then_expires() {
    let delete = r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#;
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("a"),
        receive(1),
        delete.to_owned(),
        delete.to_owned(),
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#.to_owned(),
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn handles_expire_when_redelivered_timed_out_zero_timeout_or_from_another_queue() {
    let delete = |n: u32, queue: &str| {
        format!(r#"{{"Command":"DeleteMessage","QueueName":"{queue}","ReceiptHandle":"r-{n}"}}"#)
    };
    let responses = run_owned(&[
        STANDARD.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"p"}"#.to_owned(),
        send("a"),
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":5}"#.to_owned(),
        tick(5),
        delete(1, "q"),
        receive(1),
        delete(1, "q"),
        delete(2, "p"),
        tick(30),
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#.to_owned(),
        delete(3, "q"),
        r#"{"Command":"DeleteQueue","QueueName":"q"}"#.to_owned(),
        STANDARD.to_owned(),
        delete(3, "q"),
    ]);
    for index in [5, 7, 8, 11, 14] {
        assert_eq!(
            error_code(&responses[index]),
            "ReceiptHandleExpired",
            "line {index}"
        );
    }
}

#[test]
fn queue_lookup_comes_before_handle_validation() {
    let responses = run(&[
        r#"{"Command":"DeleteMessage","QueueName":"nope","ReceiptHandle":"junk"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"nope","ReceiptHandle":"junk","VisibilityTimeout":-1}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"nope","ReceiptHandle":"junk"}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[2]), "MissingParameter");
}

#[test]
fn change_visibility_moves_the_end_of_the_timeout_without_a_new_receive_count() {
    let change = |seconds: u32| {
        format!(
            r#"{{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":{seconds}}}"#
        )
    };
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("a"),
        receive(1),
        tick(20),
        change(50),
        tick(49),
        receive(1),
        tick(1),
        receive(1),
        change(0),
    ]);
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(error_code(&responses[9]), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("a"),
        receive(1),
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":0}"#.to_owned(),
        receive(1),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_busy_groups() {
    let responses = run_owned(&[
        FIFO.to_owned(),
        send_fifo("a1", "a"),
        send_fifo("a2", "a"),
        send_fifo("b1", "b"),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#.to_owned(),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#.to_owned(),
        r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#.to_owned(),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#.to_owned(),
    ]);
    assert_eq!(
        responses[4]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3}
        ])
    );
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[7]["Messages"][0]["MessageId"], "m-2");
    assert_eq!(responses[7]["Messages"].as_array().expect("array").len(), 1);
}

fn dead_letter_setup(dlq_attributes: &str) -> Vec<String> {
    vec![
        format!(r#"{{"Command":"CreateQueue","QueueName":"dlq","Attributes":{dlq_attributes}}}"#),
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":1,"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#.to_owned(),
        send("a"),
        send("b"),
    ]
}

#[test]
fn a_message_received_too_often_moves_to_the_dead_letter_queue_fresh() {
    let mut commands = dead_letter_setup("{}");
    commands.extend([
        receive(1),
        tick(1),
        receive(1),
        tick(1),
        receive(10),
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#.to_owned(),
        r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#.to_owned(),
    ]);
    let responses = run_owned(&commands);
    let ids: Vec<_> = responses[8]["Messages"]
        .as_array()
        .expect("array")
        .iter()
        .map(|m| m["MessageId"].clone())
        .collect();
    assert_eq!(ids, vec![json!("m-2")]);
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(
        responses[9]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[10]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2}])
    );
}

#[test]
fn a_moved_message_that_is_too_old_for_the_dead_letter_queue_is_dropped() {
    let mut commands = dead_letter_setup(r#"{"RetentionSeconds":60}"#);
    commands.extend([
        receive(1),
        tick(30),
        receive(1),
        tick(30),
        receive(1),
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#.to_owned(),
    ]);
    let responses = run_owned(&commands);
    assert_eq!(
        responses[9]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_moves_take_new_sequence_numbers_and_free_the_group() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"VisibilityTimeout":0,"DeadLetterQueue":"d.fifo","MaxReceiveCount":1}}"#.to_owned(),
        send_fifo("a1", "a"),
        send_fifo("a2", "a"),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#.to_owned(),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#.to_owned(),
        r#"{"Command":"ReceiveMessage","QueueName":"d.fifo"}"#.to_owned(),
    ]);
    assert_eq!(responses[5]["Messages"][0]["MessageId"], "m-2");
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(
        responses[6]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a1", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1}])
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        r#"{"Command":"CreateQueue","QueueName":"dlq"}"#.to_owned(),
        send("a"),
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#.to_owned(),
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#.to_owned(),
        receive(1),
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#.to_owned(),
    ]);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[6]["Counts"]["Visible"], 1);
}

#[test]
fn receiving_from_a_large_queue_one_message_at_a_time_stays_fast() {
    let mut commands = vec![STANDARD.to_owned()];
    commands.extend((0..20_000).map(|_| send("a")));
    commands.extend((0..200).map(|_| receive(10)));
    let responses = run_owned(&commands);
    assert_eq!(responses[20_001]["Messages"][0]["MessageId"], "m-1");
    assert_eq!(responses[20_200]["Messages"][9]["MessageId"], "m-2000");
}
