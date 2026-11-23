use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_a_final_line_without_newline_and_exits_zero() {
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
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":4}\n{\"Command\":\"Tick\",\"Seconds\":1}")
        .expect("the binary reads its stdin");
    let output = child.wait_with_output().expect("the binary exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "{\"ok\":true,\"Now\":4}\n{\"ok\":true,\"Now\":5}\n"
    );
    assert_eq!(output.stderr, b"");
}
