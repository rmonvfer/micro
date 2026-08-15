//! A minimal server-sent-events reader over an HTTP response body.

use futures::StreamExt;

/// One dispatched SSE event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Read an SSE body to completion, invoking `on_event` for each dispatched event.
pub async fn read_sse<F>(response: reqwest::Response, mut on_event: F) -> Result<(), reqwest::Error>
where
    F: FnMut(SseEvent),
{
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut current = SseEvent::default();
    let mut has_field = false;

    while let Some(chunk) = stream.next().await {
        buffer.extend_from_slice(&chunk?);

        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let raw = buffer.drain(..=newline).collect::<Vec<u8>>();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\r', '\n']);

            if line.is_empty() {
                if has_field {
                    on_event(std::mem::take(&mut current));
                    has_field = false;
                }
                continue;
            }

            if let Some(value) = line.strip_prefix("event:") {
                current.event = Some(value.trim().to_string());
                has_field = true;
            } else if let Some(value) = line.strip_prefix("data:") {
                if !current.data.is_empty() {
                    current.data.push('\n');
                }
                current
                    .data
                    .push_str(value.strip_prefix(' ').unwrap_or(value));
                has_field = true;
            }
        }
    }

    if has_field {
        on_event(current);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a body through the same line machinery `read_sse` uses, without HTTP.
    fn parse(body: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();
        let mut current = SseEvent::default();
        let mut has_field = false;

        for line in body.split_inclusive('\n') {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if has_field {
                    events.push(std::mem::take(&mut current));
                    has_field = false;
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                current.event = Some(value.trim().to_string());
                has_field = true;
            } else if let Some(value) = line.strip_prefix("data:") {
                if !current.data.is_empty() {
                    current.data.push('\n');
                }
                current
                    .data
                    .push_str(value.strip_prefix(' ').unwrap_or(value));
                has_field = true;
            }
        }
        if has_field {
            events.push(current);
        }
        events
    }

    #[test]
    fn splits_events_on_blank_lines() {
        let events = parse("event: a\ndata: {\"x\":1}\n\nevent: b\ndata: {\"y\":2}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("a"));
        assert_eq!(events[0].data, "{\"x\":1}");
        assert_eq!(events[1].data, "{\"y\":2}");
    }

    #[test]
    fn joins_multiline_data_payloads() {
        let events = parse("data: one\ndata: two\n\n");
        assert_eq!(events[0].data, "one\ntwo");
    }

    #[test]
    fn ignores_comments_and_unknown_fields() {
        let events = parse(": keep-alive\nid: 7\ndata: payload\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "payload");
    }
}
