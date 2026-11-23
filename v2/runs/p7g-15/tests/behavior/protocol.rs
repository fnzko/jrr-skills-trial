use crate::common::{error_code, session_bytes};
use serde_json::{Value, json};

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let responses = session_bytes(b"\n{}\n[1]");
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["ok"], json!(false));
        assert_eq!(error_code(response), Some("InvalidRequest"));
    }
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(session_bytes(b""), Vec::<Value>::new());
}

#[test]
fn malformed_requests_are_invalid_request() {
    let inputs: [&[u8]; 6] = [
        b"not json\n",
        b"{\"Command\": 5}\n",
        b"{\"Command\": null}\n",
        b"{\"Command\": \"Nope\"}\n",
        b"{\"Command\": \"Tick\"} trailing\n",
        b"{\"Command\": \"\xff\"}\n",
    ];
    for input in inputs {
        let responses = session_bytes(input);
        assert_eq!(responses.len(), 1, "input {input:?}");
        assert_eq!(
            error_code(&responses[0]),
            Some("InvalidRequest"),
            "input {input:?}"
        );
        assert_eq!(responses[0]["ok"], json!(false));
    }
}

#[test]
fn unknown_members_are_ignored_and_null_means_absent() {
    let responses = session_bytes(
        b"{\"Command\": \"Tick\", \"Seconds\": 5, \"Extra\": [1]}\n{\"Command\": \"Tick\", \"Seconds\": null}\n",
    );
    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(error_code(&responses[1]), Some("MissingParameter"));
}
