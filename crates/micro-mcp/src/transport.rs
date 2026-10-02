//! The two ways a server is spoken to: over a program's standard streams, and over streamable
//! HTTP (MCP 2025-06-18). Either way, what the server says arrives on one channel, and the client
//! routes answers back to whoever asked.

use crate::oauth::Challenge;
use async_trait::async_trait;
use futures::StreamExt as _;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

/// A single message larger than this is refused rather than buffered.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Characters of an error body quoted in a message.
const ERROR_BODY_CHARS: usize = 500;

/// How much of a server's standard error is kept to explain a failure.
const STDERR_TAIL_BYTES: usize = 4 * 1024;

/// How often the server-to-client stream is reopened before it is given up on.
const STREAM_RETRIES: u32 = 5;
const STREAM_INITIAL_DELAY: Duration = Duration::from_secs(1);
const STREAM_MAX_DELAY: Duration = Duration::from_secs(30);

/// How long closing a session may hold up shutting the connection.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// What went wrong carrying a message.
#[derive(Debug, Clone)]
pub enum TransportError {
    /// The connection is gone.
    Closed,
    /// The server needs a sign-in, or more scope than the stored grant has.
    AuthRequired(Challenge),
    /// The server answered with a failure status.
    Http { status: u16, message: String },
    /// The request never reached the server, or its answer never arrived.
    Network(String),
    /// Anything else, already worded.
    Other(String),
}

impl TransportError {
    /// Whether another attempt may succeed: the network failed, or the server was overloaded,
    /// restarting, or slow to answer.
    pub fn is_transient(&self) -> bool {
        match self {
            TransportError::Network(_) => true,
            TransportError::Http { status, .. } => is_transient(*status) && *status != 501,
            TransportError::Closed | TransportError::AuthRequired(_) | TransportError::Other(_) => {
                false
            }
        }
    }
}

/// Something that can give an HTTP server its bearer token and act on the server turning it down.
#[async_trait]
pub trait Authorizer: Send + Sync {
    /// The token to send, if there is one.
    async fn token(&self) -> Result<Option<String>, TransportError>;

    /// The server refused `rejected` with `challenge`. `Ok` means a retry may succeed.
    async fn unauthorized(
        &self,
        challenge: &Challenge,
        rejected: Option<&str>,
    ) -> Result<(), TransportError>;
}

#[async_trait]
pub trait Transport: Send + Sync {
    /// Deliver one message. Whatever the server says back arrives on the incoming channel.
    async fn send(&self, message: &Value) -> Result<(), TransportError>;

    /// The protocol revision the handshake settled on.
    fn set_protocol_version(&self, version: &str) {
        let _ = version;
    }

    /// What the server wrote to its standard error lately, for explaining a failure.
    fn stderr_tail(&self) -> Option<String> {
        None
    }
}

/// A program micro started, spoken to one line of JSON at a time.
pub struct StdioTransport {
    outbound: UnboundedSender<String>,
    stderr: Arc<Mutex<String>>,
    _child: tokio::process::Child,
}

