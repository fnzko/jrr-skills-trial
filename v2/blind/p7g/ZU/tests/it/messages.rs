use crate::common::{assert_responses, err, ok, run};
use serde_json::json;

const STANDARD: &str = r#"{"Command":"CreateQueue","QueueName":"q"}"#;
const FIFO: &str = r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#;
const FIFO_CBD: &str = r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#;

fn tick(seconds: u32) -> String {
    format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#)
}

fn send(queue: &str, extra: &str) -> String {
    format!(r#"{{"Command":"SendMessage","QueueName":"{queue}","Body":"hello"{extra}}}"#)
}

fn counts(queue: &str) -> String {
    format!(r#"{{"Command":"GetQueueAttributes","QueueName":"{queue}"}}"#)
}

fn counts_reply(visible: u32, in_flight: u32, delayed: u32) -> serde_json::Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}

fn run_owned(lines: &[String]) -> Vec<serde_json::Value> {
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&lines)
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let responses = run(&[
        STANDARD,
        FIFO,
        r#"{"Command":"CreateQueue","QueueName":"d","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":4}}"#,
        &counts("q"),
        &counts("f.fifo"),
        &counts("d"),
        &counts("missing"),
    ]);
    let attributes = |fifo: bool| {
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": fifo, "ContentBasedDeduplication": false,
        })
    };
    let mut with_dead_letter = attributes(false);
    with_dead_letter["DeadLetterQueue"] = json!("q");
    with_dead_letter["MaxReceiveCount"] = json!(4);
    assert_eq!(
        responses[3..],
        [
            json!({"ok": true, "Attributes": attributes(false), "Counts": counts_reply(0, 0, 0)}),
            json!({"ok": true, "Attributes": attributes(true), "Counts": counts_reply(0, 0, 0)}),
            json!({"ok": true, "Attributes": with_dead_letter, "Counts": counts_reply(0, 0, 0)}),
            responses[6].clone(),
        ]
    );
    assert_eq!(responses[6]["error"]["code"], json!("QueueDoesNotExist"));
}

#[test]
fn standard_sends_number_messages_across_queues() {
    assert_responses(
        run(&[
            STANDARD,
            r#"{"Command":"CreateQueue","QueueName":"r"}"#,
            &send("q", ""),
            &send("r", ""),
            &send("q", r#","DelaySeconds":null"#),
        ]),
        vec![
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "MessageId": "m-3"}),
        ],
    );
}

