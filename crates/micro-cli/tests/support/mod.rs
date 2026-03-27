//! A fake provider served from the test process, and a scratch home to point the binary at.
//!
//! Nothing here reaches the network or the caller's real configuration: the server binds a
//! loopback port the operating system chooses, and every run gets its own `MICRO_DIR`.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use serde_json::Value;

/// How long a test waits for a request that should already have arrived.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one connection may sit idle before its thread gives up on it.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// What the fake provider answers one request with.
pub enum Reply {
    /// A streamed chat completion: each entry is one SSE `data:` payload, followed by the
    /// terminating sentinel.
    Sse(Vec<Value>),
    /// An HTTP failure, the way a provider reports a rate limit or an outage.
    Status(u16, String),
}

impl Reply {
    /// A response whose whole body is one block of text.
    pub fn text(text: &str) -> Reply {
        Reply::Sse(vec![text_delta(text), finish("stop")])
    }

    /// A response asking for one tool, the way a real stream spreads it over deltas.
    pub fn tool_call(id: &str, name: &str, arguments: Value) -> Reply {
        Reply::Sse(vec![
            tool_call_open(0, id, name),
            tool_call_arguments(0, &arguments.to_string()),
            finish("tool_calls"),
        ])
    }
}

/// One SSE chunk carrying a fragment of assistant text.
pub fn text_delta(delta: &str) -> Value {
    json!({ "choices": [{ "index": 0, "delta": { "content": delta } }] })
}

/// The chunk that names a tool call, before its arguments start arriving.
pub fn tool_call_open(index: u64, id: &str, name: &str) -> Value {
    json!({
        "choices": [{
            "index": 0,
            "delta": { "tool_calls": [{
                "index": index,
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": "" },
            }] },
        }],
    })
}

/// A fragment of a tool call's JSON arguments.
pub fn tool_call_arguments(index: u64, fragment: &str) -> Value {
    json!({
        "choices": [{
            "index": 0,
            "delta": { "tool_calls": [{
                "index": index,
                "function": { "arguments": fragment },
            }] },
        }],
    })
}

pub fn finish(reason: &str) -> Value {
    json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": reason }] })
}

/// The headers of one request, keyed by lowercased name.
pub type Headers = BTreeMap<String, String>;

/// A provider that answers from a script and remembers what it was asked.
pub struct FakeApi {
    base_url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    /// Recorded alongside `requests`, so index `n` of each describes the same request.
    headers: Arc<Mutex<Vec<Headers>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    running: Arc<AtomicBool>,
}

impl FakeApi {
    /// Start a server on a loopback port the operating system picks.
    pub fn start(replies: impl IntoIterator<Item = Reply>) -> FakeApi {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("read the bound port").port();
        listener
            .set_nonblocking(true)
            .expect("poll for connections so the server can be shut down");

        let api = FakeApi {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            requests: Arc::new(Mutex::new(Vec::new())),
            headers: Arc::new(Mutex::new(Vec::new())),
            replies: Arc::new(Mutex::new(replies.into_iter().collect())),
            running: Arc::new(AtomicBool::new(true)),
        };

        let requests = api.requests.clone();
        let headers = api.headers.clone();
        let replies = api.replies.clone();
        let running = api.running.clone();
        std::thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                match listener.accept() {
                    // Each connection is served on its own thread. An HTTP client may hold
                    // an idle connection open, and reading it on the accept loop would
                    // stall every later request behind it.
                    Ok((stream, _)) => {
                        let requests = requests.clone();
                        let headers = headers.clone();
                        let replies = replies.clone();
                        std::thread::spawn(move || serve(stream, &requests, &headers, &replies));
                    }
                    // Nothing waiting, or a connection that went away between the queue and
                    // the accept. Neither is a reason to stop serving.
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });

