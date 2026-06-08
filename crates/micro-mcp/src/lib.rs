//! Tools a separate program provides, over the Model Context Protocol.

use async_trait::async_trait;
use micro_types::ContentBlock;
use micro_types::ToolDefinition;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

/// The revision of the protocol this speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// How long a server has to answer the handshake before it is given up on.
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a single tool call may take.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// What separates a server's name from a tool's in the name the model sees.
const NAME_SEPARATOR: &str = "__";

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

    #[error("{server}: stopped answering")]
    Closed { server: String },

    #[error("{server}: took longer than {seconds}s")]
    TimedOut { server: String, seconds: u64 },
}

pub type Result<T, E = McpError> = std::result::Result<T, E>;

/// One server, as it is written in the config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// The program to run.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Where the server runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// A server listed but turned off stays in the file, so it can be turned back on without being
    /// written out again.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Seconds the handshake may take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout: Option<u64>,
    /// Seconds any one of its tools may take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout: Option<u64>,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: enabled_by_default(),
            startup_timeout: None,
            tool_timeout: None,
        }
    }
}

/// A running server, and the way to ask it things.
pub struct Client {
    name: String,
    outbound: tokio::sync::mpsc::UnboundedSender<String>,
    /// Answers are routed back to whoever asked, by the id they asked with.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
    next_id: AtomicU64,
    call_timeout: Duration,

    _child: Arc<tokio::process::Child>,
}

impl Client {
    /// Start a server and shake hands with it.
    pub async fn start(name: &str, config: &ServerConfig) -> Result<Arc<Client>> {
        let mut command = tokio::process::Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn().map_err(|error| McpError::Start {
            server: name.to_string(),
            command: config.command.clone(),
            message: error.to_string(),
        })?;

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>> = Arc::default();

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

        let reader_pending = Arc::clone(&pending);
        let reader_name = name.to_string();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };

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

        let client = Arc::new(Client {
            name: name.to_string(),
            outbound,
            pending,
            next_id: AtomicU64::new(1),
            call_timeout: config
                .tool_timeout
                .map_or(DEFAULT_CALL_TIMEOUT, Duration::from_secs),
            _child: Arc::new(child),
        });

        let startup = config
            .startup_timeout
            .map_or(DEFAULT_STARTUP_TIMEOUT, Duration::from_secs);
        client
            .request_within(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "micro", "version": env!("CARGO_PKG_VERSION") },
                }),
                startup,
            )
            .await?;
        client.notify("notifications/initialized", json!({}));

        Ok(client)
    }

    /// The tools this server offers, each ready to be called.
    pub async fn tools(self: &Arc<Self>) -> Result<Vec<Arc<dyn micro_tools::Tool>>> {
        let listed = self
            .request_within("tools/list", json!({}), self.call_timeout)
            .await?;

        let tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| McpError::Protocol {
                server: self.name.clone(),
                message: "answered tools/list without a list of tools".to_string(),
            })?;

        Ok(tools
            .iter()
            .filter_map(|tool| {
                let remote = tool.get("name").and_then(Value::as_str)?.to_string();
                Some(Arc::new(RemoteTool {
                    client: Arc::clone(self),
                    definition: ToolDefinition {
                        name: qualified_name(&self.name, &remote),
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
                }) as Arc<dyn micro_tools::Tool>)
            })
            .collect())
    }

    /// Ask for something and wait for the answer.
    async fn request_within(&self, method: &str, params: Value, within: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (answered, answer) = oneshot::channel();
        self.pending.lock().await.insert(id, answered);

        let line = format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        );
        if self.outbound.send(line).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(McpError::Closed {
                server: self.name.clone(),
            });
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

    /// Say something that takes no answer.
    fn notify(&self, method: &str, params: Value) {
        let line = format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "method": method, "params": params })
        );
        let _ = self.outbound.send(line);
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

/// The name the model calls a remote tool by.
pub fn qualified_name(server: &str, tool: &str) -> String {
    format!("mcp{NAME_SEPARATOR}{server}{NAME_SEPARATOR}{tool}")
}

