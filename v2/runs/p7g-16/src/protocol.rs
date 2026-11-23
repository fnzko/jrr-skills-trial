//! Line framing and response formatting (SPEC section 1).

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value, json};

use crate::broker::Broker;
use crate::error::{Error, ErrorCode};

/// Turns one input line (without its newline) into one response line (without newline).
pub fn respond(broker: &mut Broker, line: &[u8]) -> String {
    let result = parse_command(line).and_then(|object| broker.execute(&object));
    let response = match result {
        Ok(mut members) => {
            members.insert("ok".to_owned(), Value::Bool(true));
            Value::Object(members)
        }
        Err(Error { code, message }) => json!({
            "ok": false,
            "error": { "code": code.as_str(), "message": message },
        }),
    };
    response.to_string()
}

fn parse_command(line: &[u8]) -> Result<Map<String, Value>, Error> {
    let invalid = |why: String| Error::new(ErrorCode::InvalidRequest, why);
    let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8".into()))?;
    match serde_json::from_str(text) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(invalid("line is not a JSON object".into())),
        Err(e) => Err(invalid(format!("line is not valid JSON: {e}"))),
    }
}

/// Serves commands from `input` until it ends, answering each on `output` and flushing
/// after every response.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let mut response = respond(&mut broker, &line);
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
