//! Tools a separate program provides, over the Model Context Protocol.

pub mod config;
mod log;
pub mod names;
pub mod oauth;
mod servers;
mod transport;

pub use config::Exposure;
pub use config::LoadedConfig;
pub use config::ServerConfig;
pub use config::ServerEntry;
pub use log::ServerLog;
pub use servers::ProviderAuthorizer;
pub use servers::ServerReport;
pub use servers::Servers;
pub use servers::Status;
pub use transport::Authorizer;
pub use transport::TransportError;

use async_trait::async_trait;
use micro_tools::ToolOutput;
use micro_types::ContentBlock;
use micro_types::ToolAnnotations;
use micro_types::ToolDefinition;
use micro_types::ToolExposure;
use micro_types::ToolNamespace;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use transport::Transport;

/// The revision of the protocol this speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long a server has to answer the handshake, or any request but a tool call, when its
/// configuration does not say.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a single tool call may take when the configuration does not say.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// JSON-RPC's code for a method the receiver does not offer.
const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("{server}: cannot start `{command}`: {message}")]
    Start {
        server: String,
        command: String,
        message: String,
    },

    #[error("{server}: {message}")]
    Protocol { server: String, message: String },

    /// The network failed, or the server answered with a status worth another attempt.
    #[error("{server}: {message}")]
    Unavailable { server: String, message: String },

    #[error("{server}: stopped answering")]
    Closed { server: String },

    #[error("{server}: took longer than {seconds}s")]
    TimedOut { server: String, seconds: u64 },

    #[error("{server}: needs sign-in; run `micro mcp login {server}` or `/mcp login {server}`")]
    AuthRequired { server: String },

    #[error("{server}: {message}")]
    Config { server: String, message: String },
}

impl McpError {
    pub fn server(&self) -> &str {
        match self {
            McpError::Start { server, .. }
            | McpError::Protocol { server, .. }
            | McpError::Unavailable { server, .. }
            | McpError::Closed { server }
            | McpError::TimedOut { server, .. }
            | McpError::AuthRequired { server }
            | McpError::Config { server, .. } => server,
        }
    }

    /// Whether connecting again may succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, McpError::Unavailable { .. })
    }

    fn carrying(server: &str, error: TransportError) -> McpError {
        let server = server.to_string();
        let transient = error.is_transient();
        match error {
            TransportError::Closed => McpError::Closed { server },
            TransportError::AuthRequired(_) => McpError::AuthRequired { server },
            TransportError::Http { message, .. }
            | TransportError::Network(message)
            | TransportError::Other(message) => match transient {
                true => McpError::Unavailable { server, message },
                false => McpError::Protocol { server, message },
            },
        }
    }
}

pub type Result<T, E = McpError> = std::result::Result<T, E>;

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

/// A connected server, and the way to ask it things.
pub struct Client {
    name: String,
    transport: Arc<dyn Transport>,
    /// Answers are routed back to whoever asked, by the id they asked with.
    pending: Waiting,
    next_id: AtomicU64,
    timeout: Option<Duration>,
    /// What the server said about itself when it shook hands.
    instructions: Option<String>,
}

