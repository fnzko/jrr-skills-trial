//! The line protocol of section 1, driven through `serve` with byte input.

use miniq::{serve, Broker};
use serde_json::Value;

/// Runs `input` through a fresh broker and returns the raw output bytes.
fn run(input: &[u8]) -> Vec<u8> {
    let mut broker = Broker::new();
    let mut output = Vec::new();
    serve(input, &mut output, &mut broker).expect("in-memory I/O does not fail");
    output
}

/// Runs `input` and returns the response lines, parsed.
fn responses(input: &[u8]) -> Vec<Value> {
    let output = run(input);
    let text = String::from_utf8(output).expect("responses are UTF-8");
    text.split_inclusive('\n')
        .map(|line| {
            assert!(
                line.ends_with('\n'),
                "every response ends with a newline: {line:?}"
            );
            assert_eq!(
                line.matches('\n').count(),
                1,
                "one response per line: {line:?}"
            );
            serde_json::from_str(line).expect("each response is one JSON object")
        })
        .collect()
}

#[test]
fn each_line_gets_exactly_one_response_in_input_order() {
    let input =
        b"{\"Command\": \"Tick\", \"Seconds\": 2}\n{\"Command\": \"Tick\", \"Seconds\": 3}\n";
    let responses = responses(input);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["Now"], 2);
    assert_eq!(responses[1]["Now"], 5);
}

#[test]
fn a_final_line_without_a_newline_counts() {
    let responses = responses(br#"{"Command": "Tick", "Seconds": 1}"#);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["Now"], 1);
}

#[test]
fn an_empty_input_or_a_trailing_newline_alone_gives_no_extra_response() {
    assert!(run(b"").is_empty());
    assert_eq!(
        responses(b"{\"Command\": \"Tick\", \"Seconds\": 0}\n").len(),
        1
    );
}

#[test]
fn blank_lines_are_commands_and_get_invalid_request() {
    let responses = responses(b"\n\n");
    assert_eq!(responses.len(), 2);
    for response in responses {
        assert_eq!(response["error"]["code"], "InvalidRequest");
        assert_eq!(response["ok"], false);
    }
}

#[test]
fn a_bad_line_does_not_stop_the_loop() {
    let input = b"not json\n{\"Command\": \"Tick\", \"Seconds\": 4}\n";
    let responses = responses(input);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[1]["Now"], 4);
}

#[test]
fn invalid_utf8_is_an_invalid_request() {
    let responses = responses(b"{\"Command\": \"\xff\"}\n");
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn carriage_return_line_endings_are_accepted() {
    let responses = responses(b"{\"Command\": \"Tick\", \"Seconds\": 7}\r\n");
    assert_eq!(responses[0]["Now"], 7);
}

#[test]
fn responses_use_the_documented_shapes() {
    let input = concat!(
        "{\"Command\": \"CreateQueue\", \"QueueName\": \"jobs\"}\n",
        "{\"Command\": \"CreateQueue\", \"QueueName\": \"jobs\", \"Attributes\": {\"DelaySeconds\": 1}}\n",
        "{\"Command\": \"SendMessage\", \"QueueName\": \"jobs\", \"Body\": \"x\"}\n",
    );
    let responses = responses(input.as_bytes());
    assert_eq!(responses[0], serde_json::json!({"ok": true}));
    assert_eq!(responses[1]["ok"], false);
    assert_eq!(responses[1]["error"]["code"], "QueueAlreadyExists");
    assert_eq!(
        responses[2],
        serde_json::json!({"ok": true, "MessageId": "m-1"})
    );
}

#[test]
fn a_session_with_several_commands_keeps_its_state_across_lines() {
    let input = concat!(
        "{\"Command\": \"CreateQueue\", \"QueueName\": \"jobs\", \"Attributes\": {\"VisibilityTimeout\": 5}}\n",
        "{\"Command\": \"SendMessage\", \"QueueName\": \"jobs\", \"Body\": \"hello\"}\n",
        "{\"Command\": \"ReceiveMessage\", \"QueueName\": \"jobs\"}\n",
        "{\"Command\": \"GetQueueAttributes\", \"QueueName\": \"jobs\"}\n",
    );
    let responses = responses(input.as_bytes());
    assert_eq!(responses[2]["Messages"][0]["Body"], "hello");
    assert_eq!(
        responses[3]["Counts"],
        serde_json::json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}
