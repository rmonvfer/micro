//! What every browser and device-code sign-in shares: PKCE, the loopback page a browser is sent
//! back to, reading what a user pastes instead, and polling a device-code grant.

use crate::AuthError;
use crate::Result;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore as _;
use serde_json::Value;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Where a loopback callback listens, unless `MICRO_OAUTH_CALLBACK_HOST` names somewhere else.
const DEFAULT_CALLBACK_HOST: &str = "127.0.0.1";
pub const CALLBACK_HOST_ENV: &str = "MICRO_OAUTH_CALLBACK_HOST";

/// RFC 8628 §3.2: a server that names no interval gets polled every five seconds.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
/// RFC 8628 §3.5: `slow_down` adds five seconds to the interval.
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);
const MINIMUM_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The longest a request line and headers from a browser may run before the request is refused.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// A PKCE verifier and the challenge derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Pkce::from_verifier(random_base64url(32))
    }

    /// RFC 7636 S256: the challenge is the unpadded base64url SHA-256 of the verifier.
    pub fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }
}

/// `count` random bytes, base64url without padding.
pub fn random_base64url(count: usize) -> String {
    URL_SAFE_NO_PAD.encode(random_bytes(count))
}

/// `count` random bytes as lowercase hex.
pub fn random_hex(count: usize) -> String {
    random_bytes(count)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A random (version 4) UUID.
pub fn random_uuid() -> String {
    let mut bytes = random_bytes(16);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

/// `base?key=value&…`, each value percent-encoded.
pub fn url_with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse(base).expect("authorization endpoints are valid URLs");
    url.query_pairs_mut().extend_pairs(params);
    url.to_string()
}

/// The host a loopback callback listens on.
pub fn callback_host() -> String {
    std::env::var(CALLBACK_HOST_ENV)
        .ok()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| DEFAULT_CALLBACK_HOST.to_string())
}

/// What a user pasted after signing in: a redirect URL, `code#state`, a query string, or the bare
/// code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationInput {
    pub code: Option<String>,
    pub state: Option<String>,
}

pub fn parse_authorization_input(input: &str) -> AuthorizationInput {
    let value = input.trim();
    if value.is_empty() {
        return AuthorizationInput::default();
    }

    if let Ok(url) = reqwest::Url::parse(value) {
        if url.has_host() {
            let pairs: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
            return AuthorizationInput {
                code: pairs.get("code").cloned(),
                state: pairs.get("state").cloned(),
            };
        }
    }

    if let Some((code, state)) = value.split_once('#') {
        return AuthorizationInput {
            code: Some(code.to_string()).filter(|code| !code.is_empty()),
            state: Some(state.to_string()).filter(|state| !state.is_empty()),
        };
    }

    if value.contains("code=") {
        let pairs: BTreeMap<String, String> = reqwest::Url::parse(&format!(
            "http://localhost/?{}",
            value.trim_start_matches('?')
        ))
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default();
        return AuthorizationInput {
            code: pairs.get("code").cloned(),
            state: pairs.get("state").cloned(),
        };
    }

    AuthorizationInput {
        code: Some(value.to_string()),
        state: None,
    }
}

/// A pending device authorization: the page the user opens, the code they type there, and the code
/// the poll is made with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub verification_uri: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

/// The query a provider sent the browser back with.
pub type CallbackParams = BTreeMap<String, String>;

/// A one-shot HTTP listener on the loopback interface that waits for the browser to come back.
pub struct CallbackServer {
    redirect_uri: String,
    results: Mutex<mpsc::Receiver<Result<CallbackParams>>>,
    task: tokio::task::JoinHandle<()>,
}

/// What a callback server needs to know to tell the real redirect from anything else.
#[derive(Debug, Clone)]
pub struct CallbackOptions {
    /// The provider's name, shown on the page the browser lands on.
    pub provider_name: String,
    pub host: String,
    /// `0` picks a free port.
    pub port: u16,
    pub path: String,
    /// The host written into the redirect URI when it differs from `host`, such as `localhost`.
    pub redirect_host: Option<String>,
    /// The `state` the redirect must carry; `None` when the provider sends none.
    pub state: Option<String>,
}

