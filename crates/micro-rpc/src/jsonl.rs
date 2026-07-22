//! Strict JSON-lines framing.
//!
//! Records are separated by `\n` and by nothing else. A JSON string may legally hold
//! U+2028 and U+2029, and a reader that treats those as line breaks — as several standard
//! line readers do — will split a record in half and then fail to parse both halves.

use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

/// One record, ready to write. The trailing newline is the frame.
pub fn line(value: &impl serde::Serialize) -> String {
    match serde_json::to_string(value) {
        Ok(encoded) => format!("{encoded}\n"),
        // A value that cannot be encoded is still a record: saying so keeps the stream
        // parseable, where a silent drop would leave a caller waiting forever.
        Err(error) => format!("{{\"type\":\"error\",\"error\":\"{error}\"}}\n"),
    }
}

/// Read records off a stream, one line at a time.
///
/// A trailing `\r` is taken off, so a caller writing CRLF is understood. Blank lines are
/// skipped rather than reported as unreadable.
pub struct Lines<R> {
    reader: BufReader<R>,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Lines<R> {
    pub fn new(reader: R) -> Self {
        Lines {
            reader: BufReader::new(reader),
            buffer: Vec::new(),
        }
    }

    /// The next record, or `None` once the stream has ended.
    pub async fn next(&mut self) -> std::io::Result<Option<String>> {
        loop {
            self.buffer.clear();
            let read = self.reader.read_until(b'\n', &mut self.buffer).await?;
            if read == 0 {
                return Ok(None);
            }

            let line = String::from_utf8_lossy(&self.buffer);
            let line = line.strip_suffix('\n').unwrap_or(&line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.trim().is_empty() {
                continue;
            }
            return Ok(Some(line.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn read_all(input: &str) -> Vec<String> {
        let mut lines = Lines::new(input.as_bytes());
        let mut out = Vec::new();
        while let Some(line) = lines.next().await.unwrap() {
            out.push(line);
        }
        out
    }

    #[tokio::test]
    async fn records_are_split_on_newlines_alone() {
        let read = read_all("{\"a\":1}\n{\"b\":2}\n").await;
        assert_eq!(read, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
    }

    /// The separators that break a naive line reader are ordinary characters here.
    #[tokio::test]
    async fn a_paragraph_separator_inside_a_string_does_not_split_a_record() {
        let read = read_all("{\"a\":\"x\u{2028}y\u{2029}z\"}\n").await;
        assert_eq!(read.len(), 1);
        let value: serde_json::Value = serde_json::from_str(&read[0]).unwrap();
        assert_eq!(value["a"], "x\u{2028}y\u{2029}z");
    }

    #[tokio::test]
    async fn a_record_without_a_trailing_newline_is_still_read() {
        assert_eq!(read_all("{\"a\":1}").await, vec![r#"{"a":1}"#]);
    }

    #[tokio::test]
    async fn blank_lines_are_skipped_rather_than_reported() {
        assert_eq!(read_all("\n\n{\"a\":1}\n\n").await, vec![r#"{"a":1}"#]);
    }

    #[tokio::test]
    async fn carriage_returns_are_taken_off() {
        assert_eq!(read_all("{\"a\":1}\r\n").await, vec![r#"{"a":1}"#]);
    }

    #[test]
    fn a_written_record_ends_in_exactly_one_newline() {
        let written = line(&json!({ "type": "response" }));
        assert!(written.ends_with('\n'));
        assert_eq!(written.matches('\n').count(), 1);
    }
}
