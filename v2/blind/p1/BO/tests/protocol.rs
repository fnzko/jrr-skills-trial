mod common;

use common::Session;
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let input =
        b"{\"Command\":\"Tick\",\"Seconds\":5}\n\nnot json\n{\"Command\":\"Tick\",\"Seconds\":2}";
    let mut output = Vec::new();
    miniq::run(&input[..], &mut output).unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], json!({"ok": true, "Now": 5}));
    assert_eq!(lines[1]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[2]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[3], json!({"ok": true, "Now": 7}));
}

#[test]
fn empty_input_and_trailing_newline_produce_no_extra_response() {
    let mut output = Vec::new();
    miniq::run(&b""[..], &mut output).unwrap();
    assert!(output.is_empty());
    miniq::run(&b"{\"Command\":\"Tick\",\"Seconds\":1}\n"[..], &mut output).unwrap();
    assert_eq!(output.iter().filter(|&&b| b == b'\n').count(), 1);
}

#[test]
fn malformed_requests_are_invalid_request() {
    let mut s = Session::new();
    for line in [
        "",
        "[]",
        "42",
        "{",
        "{\"Command\":1}",
        "{}",
        "{\"Command\":\"Nope\"}",
        "{\"command\":\"Tick\"}",
    ] {
        assert_eq!(
            s.raw(line)["error"]["code"],
            "InvalidRequest",
            "line {line:?}"
        );
    }
    let response = s.broker_raw_bytes(b"{\"Command\":\"Tick\",\"Seconds\":1,\"x\":\"\xff\"}");
    assert_eq!(response["error"]["code"], "InvalidRequest");
}

#[test]
fn tick_validates_and_accumulates() {
    let mut s = Session::new();
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    for bad in [json!(-1), json!(1209601), json!("3"), json!(true)] {
        assert_eq!(
            s.err(json!({"Command": "Tick", "Seconds": bad})),
            "InvalidParameterValue"
        );
    }
    for bad in ["30.0", "3e1", "1E2", "1.5"] {
        let line = format!("{{\"Command\":\"Tick\",\"Seconds\":{bad}}}");
        assert_eq!(
            s.raw(&line)["error"]["code"],
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(s.raw("{\"Command\":\"Tick\",\"Seconds\":-0}")["Now"], 0);
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 10, "Extra": 1}))["Now"],
        10
    );
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209610
    );
}
