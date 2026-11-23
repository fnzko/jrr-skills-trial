mod common;

use common::Session;
use serde_json::json;

#[test]
fn tick_advances_the_clock() {
    let mut s = Session::new();
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 5}))["Now"], 5);
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 5);
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209605
    );
}

#[test]
fn malformed_requests_are_invalid_requests() {
    let mut s = Session::new();
    for line in [
        "",
        "   ",
        "not json",
        "[]",
        "\"Tick\"",
        "null",
        "{}",
        "{\"Command\": 1}",
        "{\"Command\": null}",
        "{\"Command\": \"tick\"}",
        "{\"Command\": \"Nope\"}",
        "{\"Command\": \"Tick\", \"Seconds\": 1} trailing",
    ] {
        let r = s.raw(line);
        assert_eq!(r["error"]["code"], "InvalidRequest", "line {line:?}: {r}");
        assert_eq!(r["ok"], false);
    }
}

#[test]
fn integers_must_be_written_as_integers() {
    let mut s = Session::new();
    for seconds in [
        "1.0", "1e0", "\"1\"", "true", "-0.0", "1.5", "[1]", "-1", "1209601",
    ] {
        let r = s.raw(&format!("{{\"Command\":\"Tick\",\"Seconds\":{seconds}}}"));
        assert_eq!(
            r["error"]["code"], "InvalidParameterValue",
            "{seconds}: {r}"
        );
    }
    assert_eq!(s.raw("{\"Command\":\"Tick\",\"Seconds\":-0}")["Now"], 0);
    // Out-of-range integers too large for 64 bits are still just out of range.
    let r = s.raw("{\"Command\":\"Tick\",\"Seconds\":123456789012345678901234567890}");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn null_means_absent_and_unknown_members_are_ignored() {
    let mut s = Session::new();
    s.err(
        json!({"Command": "Tick", "Seconds": null}),
        "MissingParameter",
    );
    s.err(json!({"Command": "Tick"}), "MissingParameter");
    s.ok(json!({"Command": "Tick", "Seconds": 1, "Extra": [1, 2]}));
}

#[test]
fn run_frames_lines_and_answers_each_one() {
    let input =
        b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":3}";
    let mut out = Vec::new();
    miniq::run(&input[..], &mut out).unwrap();
    let lines: Vec<serde_json::Value> = std::str::from_utf8(&out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["Now"], 2);
    assert_eq!(lines[1]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[2]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[3]["Now"], 5);
}

#[test]
fn empty_input_and_trailing_newline_produce_no_extra_responses() {
    let mut out = Vec::new();
    miniq::run(&b""[..], &mut out).unwrap();
    assert!(out.is_empty());
    miniq::run(&b"{\"Command\":\"Tick\",\"Seconds\":1}\n"[..], &mut out).unwrap();
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
}

#[test]
fn binary_exits_zero_at_end_of_input() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":7}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "{\"Now\":7,\"ok\":true}"
    );
}
