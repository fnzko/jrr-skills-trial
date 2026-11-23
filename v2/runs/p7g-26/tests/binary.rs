use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn the_binary_answers_each_line_and_exits_with_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\": \"Tick\", \"Seconds\": 3}\nnot json")
        .expect("the child reads its stdin");
    let output = child.wait_with_output().expect("the child exits");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    let stdout = String::from_utf8(output.stdout).expect("responses are UTF-8");
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], r#"{"ok":true,"Now":3}"#);
    assert!(
        lines[1].starts_with(r#"{"ok":false,"error":{"code":"InvalidRequest","message":""#),
        "{}",
        lines[1]
    );
}
