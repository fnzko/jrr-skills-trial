use std::io::Write;
use std::process::{Command, Stdio};

fn run_miniq(input: &[u8]) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("miniq reads its input");
    let output = child.wait_with_output().expect("miniq exits");
    (
        String::from_utf8(output.stdout).expect("miniq writes UTF-8"),
        output.status.code(),
    )
}

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let input =
        b"{\"Command\": \"Tick\", \"Seconds\": 5}\n\n\xff\n{\"Command\": \"Tick\", \"Seconds\": 1}";
    let (stdout, code) = run_miniq(input);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    assert!(stdout.ends_with('\n'));
    assert_eq!(lines[0], r#"{"ok":true,"Now":5}"#);
    assert!(
        lines[1].contains(r#""code":"InvalidRequest""#),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains(r#""code":"InvalidRequest""#),
        "{}",
        lines[2]
    );
    assert_eq!(lines[3], r#"{"ok":true,"Now":6}"#);
    assert_eq!(code, Some(0));
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_miniq(b""), (String::new(), Some(0)));
}