impl Client {
    /// Shake hands over a transport whose incoming messages arrive on `incoming`. Log messages
    /// the server sends are appended to `log`.
    async fn connect(
        name: &str,
        transport: Arc<dyn Transport>,
        mut incoming: tokio::sync::mpsc::UnboundedReceiver<Value>,
        timeout: Option<Duration>,
        log: Option<Arc<ServerLog>>,
    ) -> Result<Arc<Client>> {
        let pending: Waiting = Arc::default();

        let reader_pending = Arc::clone(&pending);
        let reader_name = name.to_string();
        let replies: Weak<dyn Transport> = Arc::downgrade(&transport);
        tokio::spawn(async move {
            while let Some(message) = incoming.recv().await {
                if let Some(method) = message.get("method").and_then(Value::as_str) {
                    match (message.get("id"), replies.upgrade()) {
                        (Some(id), Some(transport)) => {
                            let reply = answer_server_request(id, method);
                            let _ = transport.send(&reply).await;
                        }
                        (None, _) if method == "notifications/message" => {
                            if let Some(log) = &log {
                                let params = message.get("params").cloned().unwrap_or(Value::Null);
                                log.message(&reader_name, &params);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                let Some(id) = message.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                let Some(waiting) = reader_pending.lock().await.remove(&id) else {
                    continue;
                };
                let _ = waiting.send(answer(&reader_name, &message));
            }

            let mut held = reader_pending.lock().await;
            for (_, waiting) in held.drain() {
                let _ = waiting.send(Err(McpError::Closed {
                    server: reader_name.clone(),
                }));
            }
        });

        let mut client = Client {
            name: name.to_string(),
            transport,
            pending,
            next_id: AtomicU64::new(1),
            timeout,
            instructions: None,
        };

        let shook = client
            .request_within(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "micro", "version": env!("CARGO_PKG_VERSION") },
                }),
                timeout.unwrap_or(DEFAULT_TIMEOUT),
            )
            .await;
        let shook = match shook {
            Ok(shook) => shook,
            Err(McpError::Closed { server }) => {
                return Err(match client.transport.stderr_tail() {
                    Some(stderr) => McpError::Protocol {
                        server,
                        message: format!("stopped answering:\n{stderr}"),
                    },
                    None => McpError::Closed { server },
                })
            }
            Err(error) => return Err(error),
        };
        if let Some(version) = shook.get("protocolVersion").and_then(Value::as_str) {
            client.transport.set_protocol_version(version);
        }
        client.instructions = shook
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|instructions| !instructions.is_empty())
            .map(str::to_string);
        client
            .transport
            .send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await
            .map_err(|error| McpError::carrying(name, error))?;

        Ok(Arc::new(client))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the server said about itself, if anything.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// The tools this server offers, each ready to be called and exposed as `exposure_of` says
    /// for the server's name for it. `description` says what the server offers, when its
    /// configuration says.
    pub async fn tools(
        self: &Arc<Self>,
        exposure_of: impl Fn(&str) -> Exposure,
        description: Option<String>,
    ) -> Result<Vec<Arc<dyn micro_tools::Tool>>> {
        let mut listed: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let page = self
                .request_within(
                    "tools/list",
                    params,
                    self.timeout.unwrap_or(DEFAULT_TIMEOUT),
                )
                .await?;
            let tools =
                page.get("tools")
                    .and_then(Value::as_array)
                    .ok_or_else(|| McpError::Protocol {
                        server: self.name.clone(),
                        message: "answered tools/list without a list of tools".to_string(),
                    })?;
            listed.extend(tools.iter().cloned());
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|next| !next.is_empty() && cursor.as_deref() != Some(*next))
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }

        let listed: Vec<(String, Value)> = listed
            .into_iter()
            .filter_map(|tool| Some((tool.get("name")?.as_str()?.to_string(), tool)))
            .collect();
        let remote_names: Vec<String> = listed.iter().map(|(name, _)| name.clone()).collect();
        let names = names::tool_names(&self.name, &remote_names);
        let namespace = ToolNamespace {
            name: names::namespace(&self.name),
            description: description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .or_else(|| self.instructions.as_deref())
                .and_then(|text| text.lines().next())
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty()),
            instructions: self.instructions.clone(),
        };

        Ok(listed
            .into_iter()
            .zip(names)
            .map(|((remote, tool), name)| {
                Arc::new(RemoteTool {
                    client: Arc::clone(self),
                    exposure: exposure_of(&remote).tool_exposure(),
                    namespace: namespace.clone(),
                    annotations: ToolAnnotations::from_wire(tool.get("annotations")),
                    output_schema: call_tool_result_schema(tool.get("outputSchema")),
                    definition: ToolDefinition {
                        name,
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),

                        parameters: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),

                        constrained_sampling: None,
                    },
                    remote,
                }) as Arc<dyn micro_tools::Tool>
            })
            .collect())
    }

    /// Ask for something and wait for the answer.
    async fn request_within(&self, method: &str, params: Value, within: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (answered, answer) = oneshot::channel();
        self.pending.lock().await.insert(id, answered);

        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let sent = tokio::time::timeout(within, self.transport.send(&message)).await;
        match sent {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.pending.lock().await.remove(&id);
                return Err(McpError::carrying(&self.name, error));
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(McpError::TimedOut {
                    server: self.name.clone(),
                    seconds: within.as_secs(),
                });
            }
        }

        match tokio::time::timeout(within, answer).await {
            Ok(Ok(result)) => result,

            Ok(Err(_)) => Err(McpError::Closed {
                server: self.name.clone(),
            }),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(McpError::TimedOut {
                    server: self.name.clone(),
                    seconds: within.as_secs(),
                })
            }
        }
    }
}

