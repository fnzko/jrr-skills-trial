//! End-to-end check of the `miniq` executable: stdin in, stdout out, exit code 0.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = "{\"Command\":\"CreateQueue\",\"QueueName\":\"jobs\"}\n\
                 {\"Command\":\"ListQueues\"}\nbroken";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "{\"ok\":true}");
    assert!(lines[1].contains("\"QueueNames\":[\"jobs\"]"));
    assert!(lines[2].contains("InvalidRequest"));
}
