//! The extension host process, and the protocol micro speaks to it.
//!
//! Extensions are TypeScript, so they run where TypeScript runs: a Bun process, started
//! once, holding every extension for the life of the session. micro and the host exchange
//! JSON lines over its stdin and stdout — the same framing the RPC mode uses, for the same
//! reason.
//!
//! Running them in another process is the point rather than a compromise. An extension is
//! someone else's code: out here it cannot reach micro's memory, it can be waited on with
//! a timeout, and it can be killed.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

/// The host script, shipped inside the binary so there is nothing to install.
const HOST_SOURCE: &str = include_str!("../host/host.ts");

/// What the host is written to, under micro's own directory.
const HOST_FILE: &str = "extension-host.ts";

/// How long a tool call may run before micro stops waiting for it.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// What an extension registered, as the host describes it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Registered {
    pub path: String,
    #[serde(default)]
    pub tools: Vec<RegisteredTool>,
    #[serde(default)]
    pub commands: Vec<RegisteredCommand>,
    #[serde(default)]
    pub flags: Vec<RegisteredFlag>,
    #[serde(default)]
    pub shortcuts: Vec<RegisteredShortcut>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub providers: Vec<RegisteredProvider>,
    /// The custom types this extension draws itself.
    #[serde(default)]
    pub renderers: Vec<String>,
}

/// A provider an extension declared, or one it changed.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredProvider {
    pub name: String,
    /// The provider as ohm's `registerProvider` describes it.
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredFlag {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub r#type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredShortcut {
    pub key: String,
    #[serde(default)]
    pub description: String,
}

/// What loading produced: what was registered, and what would not load.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Loaded {
    #[serde(default)]
    pub extensions: Vec<Registered>,
    #[serde(default)]
    pub errors: Vec<LoadFailure>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoadFailure {
    pub path: String,
    pub error: String,
}

/// Something the host wants micro to do, or to answer.
#[derive(Debug, Clone, PartialEq)]
pub enum FromHost {
    /// An extension asked micro to do something. Nothing comes back.
    Action { action: String, payload: Value },
    /// An extension asked micro something and is waiting.
    Request {
        id: String,
        request: String,
        payload: Value,
    },
    /// An extension wants the user asked something.
    Ui { id: Option<String>, payload: Value },
    /// An extension's handler threw.
    Failed {
        path: String,
        event: String,
        error: String,
    },
}

/// A running host.
pub struct Host {
    child: Mutex<Child>,
    /// Behind its own lock, held only while a line is written. Nothing waits for an answer
    /// while holding it, so a call can never block the answer it is waiting for.
    stdin: Mutex<ChildStdin>,
    /// Answers waiting to be matched to the request that asked for them.
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    /// What the host said that micro has to act on, until somebody takes it.
    incoming: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<FromHost>>>,
    loaded: Loaded,
    next_id: std::sync::atomic::AtomicU64,
}

impl Host {
    /// Start the host and load these extensions.
    ///
    /// `bun` is looked for on the path. Without it there are no extensions and the run
    /// carries on: an extension is an addition, and a missing runtime is not a reason to
    /// refuse to start.
    pub async fn start(home: &Path, paths: &[PathBuf]) -> Result<Host, String> {
        if paths.is_empty() {
            return Err("no extensions to load".to_string());
        }
        let runtime = which_bun().ok_or_else(|| {
            "bun is not on the path, so extensions cannot run. Install it from https://bun.sh"
                .to_string()
        })?;

        let script = install_host(home)?;
        let mut child = tokio::process::Command::new(runtime)
            .arg("run")
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start the extension host: {error}"))?;

        let mut stdin = child.stdin.take().ok_or("the host has no stdin")?;
        let stdout = child.stdout.take().ok_or("the host has no stdout")?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (sender, incoming) = tokio::sync::mpsc::unbounded_channel();
        let (loaded_sender, loaded_receiver) = oneshot::channel();

        tokio::spawn(read_host(
            stdout,
            Arc::clone(&pending),
            sender,
            loaded_sender,
        ));

        let listed: Vec<String> = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        write_line(
            &mut stdin,
            &serde_json::json!({ "type": "load", "paths": listed }),
        )
        .await?;

        // Loading runs someone else's code, so it is given a bound rather than waited on
        // for as long as it likes.
        let loaded = tokio::time::timeout(TOOL_TIMEOUT, loaded_receiver)
            .await
            .map_err(|_| "the extension host did not finish loading".to_string())?
            .map_err(|_| "the extension host stopped while loading".to_string())?;

        Ok(Host {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            incoming: Mutex::new(Some(incoming)),
            loaded,
            next_id: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn loaded(&self) -> &Loaded {
        &self.loaded
    }

    /// Every tool the extensions registered, as the model will see them.
    pub fn tools(&self) -> Vec<RegisteredTool> {
        self.loaded
            .extensions
            .iter()
            .flat_map(|extension| extension.tools.iter().cloned())
            .collect()
    }

    /// Every flag the extensions declared.
    pub fn flags(&self) -> Vec<RegisteredFlag> {
        self.loaded
            .extensions
            .iter()
            .flat_map(|extension| extension.flags.iter().cloned())
            .collect()
    }

    /// Tell the extensions what a flag was set to.
    pub async fn set_flag(&self, name: &str, value: Value) -> Result<(), String> {
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "set_flag", "name": name, "value": value }),
        )
        .await
    }

