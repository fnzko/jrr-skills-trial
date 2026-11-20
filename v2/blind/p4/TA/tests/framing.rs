mod common;

use common::{run, run_bytes};

#[test]
fn blank_line_is_one_invalid_request() {
    let responses = run("\n");

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["ok"], false);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn empty_input_produces_no_output() {
    assert!(run("").is_empty());
}

#[test]
fn final_line_without_newline_is_still_a_command() {
    let responses = run("{\"Command\": \"Nope\"}");

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn every_line_gets_a_response_even_when_blank_or_malformed() {
    let responses = run("{\"Command\": \"First\"}\n\n{\"Command\": \"Third\"}\n");

    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["error"]["code"], "InvalidRequest");
    }
}

#[test]
fn invalid_utf8_line_is_rejected_and_processing_continues() {
    let input = b"\xff\xfe\n{\"Command\": \"Nope\"}\n";

    let responses = run_bytes(input);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], false);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[1]["error"]["code"], "InvalidRequest");
}

#[test]
fn line_that_is_not_json_is_invalid_request() {
    let responses = run("{\"Command\": \"Nope\"\n");

    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn trailing_text_after_the_object_is_invalid_request() {
    let responses = run("{\"Command\": \"Nope\"} extra\n");

    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn json_that_is_not_an_object_is_invalid_request() {
    let responses = run("[1, 2]\n\"Command\"\n42\nnull\n");

    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "InvalidRequest");
    }
}

#[test]
fn missing_command_member_is_invalid_request() {
    let responses = run("{\"QueueName\": \"jobs\"}\n");

    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn non_string_command_member_is_invalid_request() {
    let responses = run("{\"Command\": 7}\n{\"Command\": null}\n");

    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[1]["error"]["code"], "InvalidRequest");
}

#[test]
fn command_names_are_case_sensitive() {
    let responses = run("{\"Command\": \"listqueues\"}\n");

    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
}

#[test]
fn failure_response_has_exactly_ok_and_error_with_code_and_message() {
    let responses = run("{\"Command\": \"Nope\"}\n");

    let response = responses[0].as_object().unwrap();
    assert_eq!(response.len(), 2);
    let error = response["error"].as_object().unwrap();
    assert_eq!(error["code"], "InvalidRequest");
    assert!(error["message"].is_string());
}