impl StdioTransport {
    /// Start `command`. What it writes to its standard error is kept to explain a failure and,
    /// when `log` names a log and the server, appended to that log.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: Option<&std::path::Path>,
        incoming: UnboundedSender<Value>,
        log: Option<(Arc<crate::ServerLog>, String)>,
    ) -> std::io::Result<StdioTransport> {
        let mut process = tokio::process::Command::new(command);
        process
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            process.current_dir(cwd);
        }
        let mut child = process.spawn()?;

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let (outbound, mut queued) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let mut stdin = stdin;
            while let Some(line) = queued.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    return;
                }
            }
        });

        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    if incoming.send(message).is_err() {
                        return;
                    }
                }
            }
        });

        let tail = Arc::new(Mutex::new(String::new()));
        let writing = Arc::clone(&tail);
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some((log, server)) = &log {
                    log.stderr(server, &line);
                }
                let mut tail = writing
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                tail.push_str(&line);
                tail.push('\n');
                if tail.len() > STDERR_TAIL_BYTES {
                    let cut = tail.len() - STDERR_TAIL_BYTES;
                    let cut = (cut..tail.len())
                        .find(|index| tail.is_char_boundary(*index))
                        .unwrap_or(tail.len());
                    tail.drain(..cut);
                }
            }
        });

        Ok(StdioTransport {
            outbound,
            stderr: tail,
            _child: child,
        })
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, message: &Value) -> Result<(), TransportError> {
        self.outbound
            .send(format!("{message}\n"))
            .map_err(|_| TransportError::Closed)
    }

    fn stderr_tail(&self) -> Option<String> {
        let tail = self
            .stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let trimmed = tail.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

/// A streamable HTTP endpoint: every message is a POST, answered with JSON or with an event
/// stream, and the server may open a stream of its own with GET.
pub struct HttpTransport {
    shared: Arc<HttpShared>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    stream_started: AtomicBool,
}

struct HttpShared {
    url: reqwest::Url,
    headers: Vec<(String, String)>,
    http: reqwest::Client,
    authorizer: Option<Arc<dyn Authorizer>>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    incoming: UnboundedSender<Value>,
}

impl HttpTransport {
    pub fn new(
        url: reqwest::Url,
        headers: Vec<(String, String)>,
        authorizer: Option<Arc<dyn Authorizer>>,
        incoming: UnboundedSender<Value>,
    ) -> HttpTransport {
        HttpTransport {
            shared: Arc::new(HttpShared {
                url,
                headers,
                http: reqwest::Client::new(),
                authorizer,
                session: Mutex::new(None),
                protocol: Mutex::new(None),
                incoming,
            }),
            tasks: Mutex::new(Vec::new()),
            stream_started: AtomicBool::new(false),
        }
    }

    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tasks.retain(|task| !task.is_finished());
        tasks.push(tokio::spawn(task));
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        let tasks = self
            .tasks
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for task in tasks.drain(..) {
            task.abort();
        }

        let session = lock(&self.shared.session).clone();
        let (Some(session), Ok(runtime)) = (session, tokio::runtime::Handle::try_current()) else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        runtime.spawn(async move {
            let mut request = shared
                .http
                .delete(shared.url.clone())
                .header("Mcp-Session-Id", session);
            for (name, value) in &shared.headers {
                request = request.header(name, value);
            }
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, request.send()).await;
        });
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, message: &Value) -> Result<(), TransportError> {
        let response = self
            .shared
            .authorized(reqwest::Method::POST, |request| {
                request
                    .header("Accept", "application/json, text/event-stream")
                    .header("Content-Type", "application/json")
                    .body(message.to_string())
            })
            .await?;
        let response = self.shared.checked(response).await?;
        self.shared.capture_session(&response);

        let request_id = message
            .get("id")
            .filter(|_| message.get("method").is_some());
        let Some(request_id) = request_id.cloned() else {
            if message.get("method").and_then(Value::as_str) == Some("notifications/initialized")
                && !self.stream_started.swap(true, Ordering::SeqCst)
            {
                let shared = Arc::clone(&self.shared);
                self.spawn(async move { shared.server_stream().await });
            }
            return Ok(());
        };

        let status = response.status().as_u16();
        if status == 202 || status == 204 {
            return Err(TransportError::Other(format!(
                "the server accepted {} without answering it",
                message["method"]
            )));
        }

        match content_type(&response).as_deref() {
            Some("application/json") => {
                let body: Value = response
                    .json()
                    .await
                    .map_err(|error| TransportError::Other(error.to_string()))?;
                let messages = match body {
                    Value::Array(items) => items,
                    single => vec![single],
                };
                for message in messages {
                    let _ = self.shared.incoming.send(message);
                }
                Ok(())
            }
            Some("text/event-stream") => {
                let shared = Arc::clone(&self.shared);
                self.spawn(async move { shared.answer_stream(response, request_id).await });
                Ok(())
            }
            other => Err(TransportError::Other(format!(
                "unsupported response content type: {}",
                other.unwrap_or("missing")
            ))),
        }
    }

    fn set_protocol_version(&self, version: &str) {
        *lock(&self.shared.protocol) = Some(version.to_string());
    }
}

impl HttpShared {
    /// Send a request with the session, protocol and credential headers. A refusal the
    /// authorizer can act on is handed to it once, and the request sent again.
    async fn authorized(
        &self,
        method: reqwest::Method,
        build: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, TransportError> {
        let mut retried = false;
        loop {
            let mut request = self.http.request(method.clone(), self.url.clone());
            for (name, value) in &self.headers {
                request = request.header(name, value);
            }
            if let Some(session) = lock(&self.session).clone() {
                request = request.header("Mcp-Session-Id", session);
            }
            if let Some(protocol) = lock(&self.protocol).clone() {
                request = request.header("MCP-Protocol-Version", protocol);
            }
            let token = match &self.authorizer {
                Some(authorizer) => authorizer.token().await?,
                None => None,
            };
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }

            let response = build(request)
                .send()
                .await
                .map_err(|error| TransportError::Network(network_error(&error)))?;
            if !needs_authorization(&response) {
                return Ok(response);
            }
            let challenge = Challenge::from_header(header(&response, "www-authenticate"));
            match (&self.authorizer, retried) {
                (Some(authorizer), false) => {
                    authorizer
                        .unauthorized(&challenge, token.as_deref())
                        .await?;
                    retried = true;
                }
                (Some(_), true) if response.status().as_u16() == 403 => {
                    return Err(TransportError::AuthRequired(challenge))
                }
                (Some(_), true) => return Ok(response),
                (None, _) => return Err(TransportError::AuthRequired(challenge)),
            }
        }
    }

