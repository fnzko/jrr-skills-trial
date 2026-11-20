//! The `miniq` binary, run as a process: stdin in, stdout out, exit 0 at
//! end of input.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("miniq finishes");
    assert!(
        output.stderr.is_empty(),
        "nothing goes to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn the_binary_answers_each_line_and_exits_zero() {
    let input = "{\"Command\": \"CreateQueue\", \"QueueName\": \"jobs\"}\n\
                 {\"Command\": \"SendMessage\", \"QueueName\": \"jobs\", \"Body\": \"hi\"}\n\
                 nonsense\n";
    let (stdout, code) = run(input);
    assert_eq!(code, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert_eq!(lines[0], r#"{"ok":true}"#);
    assert_eq!(lines[1], r#"{"MessageId":"m-1","ok":true}"#);
    assert!(
        lines[2].starts_with(r#"{"error":{"code":"InvalidRequest","#),
        "{}",
        lines[2]
    );
}

#[test]
fn empty_input_produces_no_output_and_exits_zero() {
    assert_eq!(run(""), (String::new(), 0));
}
