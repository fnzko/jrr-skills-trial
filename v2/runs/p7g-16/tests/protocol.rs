mod common;

use common::Session;
use miniq::protocol::serve;
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let input = b"{\"Command\":\"Tick\",\"Seconds\":5}\n\n{\"Command\":\"Tick\",\"Seconds\":2}";
    let mut out = Vec::new();
    serve(&input[..], &mut out).unwrap();
    let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"Now\":5"));
    assert!(lines[1].contains("InvalidRequest"));
    assert!(lines[2].contains("\"Now\":7"));
    assert!(out.ends_with(b"\n"));
}

#[test]
fn empty_trailing_line_is_not_a_command() {
    let mut out = Vec::new();
    serve(&b"{\"Command\":\"Tick\",\"Seconds\":1}\n"[..], &mut out).unwrap();
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_service_continues() {
    let mut input = b"{\"Command\":\"Tick\",\"Seconds\":1,\"x\":\"\xff\"}\n".to_vec();
    input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    let mut out = Vec::new();
    serve(&input[..], &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("InvalidRequest"));
    assert!(lines[1].contains("\"Now\":1"));
}

#[test]
fn request_shape_errors() {
    let mut s = Session::new();
    for line in [
        "",
        "not json",
        "[1]",
        "\"Tick\"",
        "{}",
        "{\"Command\": 5}",
        "{\"Command\": null}",
        "{\"Command\": \"tick\"}",
        "{\"Command\": \"Nope\"}",
        "{\"Command\":\"Tick\",\"Seconds\":1} trailing",
    ] {
        let r = s.raw(line);
        assert_eq!(r["error"]["code"], "InvalidRequest", "line {line:?}");
    }
}

#[test]
fn unknown_members_are_ignored_and_null_is_absent() {
    let mut s = Session::new();
    s.ok(json!({"Command": "Tick", "Seconds": 1, "Extra": [1, 2]}));
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
}

#[test]
fn tick_validates_and_advances() {
    let mut s = Session::new();
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 0);
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209600
    );
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
    for bad in [
        json!(-1),
        json!(1209601),
        json!(1.5),
        json!("3"),
        json!(true),
    ] {
        assert_eq!(
            s.err(json!({"Command": "Tick", "Seconds": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": 1.0e0})),
        "InvalidParameterValue"
    );
    let r = s.raw("{\"Command\":\"Tick\",\"Seconds\":3e0}");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    let r = s.raw("{\"Command\":\"Tick\",\"Seconds\":1e999}");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}
