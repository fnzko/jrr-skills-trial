use std::io::{Read, Write};
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_as_it_arrives_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // The response must be there before the next line is sent: reading it
    // while stdin is still open proves it was flushed.
    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n")
        .unwrap();
    let expected = b"{\"ok\":true,\"Now\":3}\n";
    let mut first = vec![0; expected.len()];
    stdout.read_exact(&mut first).unwrap();
    assert_eq!(first, expected);

    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":4}")
        .unwrap();
    drop(stdin);
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "{\"ok\":true,\"Now\":7}\n");

    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
}
