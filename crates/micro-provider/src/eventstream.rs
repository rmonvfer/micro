//! AWS's binary event stream, which is what Bedrock answers with.
//!
//! Not server-sent events. Each message is a length-prefixed frame carrying its own
//! headers and a payload, so a reader has to know how long a frame is before it can find
//! the next one:
//!
//! ```text
//! total length   4 bytes, big endian, counting everything including itself
//! header length  4 bytes, big endian
//! prelude CRC    4 bytes
//! headers        header-length bytes
//! payload        total - headers - 16 bytes
//! message CRC    4 bytes
//! ```
//!
//! The checksums are not verified. The stream arrives over TLS, which already rejects a
//! corrupted body, and a frame that survived that but is damaged fails to parse as JSON
//! immediately afterwards. What the headers are for is the event's name: Bedrock puts it
//! in `:event-type`, and that is what says how to read the payload.

/// One frame: what kind of event it is, and the JSON it carried.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The `:event-type` header, which names the event.
    pub event_type: String,
    /// The `:message-type` header. An `exception` says the payload is an error.
    pub message_type: String,
    pub payload: Vec<u8>,
}

/// Everything before the headers: three big-endian words.
const PRELUDE: usize = 12;
/// The trailing checksum.
const TRAILER: usize = 4;
/// A frame longer than this is not one; the stream is being misread.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Reads frames out of bytes as they arrive.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder::default()
    }

    /// Take another piece of the stream, and hand back whatever frames are now complete.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();

        loop {
            if self.buffer.len() < PRELUDE {
                break;
            }
            let total = u32::from_be_bytes(self.buffer[0..4].try_into().unwrap()) as usize;
            let headers_length = u32::from_be_bytes(self.buffer[4..8].try_into().unwrap()) as usize;

            if total > MAX_FRAME || total < PRELUDE + TRAILER {
                return Err(format!("event stream frame claims a length of {total}"));
            }
            if headers_length > total.saturating_sub(PRELUDE + TRAILER) {
                return Err("event stream frame claims more headers than it holds".to_string());
            }
            // Not all here yet, which is ordinary: the next read will bring the rest.
            if self.buffer.len() < total {
                break;
            }

            let headers = &self.buffer[PRELUDE..PRELUDE + headers_length];
            let payload = &self.buffer[PRELUDE + headers_length..total - TRAILER];
            let named = read_headers(headers)?;

            frames.push(Frame {
                event_type: named
                    .iter()
                    .find(|(name, _)| name == ":event-type")
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default(),
                message_type: named
                    .iter()
                    .find(|(name, _)| name == ":message-type")
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default(),
                payload: payload.to_vec(),
            });

            self.buffer.drain(..total);
        }

        Ok(frames)
    }
}