    /// The response, or what its failure status means.
    async fn checked(
        &self,
        response: reqwest::Response,
    ) -> Result<reqwest::Response, TransportError> {
        let status = response.status().as_u16();
        if response.status().is_success() {
            return Ok(response);
        }
        if status == 401 {
            let challenge = Challenge::from_header(header(&response, "www-authenticate"));
            return Err(TransportError::AuthRequired(challenge));
        }
        let had_session = lock(&self.session).is_some();
        let body = response.text().await.unwrap_or_default();
        if status == 404 && had_session {
            *lock(&self.session) = None;
            return Err(TransportError::Http {
                status,
                message: "the MCP session expired".to_string(),
            });
        }
        Err(TransportError::Http {
            status,
            message: describe_failure(status, &body),
        })
    }

    fn capture_session(&self, response: &reqwest::Response) {
        if let Some(session) = header(response, "mcp-session-id") {
            *lock(&self.session) = Some(session.to_string());
        }
    }

    /// Read the event stream answering one request. A stream that ends before the answer, after
    /// the server numbered its events, is resumed from the last one with GET, retrying network
    /// failures and transient statuses with backoff. A stream that cannot be resumed is turned into
    /// an error answer, so the request does not wait for nothing.
    async fn answer_stream(&self, response: reqwest::Response, request_id: Value) {
        let mut answered = false;
        let mut last_event_id: Option<String> = None;
        let mut response = Some(response);
        let mut attempt = 0;
        let reason = loop {
            let mut received = false;
            let outcome = match response.take() {
                Some(response) => {
                    self.read_events(response, &mut last_event_id, |message| {
                        received = true;
                        if message.get("id") == Some(&request_id)
                            && (message.get("result").is_some() || message.get("error").is_some())
                        {
                            answered = true;
                        }
                    })
                    .await
                }
                None => Ok(()),
            };
            if answered {
                return;
            }
            let reason = match &outcome {
                Ok(()) => "the stream ended without an answer".to_string(),
                Err(error) => error.clone(),
            };
            if received {
                attempt = 0;
            }
            let Some(resume_from) = last_event_id.clone() else {
                break reason;
            };
            if attempt >= STREAM_RETRIES {
                break reason;
            }
            attempt += 1;
            tokio::time::sleep(backoff(attempt)).await;
            match self.resumed_stream(&resume_from).await {
                Ok(resumed) => response = Some(resumed),
                Err(error) if error.is_transient() => {}
                Err(error) => break describe_transport_error(&error),
            }
        };
        let _ = self.incoming.send(json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": { "code": -32603, "message": format!("the response stream failed: {reason}") },
        }));
    }

    /// A GET stream that picks up after the event `last_event_id`.
    async fn resumed_stream(
        &self,
        last_event_id: &str,
    ) -> Result<reqwest::Response, TransportError> {
        let response = self
            .authorized(reqwest::Method::GET, |request| {
                request
                    .header("Accept", "text/event-stream")
                    .header("Last-Event-ID", last_event_id)
            })
            .await?;
        let response = self.checked(response).await?;
        match content_type(&response).as_deref() {
            Some("text/event-stream") => Ok(response),
            other => Err(TransportError::Other(format!(
                "unsupported resumed stream content type: {}",
                other.unwrap_or("missing")
            ))),
        }
    }

    /// Keep the server-to-client stream open, reopening it with backoff when it drops.
    async fn server_stream(&self) {
        let mut last_event_id: Option<String> = None;
        let mut attempt = 0;
        while attempt <= STREAM_RETRIES {
            let opened = self
                .authorized(reqwest::Method::GET, |request| {
                    let request = request.header("Accept", "text/event-stream");
                    match &last_event_id {
                        Some(id) => request.header("Last-Event-ID", id),
                        None => request,
                    }
                })
                .await;
            let response = match opened {
                Ok(response) if response.status().as_u16() == 405 => return,
                Ok(response) if response.status().is_success() => response,
                Ok(response) if is_transient(response.status().as_u16()) => {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                Ok(_) | Err(TransportError::AuthRequired(_)) => return,
                Err(_) => {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
            };
            if content_type(&response).as_deref() != Some("text/event-stream") {
                return;
            }
            let mut received = false;
            let _ = self
                .read_events(response, &mut last_event_id, |_| received = true)
                .await;
            attempt = match received {
                true => 0,
                false => attempt + 1,
            };
            tokio::time::sleep(backoff(attempt)).await;
        }
    }

    /// Pass every JSON-RPC message of an event stream on, telling `seen` about each.
    async fn read_events(
        &self,
        response: reqwest::Response,
        last_event_id: &mut Option<String>,
        mut seen: impl FnMut(&Value),
    ) -> Result<(), String> {
        let mut parser = SseParser::default();
        let mut body = response.bytes_stream();
        let mut deliver = |events: Vec<SseEvent>, last_event_id: &mut Option<String>| {
            for event in events {
                if let Some(id) = event.id {
                    *last_event_id = Some(id);
                }
                if event.data.trim().is_empty()
                    || event.event.as_deref().is_some_and(|kind| kind != "message")
                {
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(&event.data) else {
                    continue;
                };
                seen(&message);
                let _ = self.incoming.send(message);
            }
        };
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| network_error(&error))?;
            let events = parser.push(&chunk)?;
            deliver(events, last_event_id);
        }
        deliver(parser.finish(), last_event_id);
        Ok(())
    }
}

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

