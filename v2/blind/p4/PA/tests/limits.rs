//! Section 5 and section 6 limits: 1000 queues, and 100000 messages in one queue.

mod support;

use serde_json::json;
use support::{refused, replies};

#[test]
fn the_broker_refuses_a_1001st_queue_but_still_accepts_an_identical_repeat_of_an_existing_one() {
    let mut lines: Vec<String> = (0..1000)
        .map(|index| format!(r#"{{"Command":"CreateQueue","QueueName":"q{index}"}}"#))
        .collect();
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q1000"}"#.to_owned());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#.to_owned());
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let got = replies(&line_refs);
    assert!(
        got[..1000]
            .iter()
            .all(|reply| *reply == json!({"ok": true})),
        "the first 1000 queues are created"
    );
    assert_eq!(got[1000..], [refused("LimitExceeded"), json!({"ok": true})]);
}

#[test]
fn a_queue_refuses_its_100001st_message_and_the_100000th_is_accepted() {
    let mut lines: Vec<String> = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#.to_owned()];
    lines.extend(
        (0..100_001).map(|_| r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#.to_owned()),
    );
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let got = replies(&line_refs);
    assert_eq!(got[100_000], json!({"ok": true, "MessageId": "m-100000"}));
    assert_eq!(got[100_001], refused("LimitExceeded"));
}