impl CallbackServer {
    pub async fn start(options: CallbackOptions) -> std::io::Result<Self> {
        let listener = TcpListener::bind((options.host.as_str(), options.port)).await?;
        let port = listener.local_addr()?.port();
        let redirect_host = options
            .redirect_host
            .clone()
            .unwrap_or_else(|| options.host.clone());
        let redirect_host = match redirect_host.contains(':') {
            true => format!("[{redirect_host}]"),
            false => redirect_host,
        };
        let redirect_uri = format!("http://{redirect_host}:{port}{}", options.path);

        let (sender, receiver) = mpsc::channel(1);
        let task = tokio::spawn(serve(listener, options, sender));
        Ok(CallbackServer {
            redirect_uri,
            results: Mutex::new(receiver),
            task,
        })
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// The parameters of the first redirect that carried a code, or the error it carried.
    pub async fn wait(&self) -> Result<CallbackParams> {
        let mut results = self.results.lock().await;
        match results.recv().await {
            Some(result) => result,
            None => Err(AuthError::OAuth(
                "the sign-in callback stopped listening".into(),
            )),
        }
    }
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    listener: TcpListener,
    options: CallbackOptions,
    sender: mpsc::Sender<Result<CallbackParams>>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        if let Some(result) = handle_connection(stream, &options).await {
            let _ = sender.send(result).await;
            return;
        }
    }
}

/// Answer one browser request, returning the outcome when it ends the sign-in.
async fn handle_connection(
    mut stream: TcpStream,
    options: &CallbackOptions,
) -> Option<Result<CallbackParams>> {
    let target = read_request_target(&mut stream).await?;
    let (status, page, outcome) = classify_callback(&target, options);
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    outcome
}

/// The request target of a `GET`, read up to the end of the headers.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        if buffer.len() > MAX_REQUEST_BYTES {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET").then(|| target.to_string())
}

/// What a request to the callback means: the status and page to answer with, and the outcome when
/// the sign-in is over.
fn classify_callback(
    target: &str,
    options: &CallbackOptions,
) -> (&'static str, String, Option<Result<CallbackParams>>) {
    let Ok(url) = reqwest::Url::parse(&format!("http://localhost{target}")) else {
        return (
            "400 Bad Request",
            error_page("Malformed request.", None),
            None,
        );
    };
    if url.path() != options.path {
        return (
            "404 Not Found",
            error_page("Callback route not found.", None),
            None,
        );
    }
    let params: CallbackParams = url.query_pairs().into_owned().collect();
    if let Some(expected) = &options.state {
        if params.get("state") != Some(expected) {
            return ("400 Bad Request", error_page("State mismatch.", None), None);
        }
    }
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| error.clone());
        let heading = format!("{} authorization failed.", options.provider_name);
        return (
            "400 Bad Request",
            error_page(&heading, Some(&description)),
            Some(Err(AuthError::OAuth(format!(
                "{} authorization failed: {description}",
                options.provider_name
            )))),
        );
    }
    if params.get("code").is_none_or(|code| code.is_empty()) {
        return (
            "400 Bad Request",
            error_page("Missing authorization code.", None),
            None,
        );
    }
    (
        "200 OK",
        success_page(&format!(
            "Signed in to {}. You may now close this page.",
            options.provider_name
        )),
        Some(Ok(params)),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(title: &str, heading: &str, details: Option<&str>) -> String {
    let details = details
        .map(|details| format!("<pre>{}</pre>", escape_html(details)))
        .unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"/>\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\
         <title>{title}</title><style>\
         html{{color-scheme:dark}}body{{margin:0;min-height:100vh;display:flex;align-items:center;\
         justify-content:center;padding:24px;background:#09090b;color:#fafafa;\
         font-family:ui-sans-serif,system-ui,sans-serif;text-align:center}}\
         pre{{color:#a1a1aa;white-space:pre-wrap;font-family:ui-monospace,monospace}}\
         </style></head><body><main><h1>{}</h1>{details}</main></body></html>",
        escape_html(heading)
    )
}

fn success_page(message: &str) -> String {
    page("micro: signed in", message, None)
}

fn error_page(heading: &str, details: Option<&str>) -> String {
    page("micro: sign-in failed", heading, details)
}

