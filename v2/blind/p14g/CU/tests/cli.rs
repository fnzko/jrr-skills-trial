//! The binary itself: stdin lines in, flushed stdout lines out, exit status 0.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n{\"Command\":\"Tick\",\"Seconds\":4}")
        .expect("stdin accepts input");
    let output = child.wait_with_output().expect("binary exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "{\"ok\":true,\"Now\":3}\n{\"ok\":true,\"Now\":7}\n"
    );
}
