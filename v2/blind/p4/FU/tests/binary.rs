//! Runs the `miniq` binary to check the stdin/stdout framing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn run(input: &[u8]) -> (Vec<Value>, std::process::ExitStatus) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("wait for miniq");
    writer.join().unwrap().expect("write stdin");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.is_empty() || stdout.ends_with('\n'));
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    (responses, output.status)
}

#[test]
fn one_response_per_line_including_blank_and_unterminated_lines() {
    let input =
        b"{\"Command\":\"Tick\",\"Seconds\":1}\n\n\xff\n{\"Command\":\"Tick\",\"Seconds\":2}";
    let (responses, status) = run(input);
    assert!(status.success());
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["Now"], 1);
    assert_eq!(responses[1]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[2]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[3]["Now"], 3);
}

#[test]
fn empty_input_exits_cleanly() {
    let (responses, status) = run(b"");
    assert!(status.success());
    assert!(responses.is_empty());
}

#[test]
fn fills_a_queue_to_its_limit_quickly() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    let started = Instant::now();
    let (responses, status) = run(input.as_bytes());
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_eq!(responses.len(), 100_002);
    assert_eq!(responses[100_000]["MessageId"], "m-100000");
    assert_eq!(responses[100_001]["error"]["code"], "LimitExceeded");
}

#[test]
fn answers_each_line_before_reading_the_next() {
    use std::io::{BufRead, BufReader};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    for expected in [5, 10] {
        stdin
            .write_all(b"{\"Command\":\"Tick\",\"Seconds\":5}\n")
            .unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["Now"], expected);
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
