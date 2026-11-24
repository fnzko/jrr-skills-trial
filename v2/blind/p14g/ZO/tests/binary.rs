use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n\n{\"Command\":\"Tick\",\"Seconds\":4}")
        .expect("miniq reads its input");
    drop(stdin);
    let output = child.wait_with_output().expect("miniq exits");
    assert!(output.status.success());
    let lines: Vec<&str> = std::str::from_utf8(&output.stdout)
        .expect("output is UTF-8")
        .lines()
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], r#"{"ok":true,"Now":3}"#);
    assert!(lines[1].starts_with(r#"{"ok":false,"error":{"code":"InvalidRequest""#));
    assert_eq!(lines[2], r#"{"ok":true,"Now":7}"#);
}
