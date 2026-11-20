//! Runs the real `miniq` executable to check the stdin/stdout framing.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn run(input: &[u8]) -> (Vec<Value>, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for miniq");
    let text = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "unterminated output {text:?}"
    );
    let replies = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    (replies, output.status.code().expect("exited normally"))
}

#[test]
fn every_line_gets_one_reply_including_blank_and_unterminated_ones() {
    let (replies, status) =
        run(b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}");
    assert_eq!(status, 0);
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0], json!({"ok": true, "Now": 2}));
    assert_eq!(replies[1]["error"]["code"], json!("InvalidRequest"));
    assert_eq!(replies[2], json!({"ok": true, "Now": 5}));
}

#[test]
fn empty_input_exits_cleanly_with_no_output() {
    assert_eq!(run(b""), (vec![], 0));
}

#[test]
fn trailing_newline_does_not_add_a_line() {
    let (replies, status) = run(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!((replies, status), (vec![json!({"ok": true, "Now": 1})], 0));
}