        api
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Every request body the binary sent, oldest first.
    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests lock").clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().expect("requests lock").len()
    }

    /// The request at `index`, waiting briefly for it in case the binary is still in
    /// flight, and failing with a legible message when it never arrives.
    pub fn request(&self, index: usize) -> Value {
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if let Some(request) = self.requests().get(index) {
                return request.clone();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "expected at least {} request(s); the binary sent {}",
            index + 1,
            self.request_count()
        );
    }

    /// The headers of the request at `index`, keyed by lowercased name.
    pub fn headers(&self, index: usize) -> Headers {
        // Waiting on the body first means the matching headers are already recorded.
        self.request(index);
        self.headers
            .lock()
            .expect("headers lock")
            .get(index)
            .cloned()
            .unwrap_or_default()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Read one HTTP request, record its headers and body, and answer with the next scripted
/// reply.
fn serve(
    mut stream: TcpStream,
    requests: &Mutex<Vec<Value>>,
    headers: &Mutex<Vec<Headers>>,
    replies: &Mutex<VecDeque<Reply>>,
) {
    // The listener polls for connections, and an accepted socket inherits that on the BSD
    // socket layer. Left alone it would report "would block" for data that has not landed
    // yet, which reads as an empty request and hangs up on a client that was mid-send.
    let _ = stream.set_nonblocking(false);
    // A client may open a connection it never uses. Without a deadline that thread would
    // hold its scripted reply hostage for the life of the test.
    let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
    let mut reader = BufReader::new(stream.try_clone().expect("clone the connection"));

    let mut length = 0usize;
    let mut received = Headers::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            received.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    // Recording the request and claiming a reply happen together, so the count a test
    // asserts on and the script it wrote can never drift apart.
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        let _ = write!(
            stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    };
    let reply = {
        let mut requests = requests.lock().expect("requests lock");
        let reply = replies.lock().expect("replies lock").pop_front();
        headers.lock().expect("headers lock").push(received);
        requests.push(value);
        reply
    };
    match reply {
        Some(Reply::Sse(chunks)) => {
            let mut payload = String::new();
            for chunk in chunks {
                payload.push_str(&format!("data: {chunk}\n\n"));
            }
            payload.push_str("data: [DONE]\n\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Cache-Control: no-cache\r\nConnection: close\r\n\r\n{payload}"
            );
        }
        Some(Reply::Status(code, body)) => {
            let _ = write!(
                stream,
                "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
        // Running out of script is a test bug, so it is reported rather than hung on.
        None => {
            let body = "{\"error\":{\"message\":\"the fake provider ran out of replies\"}}";
            let _ = write!(
                stream,
                "HTTP/1.1 500 Error\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    }
    let _ = stream.flush();

    // An SSE body ends at the close, so the close has to be orderly: signal end-of-body,
    // then drain whatever the client still had in flight. Dropping the socket with unread
    // bytes queued makes the kernel send a reset, which the client reports as a failed
    // request rather than a finished response.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut discard = [0u8; 1024];
    while let Ok(read) = stream.read(&mut discard) {
        if read == 0 {
            break;
        }
    }
}

/// A scratch `MICRO_DIR` and workspace, wired so the binary resolves a model that lives on
/// the fake provider.
pub struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub fn new(api: &FakeApi) -> Fixture {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "micro-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&home).expect("create the scratch home");
        std::fs::create_dir_all(&workspace).expect("create the scratch workspace");

        // A credential for a provider the registry knows, so no network and no real key.
        std::fs::write(
            home.join("auth.json"),
            json!({ "openai": { "type": "api_key", "key": "test-key" } }).to_string(),
        )
        .expect("write auth.json");

        // A model whose endpoint is the fake provider. The catalog supplies the base URL
        // the client posts to, so nothing else has to be redirected.
        std::fs::write(
            home.join("models.json"),
            json!({
                "providers": {
                    "openai": {
                        "base_url": api.base_url(),
                        "api": "openai-completions",
                        "models": [{
                            "id": "test-model",
                            "name": "Test Model",
                            "context_window": 200000,
                            "max_output_tokens": 4096,
                            "aliases": ["test"],
                        }],
                    },
                },
            })
            .to_string(),
        )
        .expect("write models.json");

        // The workspace is vouched for, the way a user vouches for a project they are
        // working in. Without it a project's own extensions and skills are left alone.
        std::fs::write(
            home.join("config.json"),
            json!({ "default_project_trust": "always" }).to_string(),
        )
        .expect("write config.json");

        Fixture {
            // Canonicalized so the workspace matches what the session store records.
            root: root.canonicalize().unwrap_or(root),
        }
    }

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }

    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.workspace().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent directory");
        }
        std::fs::write(&path, contents).expect("write a workspace file");
        path
    }

    pub fn exists(&self, name: &str) -> bool {
        self.workspace().join(name).exists()
    }

    /// The session logs written so far, newest last.
    pub fn session_logs(&self) -> Vec<String> {
        let directory = self.home().join("sessions");
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|kind| kind == "jsonl"))
            .collect();
        paths.sort();
        paths
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect()
    }

    /// The binary, pointed at this fixture and stripped of anything inherited that could
    /// reach a real provider or a real configuration directory.
    pub fn micro(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_micro"));
        command.current_dir(self.workspace());
        command.env("MICRO_DIR", self.home());
        // A stray HOME would let anything that misses MICRO_DIR find the real one.
        command.env("HOME", self.home());
        for leaked in [
            "MICRO_MODEL",
            "MICRO_PROVIDER",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "COPILOT_GITHUB_TOKEN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
        ] {
            command.env_remove(leaked);
        }
        command
    }

    /// Drive the headless protocol: write these command lines to stdin, and take back
    /// every line that came out.
    pub fn rpc(&self, commands: &[&str]) -> Vec<Value> {
        use std::process::Stdio;

        let mut command = self.micro();
        command.arg("--rpc");
        // The fixture's own model, so the run reaches the fake provider and not a real one.
        command.args(["-m", "test"]);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command.spawn().expect("micro --rpc starts");
        {
            let stdin = child.stdin.as_mut().expect("stdin is piped");
            for line in commands {
                writeln!(stdin, "{line}").expect("a command is written");
            }
        }
        // Closing stdin is what ends the mode; without it the child waits for more.
        drop(child.stdin.take());

        let finished = child.wait_with_output().expect("micro --rpc finishes");
        if finished.stdout.is_empty() {
            panic!(
                "micro --rpc said nothing; it exited {} with: {}",
                finished.status,
                String::from_utf8_lossy(&finished.stderr).trim()
            );
        }
        String::from_utf8_lossy(&finished.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("unreadable line {line}: {error}"))
            })
            .collect()
    }

    /// Run the binary with these arguments and take back what it printed.
    pub fn micro_run(&self, arguments: &[&str]) -> Output {
        let mut command = self.micro();
        command.args(arguments);
        Output::run(&mut command)
    }

    /// Run one prompt to completion with `--print`, with stdin closed so nothing can be
    /// mistaken for an approval.
    pub fn print(&self, arguments: &[&str]) -> Output {
        let mut command = self.micro();
        command.arg("--print");
        command.args(arguments);
        Output::run(&mut command)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// What a run of the binary produced.
pub struct Output {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// Takes the command by reference so `Command::args`, which borrows, chains directly
    /// into it.
    pub fn run(command: &mut Command) -> Output {
        command.stdin(std::process::Stdio::null());
        let output = command.output().expect("run the micro binary");
        Output {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    pub fn succeeded(&self) -> bool {
        self.status.success()
    }

    /// Fail with both streams attached, since a bare exit code says nothing about why.
    pub fn expect_success(&self, what: &str) -> &Output {
        assert!(
            self.succeeded(),
            "{what} exited with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.status,
            self.stdout,
            self.stderr
        );
        self
    }

    pub fn expect_failure(&self, what: &str) -> &Output {
        assert!(
            !self.succeeded(),
            "{what} unexpectedly succeeded\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout,
            self.stderr
        );
        self
    }
}

/// The text of every message in a recorded request, joined so a test can look for what it
/// expects without walking the wire shape.
pub fn transcript(request: &Value) -> String {
    request["messages"].to_string()
}

/// The tool names offered in a recorded request.
pub fn offered_tools(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Every message in a recorded request that carries a tool result.
pub fn tool_results(request: &Value) -> Vec<Value> {
    request["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message["role"] == "tool")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn path_of(fixture: &Fixture, name: &str) -> String {
    fixture.workspace().join(name).display().to_string()
}
