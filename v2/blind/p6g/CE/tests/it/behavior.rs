use serde_json::{Value, json};

use crate::common::{code, counts, create, ok, receive, run, send, tick};

fn message(
    id: &str,
    handle: &str,
    body: &str,
    receive_count: u32,
    sent_at: u32,
    first_received_at: u32,
) -> Value {
    json!({
        "MessageId": id, "ReceiptHandle": handle, "Body": body,
        "ReceiveCount": receive_count, "SentAt": sent_at, "FirstReceivedAt": first_received_at,
    })
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn messages(list: Vec<Value>) -> Value {
    json!({"ok": true, "Messages": list})
}

fn counts_of(visible: u32, in_flight: u32, delayed: u32) -> Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}

#[test]
fn a_message_goes_from_delayed_to_in_flight_to_visible_again() {
    let responses = run(&[
        create("jobs", json!({"VisibilityTimeout": 10, "DelaySeconds": 5})),
        send("jobs", "a"),
        receive("jobs"),
        tick(5),
        receive("jobs"),
        tick(9),
        receive("jobs"),
        tick(1),
        receive("jobs"),
        delete("jobs", "r-1"),
        delete("jobs", "r-2"),
        delete("jobs", "r-2"),
        delete("jobs", "r-3"),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], messages(vec![]));
    assert_eq!(
        responses[4],
        messages(vec![message("m-1", "r-1", "a", 1, 0, 5)])
    );
    assert_eq!(responses[6], messages(vec![]));
    assert_eq!(
        responses[8],
        messages(vec![message("m-1", "r-2", "a", 2, 0, 5)])
    );
    assert_eq!(code(&responses[9]), "ReceiptHandleExpired");
    assert_eq!(responses[10], ok());
    assert_eq!(code(&responses[11]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[12]), "ReceiptHandleInvalid");
}

#[test]
fn counts_follow_the_message_states() {
    let responses = run(&[
        create("q", json!({"DelaySeconds": 10, "VisibilityTimeout": 20})),
        send("q", "a"),
        counts("q"),
        tick(10),
        counts("q"),
        receive("q"),
        counts("q"),
    ]);
    assert_eq!(responses[2]["Counts"], counts_of(0, 0, 1));
    assert_eq!(responses[4]["Counts"], counts_of(1, 0, 0));
    assert_eq!(responses[6]["Counts"], counts_of(0, 1, 0));
}

#[test]
fn get_queue_attributes_lists_exactly_the_set_attributes() {
    let responses = run(&[
        create("d", json!({})),
        create(
            "q",
            json!({"RetentionSeconds": 100, "DeadLetterQueue": "d", "MaxReceiveCount": 4}),
        ),
        counts("q"),
        counts("d"),
    ]);
    assert_eq!(
        responses[2],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 100,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "d", "MaxReceiveCount": 4,
            },
            "Counts": counts_of(0, 0, 0),
        })
    );
    assert_eq!(
        responses[3]["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
        })
    );
}

#[test]
fn creating_an_identical_queue_again_succeeds() {
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 7})),
        create("q", json!({"VisibilityTimeout": 7, "DelaySeconds": 0})),
        create("q", json!({})),
    ]);
    assert_eq!(responses[1], ok());
    assert_eq!(code(&responses[2]), "QueueAlreadyExists");
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        receive("q"),
    ]);
    assert_eq!(
        responses[2],
        messages(vec![message("m-1", "r-1", "a", 1, 0, 0)])
    );
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(
        responses[4],
        messages(vec![message("m-1", "r-2", "a", 2, 0, 0)])
    );
}

#[test]
fn change_message_visibility_moves_the_end_of_the_timeout() {
    let change = |handle: &str, seconds: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": seconds});
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        tick(10),
        change("r-1", 5),
        tick(4),
        receive("q"),
        tick(1),
        receive("q"),
        change("r-2", 0),
        receive("q"),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[6], messages(vec![]));
    assert_eq!(
        responses[8],
        messages(vec![message("m-1", "r-2", "a", 2, 0, 0)])
    );
    assert_eq!(responses[9], ok());
    assert_eq!(
        responses[10],
        messages(vec![message("m-1", "r-3", "a", 3, 0, 0)])
    );
}

