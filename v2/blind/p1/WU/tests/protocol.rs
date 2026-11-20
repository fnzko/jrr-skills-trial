mod common;

use common::{assert_error, run, run_bytes, session};

#[test]
fn every_malformed_line_gets_an_invalid_request() {
    let lines = [
        "",
        "   ",
        "not json",
        "[]",
        "42",
        "\"Command\"",
        "{}",
        r#"{"Command": 1}"#,
        r#"{"Command": null}"#,
        r#"{"command": "Tick"}"#,
        r#"{"Command": "NoSuchCommand"}"#,
        r#"{"Command": "tick", "Seconds": 1}"#,
        r#"{"Command": "Tick"} trailing"#,
    ];
    for line in lines {
        let responses = session(&[line]);
        assert_error(&responses[0], "InvalidRequest");
    }
}

#[test]
fn one_response_per_line_in_order_and_final_line_without_newline_counts() {
    let responses = run("{}\n\n[1]");
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_error(response, "InvalidRequest");
    }
}

#[test]
fn input_ending_with_newline_has_no_extra_line() {
    assert_eq!(run("{}\n").len(), 1);
    assert_eq!(run("").len(), 0);
}

#[test]
fn invalid_utf8_line_fails_and_processing_goes_on() {
    let responses = run_bytes(b"{\"Command\": \"\xff\"}\n{}\n");
    assert_eq!(responses.len(), 2);
    assert_error(&responses[0], "InvalidRequest");
    assert_error(&responses[1], "InvalidRequest");
}
