//! End-to-end checks of the stdin/stdout framing, through the real binary.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (Vec<String>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.is_empty() || stdout.ends_with('\n'));
    (
        stdout.lines().map(str::to_owned).collect(),
        output.status.success(),
    )
}

#[test]
fn every_line_gets_one_response_in_order() {
    let (lines, success) = run(
        b"\n{\"Command\":\"Tick\",\"Seconds\":3}\r\nnot json\n\xff\n{\"Command\":\"ListQueues\"}",
    );
    assert!(success);
    assert_eq!(lines.len(), 5);
    assert!(lines[0].contains(r#""code":"InvalidRequest""#));
    assert!(lines[1].contains(r#""Now":3"#) && lines[1].contains(r#""ok":true"#));
    assert!(lines[2].contains("InvalidRequest"));
    assert!(lines[3].contains("InvalidRequest"));
    assert!(lines[4].contains(r#""QueueNames":[]"#));
}

#[test]
fn empty_input_and_a_final_newline_add_no_phantom_line() {
    assert_eq!(run(b"").0.len(), 0);
    assert_eq!(run(b"{\"Command\":\"ListQueues\"}\n").0.len(), 1);
    assert_eq!(run(b"\n").0.len(), 1);
}