#[test]
fn handles_of_another_queue_are_expired_not_invalid() {
    let responses = run(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "x"),
        receive("a"),
        delete("b", "r-1"),
        delete("a", "r-01"),
        delete("a", "R-1"),
        delete("a", "r-1 "),
    ]);
    assert_eq!(code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[5]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[6]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[7]), "ReceiptHandleInvalid");
}

#[test]
fn max_messages_limits_a_batch_in_arrival_order() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}),
    ]);
    assert_eq!(
        responses[4],
        messages(vec![
            message("m-1", "r-1", "a", 1, 0, 0),
            message("m-2", "r-2", "b", 1, 0, 0),
        ])
    );
}

#[test]
fn retention_removes_messages_at_the_boundary() {
    let responses = run(&[
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(59),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(responses[3]["Counts"], counts_of(1, 0, 0));
    assert_eq!(responses[5]["Counts"], counts_of(0, 0, 0));
}

#[test]
fn lowering_retention_removes_at_once_and_raising_does_not_restore() {
    let set = |seconds: u32| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    let responses = run(&[
        create("q", json!({})),
        send("q", "old"),
        tick(100),
        send("q", "new"),
        set(60),
        set(600),
        counts("q"),
        receive("q"),
    ]);
    assert_eq!(responses[6]["Counts"], counts_of(1, 0, 0));
    assert_eq!(
        responses[7],
        messages(vec![message("m-2", "r-1", "new", 1, 100, 100)])
    );
}

#[test]
fn purge_empties_the_queue_but_keeps_numbering() {
    let responses = run(&[
        create(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a", "GroupId": "g"}),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "b", "GroupId": "g"}),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

fn fifo_send(body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn fifo_message(
    id: &str,
    handle: &str,
    body: &str,
    receive_count: u32,
    group: &str,
    sequence: u32,
) -> Value {
    let mut base = message(id, handle, body, receive_count, 0, 0);
    base["GroupId"] = json!(group);
    base["SequenceNumber"] = json!(sequence);
    base
}

#[test]
fn fifo_delivers_one_message_per_group_and_holds_back_busy_groups() {
    let responses = run(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("a", "g1", "d1"),
        fifo_send("b", "g1", "d2"),
        fifo_send("c", "g2", "d3"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        delete("f.fifo", "r-1"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
    ]);
    assert_eq!(
        responses[4],
        messages(vec![
            fifo_message("m-1", "r-1", "a", 1, "g1", 1),
            fifo_message("m-3", "r-2", "c", 1, "g2", 3),
        ])
    );
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(
        responses[7],
        messages(vec![fifo_message("m-2", "r-3", "b", 1, "g1", 2)])
    );
}

#[test]
fn fifo_duplicate_window_is_300_seconds_and_not_extended_by_duplicates() {
    let responses = run(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("a", "g1", "d"),
        tick(299),
        fifo_send("other", "g2", "d"),
        tick(1),
        fifo_send("again", "g1", "d"),
        tick(299),
        fifo_send("dup", "g1", "d"),
    ]);
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    assert_eq!(responses[1], first);
    assert_eq!(responses[3], first);
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_ids_outlive_their_message() {
    let responses = run(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("a", "g", "d"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        fifo_send("b", "g", "d"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let body_send = |body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
    let responses = run(&[
        create(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        body_send("same"),
        body_send("same"),
        body_send("different"),
    ]);
    assert_eq!(responses[2], responses[1]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receive_count() {
    let responses = run(&[
        create("dead", json!({})),
        create(
            "main",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        send("main", "x"),
        receive("main"),
        receive("main"),
        receive("main"),
        counts("main"),
        counts("dead"),
        receive("dead"),
    ]);
    assert_eq!(
        responses[3],
        messages(vec![message("m-1", "r-1", "x", 1, 0, 0)])
    );
    assert_eq!(
        responses[4],
        messages(vec![message("m-1", "r-2", "x", 2, 0, 0)])
    );
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(responses[6]["Counts"], counts_of(0, 0, 0));
    assert_eq!(responses[7]["Counts"], counts_of(1, 0, 0));
    assert_eq!(
        responses[8],
        messages(vec![message("m-1", "r-3", "x", 1, 0, 0)])
    );
}

#[test]
fn a_dead_lettered_message_too_old_for_the_target_is_dropped() {
    let responses = run(&[
        create("dead", json!({"RetentionSeconds": 60})),
        create(
            "main",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("main", "x"),
        receive("main"),
        tick(60),
        receive("main"),
        counts("dead"),
    ]);
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(responses[6]["Counts"], counts_of(0, 0, 0));
}

#[test]
fn dead_lettering_a_fifo_message_frees_its_group() {
    let send_to_main = |body: &str, dedup: &str| json!({"Command": "SendMessage", "QueueName": "main.fifo", "Body": body, "GroupId": "g", "DeduplicationId": dedup});
    let responses = run(&[
        create("dead.fifo", json!({"Fifo": true})),
        create(
            "main.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        send_to_main("first", "d1"),
        send_to_main("second", "d2"),
        receive("main.fifo"),
        receive("main.fifo"),
        json!({"Command": "ReceiveMessage", "QueueName": "dead.fifo"}),
    ]);
    assert_eq!(responses[4]["Messages"][0]["Body"], json!("first"));
    assert_eq!(responses[5]["Messages"][0]["Body"], json!("second"));
    assert_eq!(responses[6]["Messages"][0]["Body"], json!("first"));
    assert_eq!(responses[6]["Messages"][0]["SequenceNumber"], json!(1));
}

#[test]
fn deleting_a_queue_clears_dead_letter_references_to_it() {
    let responses = run(&[
        create("dead", json!({})),
        create(
            "main",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        create("dead", json!({})),
        counts("main"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    ]);
    assert_eq!(responses[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[4]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(code(&responses[6]), "QueueDoesNotExist");
}

#[test]
fn set_queue_attributes_replaces_only_what_is_given() {
    let responses = run(&[
        create("d", json!({})),
        create("q", json!({"DelaySeconds": 3})),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"VisibilityTimeout": 9, "DeadLetterQueue": "d", "MaxReceiveCount": 5}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}),
        counts("q"),
    ]);
    assert_eq!(
        responses[4]["Attributes"],
        json!({
            "VisibilityTimeout": 9, "DelaySeconds": 3, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "d", "MaxReceiveCount": 5,
        })
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a", "_x", "A", "a-1", "a.fifo", "a1"];
    let mut commands: Vec<Value> = names
        .iter()
        .map(|name| {
            let attributes = if name.ends_with(".fifo") {
                json!({"Fifo": true})
            } else {
                json!({})
            };
            create(name, attributes)
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let responses = run(&commands);
    let n = names.len();
    assert_eq!(
        responses[n],
        json!({"ok": true, "QueueNames": ["A", "_x", "a", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        responses[n + 1],
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1"]})
    );
    assert_eq!(responses[n + 2], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let responses = run(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "1"),
        send("nope", "2"),
        send("b", "3"),
    ]);
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[4]["MessageId"], json!("m-2"));
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    let responses = crate::common::run_bytes(input.as_bytes());
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
}

#[test]
fn at_most_1000_queues_exist() {
    let mut commands: Vec<Value> = (0..1000)
        .map(|n| create(&format!("q{n}"), json!({})))
        .collect();
    commands.push(create("one-too-many", json!({})));
    commands.push(create("q0", json!({})));
    let responses = run(&commands);
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
}