/// What micro says to a request the server makes of it: `ping` is answered, nothing else is
/// offered.
fn answer_server_request(id: &Value, method: &str) -> Value {
    match method {
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        other => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": METHOD_NOT_FOUND, "message": format!("micro does not offer {other}") },
        }),
    }
}

/// Read a JSON-RPC answer as either a result or the error it carried.
fn answer(server: &str, message: &Value) -> Result<Value> {
    if let Some(error) = message.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("failed without saying why");
        return Err(McpError::Protocol {
            server: server.to_string(),
            message: message.to_string(),
        });
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

/// One tool belonging to a server, as the agent loop sees it.
struct RemoteTool {
    client: Arc<Client>,
    /// The name the server knows it by, which is not the one the model uses.
    remote: String,
    definition: ToolDefinition,
    exposure: ToolExposure,
    /// The server, as the group its tools belong to.
    namespace: ToolNamespace,
    annotations: Option<ToolAnnotations>,
    output_schema: Value,
}

/// The JSON Schema of an MCP `CallToolResult`, with `structured` as the schema of its
/// `structuredContent` when the tool declares one.
fn call_tool_result_schema(structured: Option<&Value>) -> Value {
    let mut properties = json!({
        "_meta": { "type": "object" },
        "content": { "type": "array", "items": { "type": "object" } },
        "isError": { "type": "boolean" },
    });
    if let Some(structured) = structured.filter(|schema| schema.is_object()) {
        properties["structuredContent"] = structured.clone();
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["content"],
    })
}

impl RemoteTool {
    /// Call the tool on the server, and hand back the `CallToolResult` it answered with.
    async fn call_remote(&self, arguments: &Value) -> std::result::Result<Value, String> {
        self.client
            .request_within(
                "tools/call",
                json!({ "name": self.remote, "arguments": arguments }),
                self.client.timeout.unwrap_or(DEFAULT_CALL_TIMEOUT),
            )
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl micro_tools::Tool for RemoteTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn namespace(&self) -> Option<ToolNamespace> {
        Some(self.namespace.clone())
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        self.annotations
    }

    fn output_schema(&self) -> Option<Value> {
        Some(self.output_schema.clone())
    }

    async fn execute(&self, arguments: &Value) -> std::result::Result<String, String> {
        self.execute_content(arguments, &micro_tools::Progress::default())
            .await
            .map(|blocks| blocks.iter().map(ContentBlock::as_text).collect())
    }

    async fn execute_content(
        &self,
        arguments: &Value,
        _progress: &micro_tools::Progress,
    ) -> std::result::Result<Vec<ContentBlock>, String> {
        let result = self.call_remote(arguments).await?;
        let blocks = content_blocks(&result);

        match result.get("isError").and_then(Value::as_bool) {
            Some(true) => Err(blocks.iter().map(ContentBlock::as_text).collect()),
            _ => Ok(blocks),
        }
    }

    /// The model reads the result's content; a script reads the whole `CallToolResult`,
    /// `isError` and `structuredContent` included.
    async fn call(&self, arguments: &Value, _context: &micro_tools::ToolContext<'_>) -> ToolOutput {
        match self.call_remote(arguments).await {
            Err(error) => ToolOutput::error(error),
            Ok(result) => ToolOutput {
                content: content_blocks(&result),
                is_error: result.get("isError").and_then(Value::as_bool) == Some(true),
                structured: Some(result),
                usage: None,
                cost: None,
            },
        }
    }
}

/// What a server said, as blocks the model can read or look at.
fn content_blocks(result: &Value) -> Vec<ContentBlock> {
    let blocks: Vec<ContentBlock> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| content.iter().filter_map(content_block).collect())
        .unwrap_or_default();

