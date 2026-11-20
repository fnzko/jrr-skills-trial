mod common;

use common::Session;

#[test]
fn malformed_lines_are_invalid_requests() {
    let mut s = Session::new();
    let lines = [
        "",
        "   ",
        "not json",
        "[]",
        "\"Command\"",
        "{}",
        r#"{"Command": 7}"#,
        r#"{"Command": null}"#,
        r#"{"command": "Tick", "Seconds": 1}"#,
        r#"{"Command": "Nope"}"#,
        r#"{"Command": "tick", "Seconds": 1}"#,
        r#"{"Command": "Tick"} trailing"#,
    ];
    for line in lines {
        assert_eq!(s.err(line), "InvalidRequest", "line {line:?}");
    }
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let mut s = Session::new();
    let response = s.send_bytes(b"{\"Command\": \"Tick\", \"Seconds\": \"\xff\"}");
    assert_eq!(response["error"]["code"], "InvalidRequest");
    assert_eq!(s.err("{}"), "InvalidRequest");
}
