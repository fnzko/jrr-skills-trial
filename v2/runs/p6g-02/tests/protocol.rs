//! End-to-end tests: script lines in, response lines out.

use serde_json::{Value, json};

fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn run(commands: &[Value]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    run_raw(input.as_bytes())
}

fn ok() -> Value {
    json!({"ok": true})
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

#[test]
fn every_line_gets_one_response_including_bad_and_blank_ones() {
    let responses = run_raw(
        b"\n{\"Command\":\"Tick\",\"Seconds\":5}\nnope\n\xff\n{\"Command\":\"Tick\",\"Seconds\":1}",
    );
    let codes: Vec<_> = responses
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "InvalidRequest",
            "ok",
            "InvalidRequest",
            "InvalidRequest",
            "ok"
        ]
    );
    assert_eq!(responses[4], json!({"ok": true, "Now": 6}));
}

#[test]
fn empty_input_and_final_empty_line_produce_nothing() {
    assert_eq!(run_raw(b"").len(), 0);
    assert_eq!(run_raw(b"{\"Command\":\"Tick\",\"Seconds\":1}\n").len(), 1);
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 10})),
        create("q", json!({"VisibilityTimeout": 10})),
        create("q", json!({})),
        create("a.fifo", json!({})),
        create("b", json!({"Fifo": true})),
        create("c", json!({"ContentBasedDeduplication": true})),
        create("d", json!({"DeadLetterQueue": "q"})),
        create(
            "e",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
        ),
        create(
            "f.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "q", "MaxReceiveCount": 2}),
        ),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], ok());
    let codes: Vec<_> = responses[2..].iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "QueueAlreadyExists",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn queue_attributes_report_defaults_and_counts() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    assert_eq!(
        responses[2],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a_", "A", "a-", "a.fifo", "a1", "ab"];
    let mut commands: Vec<Value> = names
        .iter()
        .map(|n| {
            let attributes = if n.ends_with(".fifo") {
                json!({"Fifo": true})
            } else {
                json!({})
            };
            create(n, attributes)
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let responses = run(&commands);
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["A", "a-", "a.fifo", "a1", "a_", "ab", "b"]})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "a_", "ab"]})
    );
    assert_eq!(responses[9], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn send_then_receive_returns_message_with_ids_and_times() {
    let responses = run(&[
        create("q", json!({})),
        tick(7),
        send("q", "hello"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 7, "FirstReceivedAt": 8
        }]})
    );
}

#[test]
fn visibility_timeout_hides_then_returns_message_in_place() {
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[3]["Messages"][0]["Body"], "a");
    assert_eq!(responses[5]["Messages"][0]["Body"], "b");
    assert_eq!(responses[7]["Messages"][0]["Body"], "a");
    assert_eq!(responses[7]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[7]["Messages"][0]["FirstReceivedAt"], 0);
}