/// The headers of one frame.
///
/// Only string-valued headers are read back as values; the rest are named but left empty,
/// because the ones that matter here — the event's type and the message's kind — are
/// strings, and skipping the others correctly is what keeps the walk in step.
fn read_headers(mut bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();

    while !bytes.is_empty() {
        let name_length = bytes[0] as usize;
        if bytes.len() < 1 + name_length + 1 {
            return Err("event stream header ends part way through".to_string());
        }
        let name = String::from_utf8_lossy(&bytes[1..1 + name_length]).into_owned();
        let kind = bytes[1 + name_length];
        bytes = &bytes[1 + name_length + 1..];

        // The value's length depends on what kind of value it is.
        let value_length = match kind {
            // true, false: the type is the value.
            0 | 1 => 0,
            // byte
            2 => 1,
            // short
            3 => 2,
            // integer
            4 => 4,
            // long, timestamp
            5 | 8 => 8,
            // byte array and string, both prefixed with a two-byte length.
            6 | 7 => {
                if bytes.len() < 2 {
                    return Err("event stream header value ends part way through".to_string());
                }
                let length = u16::from_be_bytes(bytes[0..2].try_into().unwrap()) as usize;
                bytes = &bytes[2..];
                length
            }
            // uuid
            9 => 16,
            other => return Err(format!("event stream header has an unknown type {other}")),
        };

        if bytes.len() < value_length {
            return Err("event stream header value ends part way through".to_string());
        }
        let value = match kind {
            7 => String::from_utf8_lossy(&bytes[..value_length]).into_owned(),
            _ => String::new(),
        };
        bytes = &bytes[value_length..];
        headers.push((name, value));
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a frame the way Bedrock does, so the decoder is read against the shape it
    /// will actually meet.
    fn frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
        let mut headers = Vec::new();
        for (name, value) in [(":event-type", event_type), (":message-type", "event")] {
            headers.push(name.len() as u8);
            headers.extend_from_slice(name.as_bytes());
            // 7 is the string type.
            headers.push(7);
            headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
            headers.extend_from_slice(value.as_bytes());
        }

        let total = (PRELUDE + headers.len() + payload.len() + TRAILER) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&total.to_be_bytes());
        out.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        // The checksums are not read, so what is written here does not matter.
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&headers);
        out.extend_from_slice(payload);
        out.extend_from_slice(&0u32.to_be_bytes());
        out
    }

    #[test]
    fn one_whole_frame_is_read() {
        let mut decoder = Decoder::new();
        let frames = decoder
            .push(&frame("contentBlockDelta", br#"{"delta":{"text":"hi"}}"#))
            .expect("it reads");

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_type, "contentBlockDelta");
        assert_eq!(frames[0].message_type, "event");
        assert_eq!(frames[0].payload, br#"{"delta":{"text":"hi"}}"#);
    }

    /// A stream arrives in whatever pieces the network gives it, and a frame split across
    /// two of them is still one frame.
    #[test]
    fn a_frame_split_across_reads_is_still_one_frame() {
        let whole = frame("messageStart", br#"{"role":"assistant"}"#);
        let (first, second) = whole.split_at(whole.len() / 2);

        let mut decoder = Decoder::new();
        assert!(decoder.push(first).unwrap().is_empty(), "not all here yet");
        let frames = decoder.push(second).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_type, "messageStart");
    }

    /// Several frames in one read all come back, in order.
    #[test]
    fn frames_come_back_in_the_order_they_arrived() {
        let mut bytes = frame("messageStart", b"{}");
        bytes.extend(frame("contentBlockDelta", b"{\"i\":1}"));
        bytes.extend(frame("messageStop", b"{}"));

        let mut decoder = Decoder::new();
        let frames = decoder.push(&bytes).unwrap();
        let names: Vec<&str> = frames.iter().map(|f| f.event_type.as_str()).collect();
        assert_eq!(names, vec!["messageStart", "contentBlockDelta", "messageStop"]);
    }

    /// An exception says so in its message type, which is how an error is told from an
    /// answer.
    #[test]
    fn an_exception_is_marked_as_one() {
        let mut headers = Vec::new();
        for (name, value) in [(":message-type", "exception"), (":exception-type", "throttling")] {
            headers.push(name.len() as u8);
            headers.extend_from_slice(name.as_bytes());
            headers.push(7);
            headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
            headers.extend_from_slice(value.as_bytes());
        }
        let payload = br#"{"message":"slow down"}"#;
        let total = (PRELUDE + headers.len() + payload.len() + TRAILER) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&total.to_be_bytes());
        bytes.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&headers);
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&0u32.to_be_bytes());

        let frames = Decoder::new().push(&bytes).unwrap();
        assert_eq!(frames[0].message_type, "exception");
    }

    /// A length that cannot be right is reported rather than used to index into memory.
    #[test]
    fn a_nonsense_length_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert!(Decoder::new().push(&bytes).is_err());
    }

    /// Headers of other types are stepped over correctly, so the ones that matter are
    /// still found after them.
    #[test]
    fn headers_of_other_types_do_not_lose_the_walk() {
        let mut headers = Vec::new();
        // A timestamp header, eight bytes, of the kind Bedrock includes.
        headers.push(11u8);
        headers.extend_from_slice(b":event-time");
        headers.push(8);
        headers.extend_from_slice(&0u64.to_be_bytes());
        // Then the one that matters.
        headers.push(11u8);
        headers.extend_from_slice(b":event-type");
        headers.push(7);
        headers.extend_from_slice(&5u16.to_be_bytes());
        headers.extend_from_slice(b"start");

        let named = read_headers(&headers).expect("it walks past the timestamp");
        assert_eq!(named.len(), 2);
        assert_eq!(named[1], (":event-type".to_string(), "start".to_string()));
    }
}
