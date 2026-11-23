mod common;

use common::{code, codes, counts, create, run_cmds, send, tick};
use serde_json::{Value, json};

fn fifo_send(queue: &str, body: &str, group: &str, dedup: Option<&str>) -> Value {
    let mut v =
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group});
    if let Some(d) = dedup {
        v["DeduplicationId"] = json!(d);
    }
    v
}

#[test]
fn ids_count_accepted_messages_over_all_queues() {
    let r = run_cmds(vec![
        create("a", json!({})),
        create("b", json!({})),
        send("a", "x"),
        send("nope", "x"),
        send("b", "y"),
        json!({"Command": "SendMessage", "QueueName": "a", "Body": ""}),
        send("a", "z"),
    ]);
    assert_eq!(r[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(r[4]["MessageId"], "m-2");
    assert_eq!(r[6]["MessageId"], "m-3");
    assert_eq!(code(&r[5]), "InvalidParameterValue");
}

#[test]
fn parameter_validation_precedes_queue_lookup() {
    let r = run_cmds(vec![
        json!({"Command": "SendMessage", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "ghost"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x", "DelaySeconds": 901}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x", "GroupId": "a b"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x", "GroupId": ""}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x", "DeduplicationId": "é"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x".repeat(1025)}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": 5}),
    ]);
    assert_eq!(
        codes(&r),
        [
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn body_length_is_counted_in_bytes() {
    let ok = "é".repeat(512);
    let long = format!("{ok}a");
    let r = run_cmds(vec![
        create("q", json!({})),
        send("q", &ok),
        send("q", &long),
    ]);
    assert_eq!(codes(&r), ["ok", "ok", "InvalidParameterValue"]);
}

#[test]
fn standard_and_fifo_queues_reject_each_others_parameters() {
    let r = run_cmds(vec![
        create("s", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        create(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        fifo_send("s", "x", "g", None),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g",
               "DeduplicationId": "d", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}),
        fifo_send("f.fifo", "x", "g", None),
        fifo_send("c.fifo", "x", "g", None),
        fifo_send("f.fifo", "x", "g", Some("d")),
    ]);
    assert_eq!(
        codes(&r),
        [
            "ok",
            "ok",
            "ok",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "ok",
            "ok",
        ]
    );
    assert_eq!(r[8]["SequenceNumber"], 1);
    assert_eq!(r[9]["SequenceNumber"], 1);
}

#[test]
fn delay_comes_from_the_request_or_the_queue() {
    let r = run_cmds(vec![
        create("q", json!({"DelaySeconds": 10})),
        send("q", "a"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "c", "DelaySeconds": 20}),
        counts("q"),
        tick(10),
        counts("q"),
        tick(10),
        counts("q"),
    ]);
    assert_eq!(
        r[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(
        r[6]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        r[8]["Counts"],
        json!({"Visible": 3, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_sequence_numbers_and_duplicates() {
    let r = run_cmds(vec![
        create("f.fifo", json!({"Fifo": true})),
        create(
            "g.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        fifo_send("f.fifo", "a", "g1", Some("d1")),
        fifo_send("f.fifo", "b", "g2", Some("d2")),
        fifo_send("f.fifo", "c", "g3", Some("d1")), // duplicate, other group and body
        send("g.fifo", "x"),
        fifo_send("g.fifo", "x", "g", None),
        fifo_send("g.fifo", "y", "g", None),
        fifo_send("g.fifo", "x", "other", None), // duplicate by body, different group
        fifo_send("g.fifo", "z", "g", Some("x")), // explicit id equal to a body
    ]);
    assert_eq!(r[2]["MessageId"], "m-1");
    assert_eq!(
        (r[3]["MessageId"].clone(), r[3]["SequenceNumber"].clone()),
        (json!("m-2"), json!(2))
    );
    assert_eq!(
        (r[4]["MessageId"].clone(), r[4]["SequenceNumber"].clone()),
        (json!("m-1"), json!(1))
    );
    assert_eq!(code(&r[5]), "MissingParameter");
    assert_eq!(r[6]["MessageId"], "m-3");
    assert_eq!(r[7]["SequenceNumber"], 2);
    assert_eq!(
        (r[8]["MessageId"].clone(), r[8]["SequenceNumber"].clone()),
        (json!("m-3"), json!(1))
    );
    assert_eq!(
        (r[9]["MessageId"].clone(), r[9]["SequenceNumber"].clone()),
        (json!("m-3"), json!(1))
    );
}

#[test]
fn duplicate_window_is_300_seconds_and_not_extended() {
    let r = run_cmds(vec![
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g", Some("d")), // t = 0
        tick(299),
        fifo_send("f.fifo", "a", "g", Some("d")), // duplicate; must not extend
        tick(1),
        fifo_send("f.fifo", "a", "g", Some("d")), // t = 300: new message
        tick(299),
        fifo_send("f.fifo", "a", "g", Some("d")), // duplicate of the second
        tick(1),
        fifo_send("f.fifo", "a", "g", Some("d")),
    ]);
    assert_eq!(r[3]["MessageId"], "m-1");
    assert_eq!(r[5]["MessageId"], "m-2");
    assert_eq!(r[5]["SequenceNumber"], 2);
    assert_eq!(r[7]["MessageId"], "m-2");
    assert_eq!(r[9]["MessageId"], "m-3");
}

#[test]
fn deduplication_survives_purge_but_not_queue_deletion() {
    let r = run_cmds(vec![
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g", Some("d")),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        fifo_send("f.fifo", "a", "g", Some("d")),
        counts("f.fifo"),
        fifo_send("f.fifo", "b", "g", Some("e")),
        json!({"Command": "DeleteQueue", "QueueName": "f.fifo"}),
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g", Some("d")),
    ]);
    assert_eq!(r[3]["MessageId"], "m-1");
    assert_eq!(r[4]["Counts"]["Visible"], 0);
    assert_eq!(r[5]["SequenceNumber"], 2); // numbering carries on after a purge
    assert_eq!(
        (r[8]["MessageId"].clone(), r[8]["SequenceNumber"].clone()),
        (json!("m-3"), json!(1))
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut cmds = vec![create("q", json!({}))];
    cmds.extend((0..100_000).map(|_| send("q", "x")));
    cmds.push(send("q", "x"));
    cmds.push(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    cmds.push(send("q", "x"));
    let r = run_cmds(cmds);
    assert!(r[1..=100_000].iter().all(|r| r["ok"] == true));
    assert_eq!(code(&r[100_001]), "LimitExceeded");
    assert_eq!(r[100_003]["MessageId"], "m-100001"); // the failed send used no number
}

#[test]
fn messages_expire_after_retention() {
    let r = run_cmds(vec![
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(59),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(r[3]["Counts"]["Visible"], 1);
    assert_eq!(r[5]["Counts"]["Visible"], 0);
}