    /// Whether anything registered a way to draw this kind of message.
    pub fn draws(&self, custom_type: &str) -> bool {
        self.loaded
            .extensions
            .iter()
            .any(|extension| extension.renderers.iter().any(|kind| kind == custom_type))
    }

    /// Ask whoever registered it to draw one, at this width.
    ///
    /// What comes back is lines of text: the extension decides what it says, micro decides
    /// where it goes.
    pub async fn render(
        &self,
        custom_type: &str,
        data: &Value,
        width: usize,
    ) -> Result<Vec<String>, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "render",
                "id": id,
                "customType": custom_type,
                "data": data,
                "width": width,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("nothing drew {custom_type} in time"))?
            .map_err(|_| "the extension host stopped while drawing".to_string())?;

        if let Some(error) = answer.get("error").and_then(Value::as_str) {
            return Err(error.to_string());
        }
        Ok(answer
            .get("lines")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Every provider they declared.
    pub fn providers(&self) -> Vec<RegisteredProvider> {
        self.loaded
            .extensions
            .iter()
            .flat_map(|extension| extension.providers.iter().cloned())
            .collect()
    }

    /// Every command they registered.
    pub fn commands(&self) -> Vec<RegisteredCommand> {
        self.loaded
            .extensions
            .iter()
            .flat_map(|extension| extension.commands.iter().cloned())
            .collect()
    }

    /// Tell the extensions something happened. Nothing is waited for.
    pub async fn notify(&self, event: &str, payload: Value) -> Result<(), String> {
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "event", "event": event, "payload": payload }),
        )
        .await
    }

    /// Tell the extensions something happened and wait for what they say about it.
    ///
    /// Unlike [`Host::notify`], this is a question: an extension handling the event may
    /// answer, and what every handler answered comes back. Used where an extension is
    /// allowed to change what happens rather than only to watch it.
    pub async fn ask_event(&self, event: &str, payload: Value) -> Result<Vec<Value>, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "event",
                "id": id,
                "event": event,
                "payload": payload,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("nothing answered {event} in time"))?
            .map_err(|_| format!("the extension host stopped during {event}"))?;

        Ok(answer
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Run one of their tools and wait for what it returns.
    pub async fn call_tool(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "tool_call",
                "id": id,
                "name": name,
                "arguments": arguments,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("{name} did not answer in time"))?
            .map_err(|_| format!("the extension host stopped while running {name}"))?;

        match answer.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(answer
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()),
        }
    }

    /// Run one of their commands.
    pub async fn call_command(&self, name: &str, args: &str) -> Result<Value, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "command", "id": id, "name": name, "args": args }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("/{name} did not answer in time"))?
            .map_err(|_| format!("the extension host stopped while running /{name}"))?;

        match answer.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(answer.get("output").cloned().unwrap_or(Value::Null)),
        }
    }

    /// Answer something the host asked for.
    pub async fn answer(&self, id: &str, payload: Value) -> Result<(), String> {
        let mut message = serde_json::json!({ "type": "answer", "id": id });
        if let (Some(object), Some(extra)) = (message.as_object_mut(), payload.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        write_line(&mut *self.stdin.lock().await, &message).await
    }

    /// Take the stream of things the host wants micro to do.
    ///
    /// Handed over rather than read through the host, because waiting for the next one
    /// would otherwise hold the host's lock for as long as nothing was asked — and
    /// nothing could be answered while it was held.
    pub async fn take_asks(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<FromHost>> {
        self.incoming.lock().await.take()
    }

    /// Tell the extensions the session is over, and let the process go.
    pub async fn shutdown(&self) {
        let _ = write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "shutdown" }),
        )
        .await;
        // A host that will not leave on its own is stopped: it is someone else's code, and
        // a session should not be held open by it.
        let mut child = self.child.lock().await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
        let _ = child.kill().await;
    }

    fn claim_id(&self) -> String {
        let next = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("micro-{next}")
    }
}

