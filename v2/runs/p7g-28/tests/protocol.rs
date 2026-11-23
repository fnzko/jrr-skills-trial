mod common;

use common::{run, run_bytes};

#[test]
fn every_line_gets_one_response_in_order() {
    let out = run(&[
        r#"{"Command":"Tick","Seconds":3}"#,
        "",
        "not json",
        r#"{"Command":"Tick","Seconds":4}"#,
    ]);
    assert_eq!(out.status, 0);
    assert_eq!(out.raw.lines().count(), 4);
    assert_eq!(out.responses[0]["Now"], 3);
    assert_eq!(out.responses[1]["error"]["code"], "InvalidRequest");
    assert_eq!(out.responses[2]["error"]["code"], "InvalidRequest");
    assert_eq!(out.responses[3]["Now"], 7);
}

#[test]
fn final_line_without_newline_counts_only_if_not_empty() {
    let out = run_bytes(br#"{"Command":"Tick","Seconds":1}"#);
    assert_eq!(out.responses.len(), 1);
    let out = run_bytes(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(out.responses.len(), 1);
    assert!(run_bytes(b"").raw.is_empty());
}

#[test]
fn invalid_utf8_is_an_invalid_request() {
    let out = run_bytes(b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(out.responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(out.responses[1]["Now"], 1);
}
