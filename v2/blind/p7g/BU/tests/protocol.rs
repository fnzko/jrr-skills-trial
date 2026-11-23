mod common;

use common::{error_code, run, run_raw};
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":5}"#,
        "",
        r#"{"Command":"Tick","Seconds":7}"#,
    ]);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(responses[2], json!({"ok": true, "Now": 12}));
}

#[test]
fn final_line_without_newline_counts() {
    let responses = run_raw(br#"{"Command":"Tick","Seconds":1}"#);
    assert_eq!(responses, vec![json!({"ok": true, "Now": 1})]);
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_raw(b""), Vec::<serde_json::Value>::new());
}

#[test]
fn malformed_lines_are_invalid_requests() {
    let cases: &[&[u8]] = &[
        b"not json",
        b"[1,2]",
        b"{}",
        br#"{"Command":5}"#,
        br#"{"Command":null,"Seconds":1}"#,
        br#"{"Command":"Nope"}"#,
        br#"{"command":"Tick","Seconds":1}"#,
        br#"{"Command":"Tick","Seconds":1} trailing"#,
        b"{\"Command\":\"Tick\",\"Seconds\":1,\"X\":\"\xff\"}",
        b"\xff",
    ];
    for case in cases {
        let mut input = case.to_vec();
        input.push(b'\n');
        input.extend_from_slice(br#"{"Command":"Tick","Seconds":1}"#);
        let responses = run_raw(&input);
        assert_eq!(error_code(&responses[0]), "InvalidRequest", "{case:?}");
        assert_eq!(responses[1], json!({"ok": true, "Now": 1}), "{case:?}");
    }
}

#[test]
fn tick_validates_its_parameter() {
    let cases = [
        (r#"{"Command":"Tick"}"#, "MissingParameter"),
        (r#"{"Command":"Tick","Seconds":null}"#, "MissingParameter"),
        (
            r#"{"Command":"Tick","Seconds":"1"}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":1.0}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":1e0}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":-1}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":1209601}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":99999999999999999999}"#,
            "InvalidParameterValue",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(error_code(&run(&[line])[0]), code, "{line}");
    }
}

#[test]
fn tick_accepts_the_bounds_and_ignores_unknown_members() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":0,"Extra":true}"#,
        r#"{"Command":"Tick","Seconds":1209600}"#,
        r#"{"Command":"Tick","Seconds":-0}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 0}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 1209600}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 1209600}));
}