/// Wait for the browser to come back, or for the user to paste what the browser shows instead.
///
/// `manual` resolves to what the user typed, or to nothing when they dismissed the prompt; after a
/// dismissal only the browser can still finish the sign-in.
pub async fn callback_or_manual<M>(
    callback: Option<&CallbackServer>,
    manual: M,
) -> Result<CallbackOrManual>
where
    M: Future<Output = Option<String>>,
{
    let Some(callback) = callback else {
        return match manual.await {
            Some(input) => Ok(CallbackOrManual::Manual(input)),
            None => Err(AuthError::Cancelled),
        };
    };

    tokio::select! {
        result = callback.wait() => result.map(CallbackOrManual::Callback),
        input = manual => match input {
            Some(input) => Ok(CallbackOrManual::Manual(input)),
            None => callback.wait().await.map(CallbackOrManual::Callback),
        },
    }
}

/// How a browser sign-in came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackOrManual {
    Callback(CallbackParams),
    Manual(String),
}

/// One poll of a device-code token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll<T> {
    Complete(T),
    Pending,
    /// Back off, to the interval the server names or by five seconds.
    SlowDown {
        interval_secs: Option<u64>,
    },
    Failed(String),
}

/// How often and for how long to poll a device-code grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollSchedule {
    pub interval_secs: Option<u64>,
    pub expires_in_secs: u64,
    /// Wait one interval before the first poll, for servers that refuse an immediate one.
    pub wait_first: bool,
}

/// Poll until the user authorizes, the code expires, or the server refuses.
pub async fn poll_device_code<T, F, Fut>(schedule: PollSchedule, mut poll: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<DevicePoll<T>>>,
{
    let deadline = Instant::now() + Duration::from_secs(schedule.expires_in_secs);
    let mut interval =
        Duration::from_secs(schedule.interval_secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS))
            .max(MINIMUM_POLL_INTERVAL);
    let mut slowed = false;

    if schedule.wait_first {
        tokio::time::sleep(interval.min(deadline.saturating_duration_since(Instant::now()))).await;
    }

    while Instant::now() < deadline {
        match poll().await? {
            DevicePoll::Complete(value) => return Ok(value),
            DevicePoll::Failed(message) => return Err(AuthError::DeviceFlow(message)),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown { interval_secs } => {
                slowed = true;
                interval = match interval_secs.filter(|seconds| *seconds > 0) {
                    Some(seconds) => Duration::from_secs(seconds).max(MINIMUM_POLL_INTERVAL),
                    None => interval + SLOW_DOWN_INCREMENT,
                };
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(interval.min(remaining)).await;
    }

    Err(AuthError::DeviceFlow(match slowed {
        true => "the device code expired after the server asked to slow down; a drifting clock \
                 in a VM or WSL often causes this, so sync it and try again"
            .into(),
        false => "the device code expired before authorization completed".into(),
    }))
}

/// A response read whole, whatever its status.
#[derive(Debug, Clone)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

impl HttpReply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as a JSON object, or an empty one.
    pub fn json(&self) -> Value {
        serde_json::from_str::<Value>(&self.body)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()))
    }
}

/// How long one token-endpoint request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn send(request: reqwest::RequestBuilder) -> Result<HttpReply> {
    let response = request
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| AuthError::OAuth(error.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::OAuth(error.to_string()))?;
    Ok(HttpReply { status, body })
}

pub async fn post_form(
    http: &reqwest::Client,
    url: &str,
    fields: &[(&str, &str)],
) -> Result<HttpReply> {
    send(
        http.post(url)
            .header("accept", "application/json")
            .form(fields),
    )
    .await
}

pub async fn post_json(http: &reqwest::Client, url: &str, body: &Value) -> Result<HttpReply> {
    send(
        http.post(url)
            .header("accept", "application/json")
            .json(body),
    )
    .await
}

/// A non-empty string field.
pub fn string_field(body: &Value, field: &str) -> Option<String> {
    body.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// A positive number field, whether sent as a number or a numeric string.
pub fn positive_number(body: &Value, field: &str) -> Option<f64> {
    let value = body.get(field)?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .filter(|number| number.is_finite() && *number > 0.0)
}

/// What a refusal says for itself, in a line.
pub fn refusal(body: &str) -> String {
    let body = body.trim();
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        for field in ["error_description", "message", "error"] {
            if let Some(said) = value.get(field).and_then(Value::as_str) {
                return said.to_string();
            }
        }
        if let Some(said) = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
        {
            return said.to_string();
        }
    }
    match body {
        "" => "nothing at all".to_string(),
        page if page.starts_with('<') => {
            "a web page rather than an answer, so the trouble is at their end".to_string()
        }
        said => said.chars().take(300).collect(),
    }
}

