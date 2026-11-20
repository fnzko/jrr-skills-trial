mod common;

use common::Harness;
use serde_json::json;

#[test]
fn tick_advances_the_clock() {
    let mut h = Harness::new();
    assert_eq!(h.ok(json!({"Command": "Tick", "Seconds": 5}))["Now"], 5);
    assert_eq!(h.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 5);
    assert_eq!(
        h.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209605
    );
}

#[test]
fn tick_validates_seconds() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "Tick"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    for bad in [
        json!(-1),
        json!(1209601),
        json!("5"),
        json!(1.5),
        json!(true),
    ] {
        assert_eq!(
            h.err(json!({"Command": "Tick", "Seconds": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.raw(br#"{"Command":"Tick","Seconds":30.0}"#)["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        h.raw(br#"{"Command":"Tick","Seconds":3e1}"#)["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn bad_shapes_are_invalid_requests() {
    let mut h = Harness::new();
    let lines: [&[u8]; 9] = [
        b"",
        b"   ",
        b"garbage",
        b"[1]",
        b"\"Tick\"",
        b"{}",
        br#"{"Command": 7}"#,
        br#"{"Command": "tick", "Seconds": 1}"#,
        b"\xff\xfe",
    ];
    for line in lines {
        let r = h.raw(line);
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"]["code"], "InvalidRequest", "{line:?}");
    }
    // The broker carries on and the clock never moved.
    assert_eq!(h.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 0);
}

#[test]
fn unknown_members_are_ignored() {
    let mut h = Harness::new();
    let r = h.ok(json!({"Command": "Tick", "Seconds": 2, "Extra": [1, 2]}));
    assert_eq!(r, json!({"ok": true, "Now": 2}));
}