/// One tool belonging to a server, as the agent loop sees it.
struct RemoteTool {
    client: Arc<Client>,
    /// The name the server knows it by, which is not the one the model uses.
    remote: String,
    definition: ToolDefinition,
}

#[async_trait]
impl micro_tools::Tool for RemoteTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
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
        let result = self
            .client
            .request_within(
                "tools/call",
                json!({ "name": self.remote, "arguments": arguments }),
                self.client.call_timeout,
            )
            .await
            .map_err(|error| error.to_string())?;

        let blocks = content_blocks(&result);

        match result.get("isError").and_then(Value::as_bool) {
            Some(true) => Err(blocks.iter().map(ContentBlock::as_text).collect()),
            _ => Ok(blocks),
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

/// Start every server that is turned on, and collect the tools they offer.
pub async fn connect(
    servers: &HashMap<String, ServerConfig>,
) -> (Vec<Arc<dyn micro_tools::Tool>>, Vec<McpError>) {
    let mut names: Vec<&String> = servers.keys().collect();
    names.sort();

    let mut tools = Vec::new();
    let mut problems = Vec::new();
    for name in names {
        let config = &servers[name];
        if !config.enabled {
            continue;
        }
        match Client::start(name, config).await {
            Ok(client) => match client.tools().await {
                Ok(found) => tools.extend(found),
                Err(error) => problems.push(error),
            },
            Err(error) => problems.push(error),
        }
    }
    (tools, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that answers the handshake, offers one tool, and echoes what it is given.
    fn echo_server() -> ServerConfig {
        ServerConfig {
            command: "bash".to_string(),
            args: vec![
                "-c".to_string(),
                r#"
                while IFS= read -r line; do
                  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
                  case "$line" in
                    *'"initialize"'*)
                      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2024-11-05"}}\n' "$id" ;;
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
            ..ServerConfig::default()
        }
    }

    #[tokio::test]
    async fn a_servers_tools_arrive_named_after_it() {
        let client = Client::start("demo", &echo_server())
            .await
            .expect("it starts and shakes hands");
        let tools = client.tools().await.expect("it lists its tools");

        assert_eq!(tools.len(), 1);
        let definition = tools[0].definition();
        assert_eq!(definition.name, "mcp__demo__echo");
        assert_eq!(definition.description, "Say it back");
        assert_eq!(
            definition.parameters["properties"]["text"]["type"],
            "string"
        );
    }

    #[tokio::test]
    async fn calling_one_reaches_the_server_and_comes_back() {
        let client = Client::start("demo", &echo_server()).await.unwrap();
        let tools = client.tools().await.unwrap();

        let said = tools[0]
            .execute(&json!({ "text": "hello" }))
            .await
            .expect("the call succeeded");
        assert_eq!(said, "heard hello");
    }

    #[tokio::test]
    async fn a_server_that_will_not_start_is_reported_and_skipped() {
        let mut servers = HashMap::new();
        servers.insert(
            "broken".to_string(),
            ServerConfig {
                command: "definitely-not-a-program-anyone-has".to_string(),
                ..ServerConfig::default()
            },
        );
        servers.insert("demo".to_string(), echo_server());

        let (tools, problems) = connect(&servers).await;

        assert_eq!(tools.len(), 1, "the working server still offered its tool");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().contains("broken"), "{problems:?}");
    }

    /// A server that is turned off is not started at all.
    #[tokio::test]
    async fn a_disabled_server_is_left_alone() {
        let mut servers = HashMap::new();
        servers.insert(
            "demo".to_string(),
            ServerConfig {
                enabled: false,
                ..echo_server()
            },
        );

        let (tools, problems) = connect(&servers).await;
        assert!(tools.is_empty());
        assert!(problems.is_empty());
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let silent = ServerConfig {
            command: "bash".to_string(),
            args: vec!["-c".to_string(), "sleep 60".to_string()],
            startup_timeout: Some(1),
            ..ServerConfig::default()
        };

        let error = Client::start("silent", &silent)
            .await
            .err()
            .expect("it never shook hands");
        assert!(error.to_string().contains("longer than"), "{error}");
    }
}
