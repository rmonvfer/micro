//! A loopback HTTP server for tests, answering each request with whatever a handler says.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

/// One request the server received.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_default()
    }

    /// The form fields of an `application/x-www-form-urlencoded` body.
    pub fn form(&self) -> HashMap<String, String> {
        reqwest::Url::parse(&format!("http://x/?{}", self.body))
            .map(|url| url.query_pairs().into_owned().collect())
            .unwrap_or_default()
    }
}

/// What to answer with.
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn json(value: serde_json::Value) -> Response {
        Response {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: value.to_string(),
        }
    }

    pub fn events(messages: &[serde_json::Value]) -> Response {
        Response {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body: messages
                .iter()
                .map(|message| format!("event: message\ndata: {message}\n\n"))
                .collect(),
        }
    }

    pub fn status(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

type Handler = dyn Fn(&Request) -> Response + Send + Sync;

pub struct TestServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestServer {
    pub async fn start(
        handler: impl Fn(&Request) -> Response + Send + Sync + 'static,
    ) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let recorded = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = Arc::clone(&handler);
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move { serve(stream, &*handler, &recorded).await });
            }
        });
        TestServer {
            base,
            requests,
            task,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve(
    mut stream: tokio::net::TcpStream,
    handler: &Handler,
    recorded: &Mutex<Vec<Request>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut start = lines.next().unwrap_or_default().split_whitespace();
    let method = start.next().unwrap_or_default().to_string();
    let target = start.next().unwrap_or("/").to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&chunk[..read]),
        }
    }

    let url = reqwest::Url::parse(&format!("http://x{target}")).unwrap();
    let request = Request {
        method,
        path: url.path().to_string(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    recorded.lock().unwrap().push(request.clone());
    let response = handler(&request);

    let mut written = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.body.len()
    );
    for (name, value) in &response.headers {
        written.push_str(&format!("{name}: {value}\r\n"));
    }
    written.push_str("\r\n");
    written.push_str(&response.body);
    let _ = stream.write_all(written.as_bytes()).await;
    let _ = stream.shutdown().await;
}
