//! The miniq executable itself: stdin in, stdout out, exit status 0 at the end
//! of input.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn the_binary_answers_each_line_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    // The last line has no newline, and it still gets a response.
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(
            b"{\"Command\": \"Tick\", \"Seconds\": 3}\n{\"Command\": \"Tick\", \"Seconds\": 4}",
        )
        .expect("miniq reads its input");
    let output = child.wait_with_output().expect("miniq finishes");
    assert!(output.status.success(), "status {:?}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "{\"Now\":3,\"ok\":true}\n{\"Now\":7,\"ok\":true}\n"
    );
}
