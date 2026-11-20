use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn the_binary_answers_each_line_before_reading_the_next() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let mut ask = |line: &str| -> Value {
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut response = String::new();
        stdout.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    };
    assert_eq!(
        ask(r#"{"Command": "Tick", "Seconds": 3}"#),
        json!({"ok": true, "Now": 3})
    );
    assert_eq!(ask("garbage")["error"]["code"], "InvalidRequest");
    assert_eq!(ask(r#"{"Command": "Tick", "Seconds": 4}"#)["Now"], 7);

    // A last line without a newline still counts.
    write!(stdin, r#"{{"Command": "Tick", "Seconds": 1}}"#).unwrap();
    drop(stdin);
    let mut last = String::new();
    stdout.read_line(&mut last).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&last).unwrap()["Now"], 8);
    let mut rest = String::new();
    assert_eq!(stdout.read_line(&mut rest).unwrap(), 0);
    assert!(child.wait().unwrap().success());
}