#[test]
fn delayed_messages_become_visible_when_delay_ends() {
    let responses = run(&[
        create("q", json!({"DelaySeconds": 5})),
        send("q", "queue delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 0}),
        receive("q"),
        tick(5),
        receive("q"),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    assert_eq!(responses[3]["Messages"][0]["Body"], "own");
    assert_eq!(responses[5]["Messages"][0]["Body"], "queue delay");
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
}

#[test]
fn receipt_handles_follow_invalid_versus_expired_rules() {
    let delete =
        |h: &str| json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": h});
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 5})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        tick(5),
        receive("q"),
        delete("r-1"),
        delete("r-3"),
        delete("r-01"),
        delete("r-2 "),
        json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-2"}),
        json!({"Command": "DeleteMessage", "QueueName": "missing", "ReceiptHandle": "zzz"}),
        delete("r-2"),
        delete("r-2"),
    ]);
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[7]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[8]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[9]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[10]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[11]), "QueueDoesNotExist");
    assert_eq!(responses[12], ok());
    assert_eq!(code(&responses[13]), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let change = |t: u64| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": t});
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(4),
        change(20),
        tick(19),
        receive("q"),
        tick(1),
        receive("q"),
        change(0),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(code(&responses[9]), "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_makes_handle_unusable() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(responses[2]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
}

#[test]
fn retention_removes_messages_and_lowering_it_acts_at_once() {
    let responses = run(&[
        create("q", json!({"RetentionSeconds": 100})),
        send("q", "a"),
        tick(60),
        send("q", "b"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn standard_queue_rejects_fifo_parameters_and_fifo_rejects_delay() {
    let responses = run(&[
        create("s", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0, "GroupId": "g"}),
        send("f.fifo", "x"),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
    ]);
    assert_eq!(code(&responses[2]), "UnsupportedOperation");
    assert_eq!(code(&responses[3]), "UnsupportedOperation");
    assert_eq!(code(&responses[4]), "MissingParameter");
    assert_eq!(code(&responses[5]), "MissingParameter");
}

#[test]
fn fifo_deduplication_window_does_not_extend_and_ids_are_not_consumed() {
    let dup = |d: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": d});
    let responses = run(&[
        create("f.fifo", json!({"Fifo": true})),
        dup("d"),
        tick(299),
        dup("d"),
        tick(1),
        dup("d"),
        send("f.fifo", "no group"),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "e"}),
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
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn content_based_deduplication_uses_the_body_and_survives_purge() {
    let body = |b: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": b, "GroupId": "g"});
    let responses = run(&[
        create(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        body("same"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        body("same"),
        body("other"),
    ]);
    assert_eq!(responses[3]["MessageId"], "m-1");
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn fifo_receive_delivers_one_message_per_group_and_blocks_busy_groups() {
    let send_to = |g: &str, b: &str, d: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": b, "GroupId": g, "DeduplicationId": d});
    let receive_many =
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10});
    let responses = run(&[
        create("f.fifo", json!({"Fifo": true})),
        send_to("a", "a1", "1"),
        send_to("a", "a2", "2"),
        send_to("b", "b1", "3"),
        receive_many.clone(),
        receive_many,
    ]);
    let bodies = |r: &Value| -> Vec<String> {
        r["Messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["Body"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(bodies(&responses[4]), ["a1", "b1"]);
    assert_eq!(responses[4]["Messages"][0]["GroupId"], "a");
    assert_eq!(responses[4]["Messages"][0]["SequenceNumber"], 1);
    assert_eq!(bodies(&responses[5]), Vec::<String>::new());
}

#[test]
fn messages_past_max_receive_count_move_to_the_dead_letter_queue() {
    let responses = run(&[
        create("dlq", json!({"RetentionSeconds": 100})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        send("q", "a"),
        receive("q"),
        receive("q"),
        receive("q"),
        json!({"Command": "GetQueueAttributes", "QueueName": "dlq"}),
        receive("dlq"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[6]["Counts"]["Visible"], 1);
    let moved = &responses[7]["Messages"][0];
    assert_eq!(moved["MessageId"], "m-1");
    assert_eq!(moved["ReceiveCount"], 1);
    assert_eq!(moved["FirstReceivedAt"], 0);
}

#[test]
fn dead_letter_arrival_is_dropped_when_older_than_target_retention() {
    let responses = run(&[
        create("dlq", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        receive("q"),
        tick(60),
        receive("q"),
        json!({"Command": "GetQueueAttributes", "QueueName": "dlq"}),
    ]);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_of_its_users() {
    let responses = run(&[
        create("dlq", json!({})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
    ]);
    assert_eq!(responses[3]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[3]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(code(&responses[4]), "InvalidParameterValue");
    assert_eq!(code(&responses[5]), "InvalidParameterValue");
    assert_eq!(code(&responses[6]), "UnsupportedOperation");
    assert_eq!(code(&responses[7]), "InvalidParameterValue");
}

#[test]
fn error_precedence_follows_the_spec_order() {
    let responses = run(&[
        json!({"Command": "SendMessage", "Body": 5}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": 5}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"}),
        json!({"Command": "PurgeQueue", "QueueName": "bad name"}),
        json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "x"}),
    ]);
    let codes: Vec<_> = responses.iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut commands = vec![create("q", json!({}))];
    commands.extend((0..100_001).map(|_| send("q", "x")));
    let responses = run(&commands);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
}

#[test]
fn at_most_1000_queues_exist() {
    let mut commands: Vec<Value> = (0..1001)
        .map(|i| create(&format!("q{i}"), json!({})))
        .collect();
    commands.push(create("q0", json!({})));
    let responses = run(&commands);
    assert_eq!(responses[999], ok());
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q", json!({})),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        receive("q"),
    ]);
    assert_eq!(code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(responses[6]["Messages"], json!([]));
}