/// A URL a provider asks the user to open, accepted only over https (or http when `allow_http`), so
/// a response cannot make the browser launch something else.
pub fn trusted_url(raw: &str, allow_http: bool) -> Option<String> {
    let url = reqwest::Url::parse(raw).ok()?;
    match url.scheme() {
        "https" => Some(url.to_string()),
        "http" if allow_http => Some(url.to_string()),
        _ => None,
    }
}

/// Open a URL in the user's browser, saying whether a browser was asked to.
pub fn open_browser(url: &str) -> bool {
    let mut command = match std::env::consts::OS {
        "macos" => std::process::Command::new("open"),
        "windows" => {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        }
        _ => std::process::Command::new("xdg-open"),
    };
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// The claims of a JWT, without verifying it.
pub fn jwt_claims(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let (_, payload, _) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
pub(crate) mod testing {
    //! A local HTTP server that answers each request with the next canned reply and remembers what
    //! it was sent.

    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use tokio::io::AsyncReadExt as _;
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::TcpListener;

    #[derive(Debug, Clone)]
    pub struct Recorded {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Recorded {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).expect("a JSON body")
        }

        pub fn form(&self) -> std::collections::BTreeMap<String, String> {
            reqwest::Url::parse(&format!("http://localhost/?{}", self.body))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect()
        }
    }

    pub struct TestServer {
        pub base: String,
        pub requests: Arc<StdMutex<Vec<Recorded>>>,
    }

    impl TestServer {
        /// Serve `replies` in order, each as `(status, body)`.
        pub async fn start(replies: Vec<(u16, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(StdMutex::new(Vec::new()));
            let recorded = Arc::clone(&requests);
            tokio::spawn(async move {
                for (status, body) in replies {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let request = read_request(&mut stream).await;
                    recorded.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            });
            TestServer { base, requests }
        }

        pub fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        pub fn requests(&self) -> Vec<Recorded> {
            self.requests.lock().unwrap().clone()
        }
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> Recorded {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.unwrap();
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            if read == 0 {
                break buffer.len();
            }
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let mut lines = head.lines();
        let mut first = lines.next().unwrap_or_default().split_whitespace();
        let method = first.next().unwrap_or_default().to_string();
        let path = first.next().unwrap_or_default().to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect();
        let length: usize = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or(0);
        while buffer.len() < header_end + length {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();
        Recorded {
            method,
            path,
            headers,
            body,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_rfc_7636_transform_of_the_verifier() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn every_verifier_is_fresh_and_url_safe() {
        let first = Pkce::generate();
        let second = Pkce::generate();
        assert_ne!(first.verifier, second.verifier);
        assert_eq!(first.verifier.len(), 43);
        assert!(first
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn a_uuid_is_version_four() {
        let uuid = random_uuid();
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "4");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn pasted_input_yields_its_code_and_state_in_every_shape() {
        let expect = |code: &str, state: Option<&str>| AuthorizationInput {
            code: Some(code.into()),
            state: state.map(str::to_string),
        };
        assert_eq!(
            parse_authorization_input("http://localhost:53692/callback?code=abc&state=xyz"),
            expect("abc", Some("xyz"))
        );
        assert_eq!(
            parse_authorization_input("  abc#xyz "),
            expect("abc", Some("xyz"))
        );
        assert_eq!(
            parse_authorization_input("code=abc&state=xyz"),
            expect("abc", Some("xyz"))
        );
        assert_eq!(parse_authorization_input("abc"), expect("abc", None));
        assert_eq!(
            parse_authorization_input("   "),
            AuthorizationInput::default()
        );
    }

    #[test]
    fn a_query_is_percent_encoded() {
        let url = url_with_query(
            "https://example.test/authorize",
            &[("scope", "a b"), ("redirect_uri", "http://localhost:1/cb")],
        );
        assert_eq!(
            url,
            "https://example.test/authorize?scope=a+b&redirect_uri=http%3A%2F%2Flocalhost%3A1%2Fcb"
        );
    }

    fn options(state: Option<&str>) -> CallbackOptions {
        CallbackOptions {
            provider_name: "Example".into(),
            host: "127.0.0.1".into(),
            port: 0,
            path: "/callback".into(),
            redirect_host: Some("localhost".into()),
            state: state.map(str::to_string),
        }
    }

    #[test]
    fn a_callback_must_carry_the_expected_state_and_a_code() {
        let options = options(Some("s1"));

        let (status, _, outcome) = classify_callback("/callback?code=c&state=wrong", &options);
        assert_eq!(status, "400 Bad Request");
        assert!(outcome.is_none(), "a mismatch keeps waiting");

        let (status, _, outcome) = classify_callback("/elsewhere?code=c&state=s1", &options);
        assert_eq!(status, "404 Not Found");
        assert!(outcome.is_none());

        let (_, _, outcome) = classify_callback("/callback?state=s1", &options);
        assert!(outcome.is_none(), "no code, no answer");

        let (_, _, outcome) = classify_callback("/callback?error=access_denied&state=s1", &options);
        assert!(matches!(outcome, Some(Err(_))));

        let (status, page, outcome) = classify_callback("/callback?code=c&state=s1", &options);
        assert_eq!(status, "200 OK");
        assert!(page.contains("Signed in to Example"));
        assert_eq!(outcome.unwrap().unwrap().get("code").unwrap(), "c");
    }

    #[tokio::test]
    async fn the_loopback_server_hands_over_the_redirect() {
        let server = CallbackServer::start(options(Some("s1"))).await.unwrap();
        assert!(server.redirect_uri().starts_with("http://localhost:"));
        let target = server.redirect_uri().replace("localhost", "127.0.0.1");

        let http = reqwest::Client::new();
        let wrong = http
            .get(format!("{target}?code=c&state=nope"))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status().as_u16(), 400);

        let right = http
            .get(format!("{target}?code=the-code&state=s1"))
            .send()
            .await
            .unwrap();
        assert_eq!(right.status().as_u16(), 200);

        let params = server.wait().await.unwrap();
        assert_eq!(params.get("code").unwrap(), "the-code");
    }

    #[tokio::test]
    async fn pasted_input_wins_when_the_browser_never_returns() {
        let server = CallbackServer::start(options(None)).await.unwrap();
        let outcome = callback_or_manual(Some(&server), async { Some("pasted".to_string()) })
            .await
            .unwrap();
        assert_eq!(outcome, CallbackOrManual::Manual("pasted".into()));
    }

    #[tokio::test]
    async fn dismissing_the_prompt_without_a_server_cancels() {
        let outcome = callback_or_manual(None, async { None }).await;
        assert!(matches!(outcome, Err(AuthError::Cancelled)));
    }

    #[tokio::test(start_paused = true)]
    async fn polling_backs_off_when_told_and_finishes_when_granted() {
        let mut answers = vec![
            DevicePoll::Pending,
            DevicePoll::SlowDown {
                interval_secs: None,
            },
            DevicePoll::Complete("token"),
        ]
        .into_iter();
        let started = tokio::time::Instant::now();
        let token = poll_device_code(
            PollSchedule {
                interval_secs: Some(2),
                expires_in_secs: 600,
                wait_first: false,
            },
            || {
                let next = answers.next().unwrap();
                async move { Ok(next) }
            },
        )
        .await
        .unwrap();
        assert_eq!(token, "token");
        assert_eq!(started.elapsed(), Duration::from_secs(2 + 7));
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_poll_ends_the_flow() {
        let error = poll_device_code::<(), _, _>(
            PollSchedule {
                interval_secs: None,
                expires_in_secs: 60,
                wait_first: true,
            },
            || async { Ok(DevicePoll::Failed("denied".into())) },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("denied"), "{error}");
    }

    #[test]
    fn only_web_urls_are_trusted() {
        assert!(trusted_url("https://auth.example/device", false).is_some());
        assert!(trusted_url("http://auth.example/device", false).is_none());
        assert!(trusted_url("http://auth.example/device", true).is_some());
        assert!(trusted_url("file:///etc/passwd", true).is_none());
    }

    #[test]
    fn jwt_claims_are_read_from_the_middle_segment() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"user"}"#);
        let claims = jwt_claims(&format!("h.{payload}.s")).unwrap();
        assert_eq!(claims["sub"], "user");
        assert!(jwt_claims("not-a-jwt").is_none());
    }
}