/// Read everything the host says: answers go to whoever is waiting, anything else goes to
/// the caller to act on.
async fn read_host(
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    outgoing: tokio::sync::mpsc::UnboundedSender<FromHost>,
    loaded: oneshot::Sender<Loaded>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut loaded = Some(loaded);

    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match kind {
            "loaded" => {
                if let Some(sender) = loaded.take() {
                    let described = serde_json::from_value(message).unwrap_or_default();
                    let _ = sender.send(described);
                }
            }
            "tool_result" | "command_result" | "event_result" | "render_result" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(sender) = pending.lock().await.remove(id) {
                    let _ = sender.send(message);
                }
            }
            "action" => {
                let action = message
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let _ = outgoing.send(FromHost::Action {
                    action,
                    payload: message,
                });
            }
            "request" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let request = message
                    .get("request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let _ = outgoing.send(FromHost::Request {
                    id: id.to_string(),
                    request,
                    payload: message,
                });
            }
            "ui_request" => {
                let id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let _ = outgoing.send(FromHost::Ui {
                    id,
                    payload: message,
                });
            }
            "extension_error" => {
                let _ = outgoing.send(FromHost::Failed {
                    path: text(&message, "path"),
                    event: text(&message, "event"),
                    error: text(&message, "error"),
                });
            }
            _ => {}
        }
    }
}