/// Turns the bytes of an event stream into events, however the bytes are split.
#[derive(Default)]
pub struct SseParser {
    buffered: Vec<u8>,
    event: Option<String>,
    id: Option<String>,
    data: Vec<String>,
    data_bytes: usize,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buffered.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffered.drain(..=newline).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            if let Some(event) = self.line(&line)? {
                events.push(event);
            }
        }
        if self.buffered.len() > MAX_MESSAGE_BYTES {
            return Err(format!("an event exceeds {MAX_MESSAGE_BYTES} bytes"));
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.buffered.is_empty() {
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.buffered)).into_owned();
            if let Ok(Some(event)) = self.line(&line) {
                events.push(event);
            }
        }
        events.extend(self.dispatch());
        events
    }

    fn line(&mut self, line: &str) -> Result<Option<SseEvent>, String> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                self.data_bytes += value.len() + usize::from(!self.data.is_empty());
                if self.data_bytes > MAX_MESSAGE_BYTES {
                    return Err(format!("an event exceeds {MAX_MESSAGE_BYTES} bytes"));
                }
                self.data.push(value.to_string());
            }
            "event" => self.event = Some(value.to_string()),
            "id" if !value.contains('\0') => self.id = Some(value.to_string()),
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        let id = self.id.take();
        if self.data.is_empty() {
            return id.map(|id| SseEvent {
                event,
                data: String::new(),
                id: Some(id),
            });
        }
        let data = std::mem::take(&mut self.data).join("\n");
        self.data_bytes = 0;
        Some(SseEvent { event, data, id })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn content_type(response: &reqwest::Response) -> Option<String> {
    header(response, "content-type").map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    })
}

/// 401, or 403 with an `insufficient_scope` challenge asking for more than the token grants.
fn needs_authorization(response: &reqwest::Response) -> bool {
    match response.status().as_u16() {
        401 => true,
        403 => {
            Challenge::from_header(header(response, "www-authenticate"))
                .error
                .as_deref()
                == Some("insufficient_scope")
        }
        _ => false,
    }
}

fn is_transient(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

fn backoff(attempt: u32) -> Duration {
    STREAM_INITIAL_DELAY
        .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
        .min(STREAM_MAX_DELAY)
}

fn describe_failure(status: u16, body: &str) -> String {
    let text = body.trim();
    let snippet: String = text.chars().take(ERROR_BODY_CHARS).collect();
    match snippet.is_empty() {
        true => format!("HTTP {status}"),
        false if snippet.len() < text.len() => format!("HTTP {status}: {snippet}…"),
        false => format!("HTTP {status}: {snippet}"),
    }
}

fn describe_transport_error(error: &TransportError) -> String {
    match error {
        TransportError::Closed => "the connection closed".to_string(),
        TransportError::AuthRequired(_) => "the server asked for a sign-in".to_string(),
        TransportError::Http { message, .. }
        | TransportError::Network(message)
        | TransportError::Other(message) => message.clone(),
    }
}

fn network_error(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_survive_being_split_anywhere() {
        let stream = b"event: message\r\nid: 7\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\n: comment\n\ndata: two\n\n";
        for split in 0..stream.len() {
            let mut parser = SseParser::default();
            let mut events = parser.push(&stream[..split]).unwrap();
            events.extend(parser.push(&stream[split..]).unwrap());
            events.extend(parser.finish());
            assert_eq!(
                events,
                vec![
                    SseEvent {
                        event: Some("message".into()),
                        data: "{\"a\":\n1}".into(),
                        id: Some("7".into()),
                    },
                    SseEvent {
                        event: None,
                        data: "two".into(),
                        id: None,
                    },
                ],
                "split at {split}"
            );
        }
    }

    #[test]
    fn an_unterminated_last_event_still_arrives() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: last").unwrap().is_empty());
        assert_eq!(parser.finish()[0].data, "last");
    }
}