#[test]
fn send_checks_parameters_before_the_queue_and_the_queue_before_the_rules() {
    assert_responses(
        run(&[
            STANDARD,
            FIFO,
            r#"{"Command":"SendMessage","QueueName":"nope"}"#,
            r#"{"Command":"SendMessage","Body":"x"}"#,
            r#"{"Command":"SendMessage","QueueName":"nope","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"nope","Body":"x"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","DelaySeconds":901}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"has space"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","DeduplicationId":7}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DelaySeconds":0,"GroupId":"g","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g"}"#,
        ]),
        vec![
            ok(),
            ok(),
            err("MissingParameter"),
            err("MissingParameter"),
            err("InvalidParameterValue"),
            err("QueueDoesNotExist"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("UnsupportedOperation"),
            err("UnsupportedOperation"),
            err("UnsupportedOperation"),
            err("MissingParameter"),
            err("MissingParameter"),
        ],
    );
}

#[test]
fn body_length_is_counted_in_bytes() {
    let long = "a".repeat(1024);
    let wide = "\u{e9}".repeat(512);
    let too_wide = "\u{e9}".repeat(513);
    let lines: Vec<String> = [&long, &wide, &too_wide]
        .iter()
        .map(|body| format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{body}"}}"#))
        .collect();
    let mut all = vec![STANDARD.to_owned()];
    all.extend(lines);
    assert_responses(
        run_owned(&all),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            err("InvalidParameterValue"),
        ],
    );
}

#[test]
fn fifo_sends_get_sequence_numbers_and_deduplicate() {
    let dedup = |id: &str| {
        send(
            "f.fifo",
            &format!(r#","GroupId":"g","DeduplicationId":"{id}""#),
        )
    };
    assert_responses(
        run_owned(&[
            FIFO.to_owned(),
            dedup("a"),
            dedup("b"),
            dedup("a"),
            dedup("c"),
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ],
    );
}

#[test]
fn deduplication_window_is_300_seconds_and_a_duplicate_does_not_extend_it() {
    let dedup = send("f.fifo", r#","GroupId":"g","DeduplicationId":"a""#);
    assert_responses(
        run_owned(&[
            FIFO.to_owned(),
            dedup.clone(),
            tick(299),
            dedup.clone(),
            tick(1),
            dedup.clone(),
            tick(299),
            dedup.clone(),
            tick(1),
            dedup,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 299}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Now": 599}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Now": 600}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ],
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let by_body = |body: &str, extra: &str| {
        format!(
            r#"{{"Command":"SendMessage","QueueName":"c.fifo","Body":"{body}","GroupId":"g"{extra}}}"#
        )
    };
    assert_responses(
        run_owned(&[
            FIFO_CBD.to_owned(),
            by_body("x", ""),
            by_body("x", ""),
            by_body("y", ""),
            by_body("z", r#","DeduplicationId":"x""#),
            by_body("x", r#","DeduplicationId":"other""#),
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ],
    );
}

#[test]
fn deduplication_ignores_the_group_and_outlives_purge_and_deletion() {
    let send_in = |group: &str| {
        send(
            "f.fifo",
            &format!(r#","GroupId":"{group}","DeduplicationId":"a""#),
        )
    };
    let responses = run_owned(&[
        FIFO.to_owned(),
        send_in("g1"),
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#.to_owned(),
        send_in("g2"),
        counts("f.fifo"),
        r#"{"Command":"DeleteQueue","QueueName":"f.fifo"}"#.to_owned(),
        FIFO.to_owned(),
        send_in("g1"),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[4]["Counts"], counts_reply(0, 0, 0));
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn purge_empties_the_queue_but_sequence_numbers_carry_on() {
    let dedup = |id: &str| {
        send(
            "f.fifo",
            &format!(r#","GroupId":"g","DeduplicationId":"{id}""#),
        )
    };
    let responses = run_owned(&[
        FIFO.to_owned(),
        dedup("a"),
        dedup("b"),
        counts("f.fifo"),
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#.to_owned(),
        counts("f.fifo"),
        dedup("c"),
        r#"{"Command":"PurgeQueue","QueueName":"missing"}"#.to_owned(),
    ]);
    assert_eq!(responses[3]["Counts"], counts_reply(2, 0, 0));
    assert_eq!(responses[4], ok());
    assert_eq!(responses[5]["Counts"], counts_reply(0, 0, 0));
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(responses[7]["error"]["code"], json!("QueueDoesNotExist"));
}

#[test]
fn delayed_messages_become_visible_when_their_delay_ends() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#.to_owned(),
        send("q", ""),
        send("q", r#","DelaySeconds":0"#),
        send("q", r#","DelaySeconds":20"#),
        counts("q"),
        tick(9),
        counts("q"),
        tick(1),
        counts("q"),
        tick(10),
        counts("q"),
    ]);
    let counts_at = |index: usize| responses[index]["Counts"].clone();
    assert_eq!(counts_at(4), counts_reply(1, 0, 2));
    assert_eq!(counts_at(6), counts_reply(1, 0, 2));
    assert_eq!(counts_at(8), counts_reply(2, 0, 1));
    assert_eq!(counts_at(10), counts_reply(3, 0, 0));
}

#[test]
fn a_new_queue_delay_applies_to_later_sends_only() {
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", ""),
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DelaySeconds":5}}"#
            .to_owned(),
        send("q", ""),
        counts("q"),
    ]);
    assert_eq!(responses[4]["Counts"], counts_reply(1, 0, 1));
}

#[test]
fn retention_removes_messages_as_soon_as_it_runs_out() {
    let responses = run_owned(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#
            .to_owned(),
        send("q", ""),
        send("q", r#","DelaySeconds":900"#),
        tick(59),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(responses[4]["Counts"], counts_reply(1, 0, 1));
    assert_eq!(responses[6]["Counts"], counts_reply(0, 0, 0));
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_does_not_bring_messages_back() {
    let set = |seconds: u32| {
        format!(
            r#"{{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{{"RetentionSeconds":{seconds}}}}}"#
        )
    };
    let responses = run_owned(&[
        STANDARD.to_owned(),
        send("q", ""),
        tick(100),
        send("q", ""),
        set(100),
        counts("q"),
        set(1_209_600),
        counts("q"),
        tick(10_000),
        counts("q"),
    ]);
    assert_eq!(responses[5]["Counts"], counts_reply(1, 0, 0));
    assert_eq!(responses[7]["Counts"], counts_reply(1, 0, 0));
    assert_eq!(responses[9]["Counts"], counts_reply(1, 0, 0));
}

#[test]
fn a_queue_holds_at_most_100000_messages_in_any_state() {
    let mut lines = vec![STANDARD.to_owned()];
    lines.extend((0..100_000).map(|_| send("q", "")));
    lines.push(send("q", ""));
    lines.push(send("q", r#","DelaySeconds":5"#));
    lines.push(r#"{"Command":"PurgeQueue","QueueName":"q"}"#.to_owned());
    lines.push(send("q", ""));
    let responses = run_owned(&lines);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_responses(
        responses[100_001..].to_vec(),
        vec![
            err("LimitExceeded"),
            err("LimitExceeded"),
            ok(),
            json!({"ok": true, "MessageId": "m-100001"}),
        ],
    );
}