    match blocks.is_empty() {
        true => vec![ContentBlock::text("(no output)")],
        false => blocks,
    }
}

fn content_block(block: &Value) -> Option<ContentBlock> {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => Some(ContentBlock::text(
            block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        Some("image") => Some(ContentBlock::Image {
            data: block.get("data").and_then(Value::as_str)?.to_string(),
            mime_type: block
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_string(),
        }),

        Some(other) => Some(ContentBlock::text(format!("({other} content)"))),
        None => None,
    }
}

#[cfg(test)]
mod test_server;

#[cfg(test)]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A server that answers the handshake, offers one tool, and echoes what it is given.
    fn echo_server() -> ServerConfig {
        ServerConfig::stdio(
            "bash",
            vec![
                "-c".to_string(),
                r#"
                while IFS= read -r line; do
                  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
                  case "$line" in
                    *'"initialize"'*)
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","instructions":"Echoes things back.\\nSecond line."}}\n' "$id" ;;
                    *'"tools/list"'*)
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Say it back","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}\n' "$id" ;;
                    *'"tools/call"'*)
                      text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"heard %s"}]}}\n' "$id" "$text" ;;
                  esac
                done
                "#
                .to_string(),
            ],
        )
    }

    fn entry(name: &str, config: ServerConfig) -> ServerEntry {
        ServerEntry {
            name: name.to_string(),
            config,
            source: "mcp.json".into(),
            scope: config::Scope::Global,
        }
    }

    fn servers(entries: Vec<ServerEntry>) -> Servers {
        Servers::new(
            LoadedConfig {
                servers: entries,
                errors: Vec::new(),
            },
            Path::new("."),
        )
    }

    #[tokio::test]
    async fn a_servers_tools_arrive_named_after_it() {
        let servers = servers(vec![entry("de-mo", echo_server())]);
        let tools = servers.connect("de-mo").await.expect("it connects");

        assert_eq!(tools.len(), 1);
        let definition = tools[0].definition();
        assert_eq!(definition.name, "mcp__de_mo__echo");
        assert_eq!(definition.description, "Say it back");
        assert_eq!(
            definition.parameters["properties"]["text"]["type"],
            "string"
        );
        assert_eq!(
            servers.status("de-mo"),
            Some(Status::Connected { tools: 1 })
        );
    }

    #[tokio::test]
    async fn calling_one_reaches_the_server_and_comes_back() {
        let servers = servers(vec![entry("demo", echo_server())]);
        let tools = servers.connect("demo").await.unwrap();

        let said = tools[0]
            .execute(&json!({ "text": "hello" }))
            .await
            .expect("the call succeeded");
        assert_eq!(said, "heard hello");
    }

    /// A script reads the whole result, and the tool says which server it belongs to.
    #[tokio::test]
    async fn a_called_tool_answers_with_its_whole_result_and_names_its_server() {
        let mut config = echo_server();
        config.description = Some("Echo service\nmore".into());
        config.exposure = Some(Exposure::Deferred);
        let servers = servers(vec![entry("demo", config)]);
        let tools = servers.connect("demo").await.unwrap();

        let tool = &tools[0];
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
        let namespace = tool.namespace().expect("it belongs to its server");
        assert_eq!(namespace.name, "mcp__demo");
        assert_eq!(namespace.description.as_deref(), Some("Echo service"));
        assert_eq!(
            namespace.instructions.as_deref(),
            Some("Echoes things back.\nSecond line.")
        );
        let schema = tool.output_schema().expect("results are data");
        assert_eq!(schema["properties"]["isError"]["type"], "boolean");

        let output = tool
            .call(
                &json!({ "text": "hello" }),
                &micro_tools::ToolContext::new("call_1", micro_tools::Progress::default()),
            )
            .await;
        assert!(!output.is_error);
        assert_eq!(output.text_content(), "heard hello");
        assert_eq!(
            output.structured.unwrap()["content"][0]["text"],
            "heard hello"
        );
    }

    /// A tool named in `toolExposure` is exposed as it says, whatever the server's exposure.
    #[tokio::test]
    async fn a_tool_can_be_exposed_apart_from_its_server() {
        let mut config = echo_server();
        config.exposure = Some(Exposure::Hidden);
        config.tool_exposure = [("ec*".to_string(), Exposure::Direct)].into();
        let servers = servers(vec![entry("demo", config)]);
        let tools = servers.connect("demo").await.unwrap();
        assert_eq!(tools[0].exposure(), ToolExposure::Direct);
    }

    /// What a server logs, and what it writes to its standard error, end up in the log file.
    #[tokio::test]
    async fn a_servers_log_messages_and_standard_error_are_kept() {
        let chatty = ServerConfig::stdio(
            "bash",
            vec![
                "-c".to_string(),
                r#"
                while IFS= read -r line; do
                  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
                  case "$line" in
                    *'"initialize"'*)
                      echo 'warming up' >&2
                      printf '{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"notice","data":"index ready"}}\n'
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
                    *'"tools/list"'*)
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id" ;;
                  esac
                done
                "#
                .to_string(),
            ],
        );
        let directory =
            std::env::temp_dir().join(format!("micro-mcp-server-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("mcp.log");
        let servers = servers(vec![entry("chatty", chatty)]).with_log(ServerLog::new(&path));
        servers.connect("chatty").await.unwrap();

        let mut logged = String::new();
        for _ in 0..50 {
            logged = std::fs::read_to_string(&path).unwrap_or_default();
            if logged.contains("warming up") && logged.contains("index ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(logged.contains("[chatty] stderr warming up"), "{logged}");
        assert!(logged.contains("[chatty] notice index ready"), "{logged}");
    }

    /// A server an extension adds while the session runs connects in the background, and taking
    /// it away takes its tools; a configured server of the same name is left alone.
    #[tokio::test]
    async fn an_extension_adds_and_takes_away_a_server() {
        let servers = servers(vec![entry("configured", echo_server())]);
        let registered = config::registered(
            "added",
            &json!({ "command": "bash", "args": ["-c", "exit 1"], "exposure": "deferred" }),
            Path::new("/x/ext.ts"),
        )
        .unwrap();
        assert_eq!(registered.scope, config::Scope::Extension);
        let mut echo = registered.clone();
        echo.config = echo_server();
        echo.config.exposure = Some(Exposure::Deferred);
        servers.register(echo).unwrap();

        let arrivals = servers.arrivals();
        arrivals.settled(Duration::from_secs(10)).await;
        assert!(arrivals.find("mcp__added__echo").is_some());

        servers.unregister("added");
        assert!(arrivals.find("mcp__added__echo").is_none());
        assert!(servers.entry("added").is_none());

        let mut clashing = registered;
        clashing.name = "configured".into();
        assert!(servers.register(clashing).is_err());
        servers.unregister("configured");
        assert!(servers.entry("configured").is_some());
    }

    /// An extension may send a provider credential, which a project file may not.
    #[test]
    fn a_registration_may_name_a_provider_and_yields_to_the_files() {
        let auth =
            json!({ "url": "https://mcp.example.com/mcp", "auth": { "provider": "openai" } });
        let entry = config::registered("hosted", &auth, Path::new("/x/ext.ts")).unwrap();
        let mut loaded = LoadedConfig {
            servers: vec![self::entry("hosted", echo_server())],
            errors: Vec::new(),
        };
        loaded.add_registered(entry.clone());
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].scope, config::Scope::Global);

        let mut renamed = entry;
        renamed.name = "other".into();
        loaded.add_registered(renamed);
        assert_eq!(loaded.servers[1].scope, config::Scope::Extension);
    }

    #[test]
    fn a_declared_output_schema_becomes_the_structured_content() {
        let schema = call_tool_result_schema(Some(&json!({ "type": "object" })));
        assert_eq!(schema["properties"]["structuredContent"]["type"], "object");
        assert!(call_tool_result_schema(None)["properties"]
            .get("structuredContent")
            .is_none());
    }

    #[tokio::test]
    async fn a_server_that_will_not_start_is_reported_and_skipped() {
        let servers = servers(vec![
            entry(
                "broken",
                ServerConfig::stdio("definitely-not-a-program-anyone-has", Vec::new()),
            ),
            entry("demo", echo_server()),
        ]);

        let (tools, problems) = servers.connect_all().await;

        assert_eq!(tools.len(), 1, "the working server still offered its tool");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().contains("broken"), "{problems:?}");
        assert!(matches!(servers.status("broken"), Some(Status::Failed(_))));
    }

    /// A server that is turned off is not started at all.
    #[tokio::test]
    async fn a_disabled_server_is_left_alone() {
        let mut config = echo_server();
        config.enabled = false;
        let servers = servers(vec![entry("demo", config)]);

        let (tools, problems) = servers.connect_all().await;
        assert!(tools.is_empty());
        assert!(problems.is_empty());
        assert_eq!(servers.status("demo"), Some(Status::Disabled));
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let mut silent = ServerConfig::stdio("bash", vec!["-c".into(), "sleep 60".into()]);
        silent.timeout = Some(Duration::from_secs(1));
        let servers = servers(vec![entry("silent", silent)]);

        let error = servers
            .connect("silent")
            .await
            .err()
            .expect("it never shook hands");
        assert!(error.to_string().contains("longer than"), "{error}");
    }

    #[tokio::test]
    async fn a_server_that_dies_says_what_it_printed() {
        let dying = ServerConfig::stdio(
            "bash",
            vec!["-c".into(), "echo 'missing API key' >&2; exit 1".into()],
        );
        let servers = servers(vec![entry("dying", dying)]);
        let error = servers.connect("dying").await.err().unwrap();
        assert!(error.to_string().contains("missing API key"), "{error}");
    }

    /// Deferred servers are listed for the model with what they offer; the first line is enough.
    #[tokio::test]
    async fn undeclared_servers_are_listed_with_what_they_offer() {
        let mut described = echo_server();
        described.description = Some("Search the docs".into());
        let servers = servers(vec![
            entry("docs", described),
            entry("echo-box", echo_server()),
        ]);
        servers.connect("echo-box").await.unwrap();

        let section = servers
            .prompt_section(&["docs".to_string(), "echo-box".to_string()], false)
            .expect("there is something to list");
        assert!(section.contains("`tool_search`"), "{section}");
        assert!(
            section.contains("- mcp__docs: Search the docs"),
            "{section}"
        );
        assert!(
            section.contains("- mcp__echo_box: Echoes things back."),
            "{section}"
        );
        assert!(!section.contains("Second line"), "{section}");
        assert!(servers.prompt_section(&[], false).is_none());
        let scripted = servers.prompt_section(&["docs".to_string()], true).unwrap();
        assert!(scripted.contains("`codemode` scripts"), "{scripted}");
        assert!(scripted.contains("`searchTools()`"), "{scripted}");
    }
}
