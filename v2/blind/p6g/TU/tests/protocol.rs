mod common;

use common::*;
use serde_json::json;

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let responses = run_raw(
        b"\n{\"Command\":\"Tick\",\"Seconds\":5}\n   \n{\"Command\":\"Tick\",\"Seconds\":2}",
    );
    assert_eq!(responses.len(), 4);
    assert_eq!(code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 5}));
    assert_eq!(code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 7}));
}

#[test]
fn input_without_a_final_line_ends_cleanly() {
    assert_eq!(run_raw(b""), Vec::<serde_json::Value>::new());
    assert_eq!(run_raw(b"\n").len(), 1);
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_processing_continues() {
    let responses = run_raw(b"{\"Command\":\"Tick\xff\"}\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn crlf_line_endings_are_json_whitespace() {
    let responses = run_raw(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert_eq!(responses, [json!({"ok": true, "Now": 1})]);
}

#[test]
fn tick_advances_the_clock_and_validates_its_range() {
    let responses = run(&[
        tick(0),
        tick(1_209_600),
        tick(1_209_601),
        cmd("Tick", json!({})),
        cmd("Tick", json!({"Seconds": 1.5})),
        cmd("Tick", json!({"Seconds": "3"})),
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 0}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 1_209_600}));
    assert_eq!(code(&responses[2]), "InvalidParameterValue");
    assert_eq!(code(&responses[3]), "MissingParameter");
    assert_eq!(code(&responses[4]), "InvalidParameterValue");
    assert_eq!(code(&responses[5]), "InvalidParameterValue");
}

#[test]
fn error_precedence_runs_shape_required_values_lookup_rules() {
    let responses = run(&[
        // Missing Body beats the bad QueueName type and the missing queue.
        cmd("SendMessage", json!({"QueueName": 5})),
        // A bad value beats the missing queue.
        cmd("SendMessage", json!({"QueueName": "nope", "Body": 5})),
        // The missing queue beats the command rules (FIFO needs GroupId).
        cmd(
            "SendMessage",
            json!({"QueueName": "nope.fifo", "Body": "x"}),
        ),
        // Lookup is by exact string, whatever it looks like.
        cmd("PurgeQueue", json!({"QueueName": "not a valid name!"})),
        cmd("Nope", json!({})),
    ]);
    let codes: Vec<_> = responses.iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "InvalidRequest"
        ]
    );
}