fn text(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

async fn write_line(stdin: &mut ChildStdin, value: &impl Serialize) -> Result<(), String> {
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    stdin
        .write_all(format!("{encoded}\n").as_bytes())
        .await
        .map_err(|error| format!("cannot reach the extension host: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("cannot reach the extension host: {error}"))
}

/// Put the host script where Bun can run it, and say where that is.
///
/// Rewritten every start rather than only when missing, so an upgraded micro never runs
/// the host an older one left behind.
pub fn install_host(home: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(home)
        .map_err(|error| format!("cannot use {}: {error}", home.display()))?;
    let path = home.join(HOST_FILE);
    std::fs::write(&path, HOST_SOURCE)
        .map_err(|error| format!("cannot write the extension host: {error}"))?;
    Ok(path)
}

/// Where Bun is, if it is anywhere.
pub fn which_bun() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("MICRO_BUN") {
        let path = PathBuf::from(named);
        if path.is_file() {
            return Some(path);
        }
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join("bun"))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_written_where_bun_can_run_it() {
        let home = std::env::temp_dir().join(format!("micro-host-{}", std::process::id()));
        let path = install_host(&home).unwrap();

        assert!(path.ends_with(HOST_FILE));
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, HOST_SOURCE);
        assert!(written.contains("registerTool"), "the API is in there");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn nothing_to_load_is_not_a_host() {
        let home = std::env::temp_dir().join("micro-host-empty");
        let error = match Host::start(&home, &[]).await {
            Err(error) => error,
            Ok(_) => panic!("nothing to load is not a host"),
        };
        assert!(error.contains("no extensions"), "{error}");
    }

    /// A registration is read from what the host says, including a tool's schema.
    #[test]
    fn what_the_host_registered_is_read_back() {
        let described: Loaded = serde_json::from_value(serde_json::json!({
            "extensions": [{
                "path": "/x/hello.ts",
                "tools": [{
                    "name": "greet",
                    "description": "say hello",
                    "parameters": { "type": "object", "properties": { "who": { "type": "string" } } },
                }],
                "commands": [{ "name": "hello", "description": "say it" }],
                "flags": [{ "name": "loud", "description": "shout", "type": "boolean" }],
                "shortcuts": [{ "key": "ctrl+h", "description": "greet" }],
                "events": ["session_start"],
            }],
            "errors": [{ "path": "/x/broken.ts", "error": "boom" }],
        }))
        .unwrap();

        assert_eq!(described.extensions.len(), 1);
        let extension = &described.extensions[0];
        assert_eq!(extension.tools[0].name, "greet");
        assert_eq!(extension.tools[0].parameters["properties"]["who"]["type"], "string");
        assert_eq!(extension.commands[0].name, "hello");
        assert_eq!(extension.flags[0].r#type, "boolean");
        assert_eq!(extension.shortcuts[0].key, "ctrl+h");
        assert_eq!(extension.events, vec!["session_start"]);
        assert_eq!(described.errors[0].path, "/x/broken.ts");
    }

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "micro-host-run-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// An extension is loaded, its tool is called, and what it returned comes back — all
    /// of it through a real Bun process.
    #[tokio::test]
    async fn an_extension_registers_a_tool_and_the_tool_runs() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("tool");
        let extension = root.join("greeter.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "greet",
        description: "say hello to someone",
        parameters: { type: "object", properties: { who: { type: "string" } } },
        execute: async (args) => `hello ${args.who}`,
    });
    micro.registerCommand("wave", { description: "wave back", handler: async () => "waved" });
    micro.on("session_start", () => {});
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, std::slice::from_ref(&extension))
            .await
            .expect("the host starts");

        let loaded = host.loaded();
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        assert_eq!(loaded.extensions.len(), 1);
        assert_eq!(host.tools().len(), 1);
        assert_eq!(host.tools()[0].name, "greet");
        assert_eq!(host.tools()[0].parameters["properties"]["who"]["type"], "string");
        assert_eq!(host.commands()[0].name, "wave");
        assert_eq!(loaded.extensions[0].events, vec!["session_start"]);

        let answer = host
            .call_tool("greet", &serde_json::json!({ "who": "world" }))
            .await
            .expect("the tool answers");
        assert_eq!(answer, "hello world");

        let command = host.call_command("wave", "").await.expect("the command runs");
        assert_eq!(command, serde_json::json!("waved"));

        host.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool that throws reports the failure rather than taking the host down with it.
    #[tokio::test]
    async fn a_tool_that_throws_is_reported_and_the_host_lives() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("throws");
        let extension = root.join("bad.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "explode",
        description: "always fails",
        execute: async () => { throw new Error("it went wrong"); },
    });
    micro.registerTool({
        name: "fine",
        description: "always works",
        execute: async () => "still here",
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension]).await.expect("the host starts");

        let error = host
            .call_tool("explode", &serde_json::json!({}))
            .await
            .expect_err("it throws");
        assert!(error.contains("it went wrong"), "{error}");

        // The host is still there to answer the next call.
        let answer = host.call_tool("fine", &serde_json::json!({})).await.unwrap();
        assert_eq!(answer, "still here");

        host.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file that will not load is reported by name, and the ones beside it still load.
    #[tokio::test]
    async fn a_broken_extension_does_not_stop_the_others() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("broken");
        let broken = root.join("broken.ts");
        let working = root.join("working.ts");
        std::fs::write(&broken, "this is not typescript at all ((((").unwrap();
        std::fs::write(
            &working,
            r#"export default (micro) => { micro.registerTool({ name: "ok", execute: async () => "yes" }); };"#,
        )
        .unwrap();

        let host = Host::start(&root, &[broken.clone(), working])
            .await
            .expect("the host starts");

        assert_eq!(host.loaded().errors.len(), 1, "{:?}", host.loaded().errors);
        assert!(host.loaded().errors[0].path.ends_with("broken.ts"));
        assert_eq!(host.tools().len(), 1, "the working one still registered");

        host.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool micro asks for that nobody registered is answered, not left hanging.
    #[tokio::test]
    async fn a_tool_nobody_registered_is_answered_with_a_reason() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("missing");
        let extension = root.join("empty.ts");
        std::fs::write(&extension, "export default () => {};").unwrap();

        let host = Host::start(&root, &[extension]).await.expect("the host starts");
        let error = host
            .call_tool("nothing-like-this", &serde_json::json!({}))
            .await
            .expect_err("nobody registered it");
        assert!(error.contains("nothing-like-this"), "{error}");

        host.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
    }
}
