//! The extension host process, and the protocol micro speaks to it.

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

/// The host, shipped inside the binary so there is nothing to install.
const HOST_SOURCE: &[(&str, &str)] = &[
    ("extension-host.ts", include_str!("../host/host.ts")),
    ("host-context.ts", include_str!("../host/context.ts")),
    ("host-ui.ts", include_str!("../host/ui.ts")),
    ("host-tools.ts", include_str!("../host/tools.ts")),
    ("host-wire.ts", include_str!("../host/wire.ts")),
    ("host-components.ts", include_str!("../host/components.ts")),
];

/// What the host is entered through, under micro's own directory.
const HOST_FILE: &str = "extension-host.ts";

/// How long a tool call may run before micro stops waiting for it.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a live component may take to answer before it is treated as unreachable.
const COMPONENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a host is given to finish its shutdown callback before it is stopped.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// A blocked protocol writer must not prevent the host from being reaped.
const SHUTDOWN_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

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
    /// What the extension itself says it may do, when it says anything: a `capabilities` export
    /// beside its default one.
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
}

/// A provider an extension declared, or one it changed.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredProvider {
    pub name: String,
    /// The provider as pi's `registerProvider` describes it.
    pub config: Value,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegisteredTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Value,
    /// A human-readable name for a UI to show instead of `name`.
    #[serde(default)]
    pub label: Option<String>,
    /// A one-line summary for the "Available tools" section of a system prompt, and the guideline
    /// bullets that go with it.
    #[serde(default)]
    pub prompt_snippet: Option<String>,
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
    /// A provider-side sampling directive for this tool's arguments.
    #[serde(default)]
    pub constrained_sampling: Option<Value>,

    #[serde(default)]
    pub render_shell: Option<String>,
    /// `"sequential"` or `"parallel"`.
    #[serde(default)]
    pub execution_mode: Option<String>,
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
    /// An extension asked micro to do something.
    Action {
        action: String,
        extension: Option<String>,
        payload: Value,
    },
    /// An extension asked micro something and is waiting.
    Request {
        id: String,
        request: String,
        extension: Option<String>,
        payload: Value,
    },
    /// An extension wants the user asked something.
    Ui {
        id: Option<String>,
        extension: Option<String>,
        payload: Value,
    },

    ComponentChanged {
        component_id: String,
    },
    /// An extension's handler threw.
    Failed {
        path: String,
        event: String,
        error: String,
    },
}

/// What renderCall/renderResult are told about the tool call beyond their own first argument.
#[derive(Debug, Clone, Default)]
pub struct ToolRenderFields {
    pub tool_call_id: String,
    pub cwd: String,
    pub execution_started: bool,
    pub args_complete: bool,
    pub is_partial: bool,
    pub expanded: bool,
    pub show_images: bool,
    pub is_error: bool,
}

/// What running a tool's renderCall or renderResult answered back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedTool {
    pub component_id: Option<String>,
    pub supported: bool,
    pub error: Option<String>,
}

/// A running host.
pub struct Host {
    child: Mutex<Child>,
    /// Behind its own lock, held only while a line is written.
    stdin: Arc<Mutex<ChildStdin>>,
    /// Answers waiting to be matched to the request that asked for them.
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,

    updates: Arc<Mutex<HashMap<String, micro_tools::Progress>>>,
    /// Tells a call still in flight to stop: a turn that was abandoned, or one that ran past
    /// `TOOL_TIMEOUT`.
    cancel: tokio::sync::mpsc::UnboundedSender<String>,
    /// What the host said that micro has to act on, until somebody takes it.
    incoming: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<FromHost>>>,
    loaded: Loaded,
    next_id: std::sync::atomic::AtomicU64,
}

impl Host {
    /// Start the host and load these extensions.
    pub async fn start(
        home: &Path,
        paths: &[PathBuf],
        workspace: &Path,
        has_ui: bool,
        trusted: bool,
        mode: &str,
    ) -> Result<Host, String> {
        if paths.is_empty() {
            return Err("no extensions to load".to_string());
        }
        let runtime = which_bun().ok_or_else(|| {
            "bun is not on the path, so extensions cannot run. Install it from https://bun.sh"
                .to_string()
        })?;

        let script = install_host(home)?;

        let home = &std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
        let script = std::fs::canonicalize(&script).unwrap_or(script);

        let compat_node_modules = crate::compat::install(home)?;
        let node_path = crate::compat::node_path(home, &compat_node_modules)?;
        let workspace_compat = crate::compat::install(&workspace.join(".micro"))?;
        let mut readable_roots = extension_read_roots(home, paths, workspace);
        readable_roots.push(compat_node_modules.clone());
        readable_roots.push(workspace_compat);
        readable_roots.push(workspace.to_path_buf());
        let sandbox = match trusted {
            true => {
                micro_sandbox::Sandbox::trusted_extension_host(&runtime, readable_roots, workspace)
            }
            false => micro_sandbox::Sandbox::extension_host(&runtime, readable_roots),
        };
        if !sandbox.is_enforced() {
            return Err(
                "extensions are disabled because this platform cannot confine the Bun host"
                    .to_string(),
            );
        }
        let runtime_name = runtime.to_string_lossy().into_owned();
        let wrapped = sandbox.wrap(
            &runtime_name,
            [
                "run".to_string(),
                "--no-install".to_string(),
                script.display().to_string(),
                "--micro-home".to_string(),
                home.display().to_string(),
            ],
            home,
        );
        if !wrapped.enforced {
            return Err(
                "extensions are disabled because the Bun host would be unconfined".to_string(),
            );
        }
        let mut command = tokio::process::Command::new(&wrapped.program);
        command
            .args(&wrapped.args)
            .current_dir(&wrapped.cwd)
            .kill_on_drop(true)
            .env_clear();
        for (name, value) in &wrapped.env {
            command.env(name, value);
        }
        let mut child = command
            .env("NODE_PATH", node_path)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start the extension host: {error}"))?;

        let stdin = child.stdin.take().ok_or("the host has no stdin")?;
        let stdin = Arc::new(Mutex::new(stdin));
        let stdout = child.stdout.take().ok_or("the host has no stdout")?;

        let complaint = Arc::new(Mutex::new(String::new()));
        let complaining = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(read_complaint(stderr, Arc::clone(&complaint))));

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let updates: Arc<Mutex<HashMap<String, micro_tools::Progress>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (sender, incoming) = tokio::sync::mpsc::unbounded_channel();
        let (loaded_sender, loaded_receiver) = oneshot::channel();

        tokio::spawn(read_host(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&updates),
            sender,
            loaded_sender,
        ));

        let (cancel, mut cancelled) = tokio::sync::mpsc::unbounded_channel::<String>();
        {
            let stdin = Arc::clone(&stdin);
            tokio::spawn(async move {
                while let Some(id) = cancelled.recv().await {
                    let _ = write_line(
                        &mut *stdin.lock().await,
                        &serde_json::json!({ "type": "abort_tool", "id": id }),
                    )
                    .await;
                }
            });
        }

        let listed: Vec<String> = paths.iter().map(|path| resolved(path)).collect();
        if let Err(error) = write_line(
            &mut *stdin.lock().await,
            &serde_json::json!({
                "type": "load",
                "paths": listed,
                "cwd": resolved(workspace),
                "has_ui": has_ui,
                "trusted": trusted,
                "mode": mode,
            }),
        )
        .await
        {
            stop_child(&mut child).await;
            return Err(error);
        }

        let loaded = match tokio::time::timeout(TOOL_TIMEOUT, loaded_receiver).await {
            Ok(Ok(loaded)) => loaded,

            Ok(Err(_)) => {
                let error = with_complaint(
                    "the extension host stopped while loading",
                    &complaint,
                    complaining,
                )
                .await;
                stop_child(&mut child).await;
                return Err(error);
            }
            Err(_) => {
                let error = with_complaint(
                    "the extension host did not finish loading",
                    &complaint,
                    complaining,
                )
                .await;
                stop_child(&mut child).await;
                return Err(error);
            }
        };

        Ok(Host {
            child: Mutex::new(child),
            stdin,
            pending,
            updates,
            cancel,
            incoming: Mutex::new(Some(incoming)),
            loaded,
            next_id: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn loaded(&self) -> &Loaded {
        &self.loaded
    }

    /// Drop every registration an extension was not granted, and say what was refused.
    pub fn retain_granted(&mut self, grants: &crate::Grants) -> Vec<String> {
        let mut refused = Vec::new();
        for extension in &mut self.loaded.extensions {
            let Some(grant) = grants.grant(Some(&extension.path)) else {
                continue;
            };
            let mut refuse = |kind: &str, named: Vec<String>| {
                if !named.is_empty() {
                    refused.push(format!(
                        "{} registered {kind} without asking for the `{kind}` capability, so {} left out: {}",
                        grant.name,
                        match named.len() {
                            1 => "it was",
                            _ => "they were",
                        },
                        named.join(", "),
                    ));
                }
            };

            if !grant.allows(crate::Capability::Tools) {
                refuse(
                    "tools",
                    extension.tools.drain(..).map(|tool| tool.name).collect(),
                );
            }
            if !grant.allows(crate::Capability::Commands) {
                refuse(
                    "commands",
                    extension
                        .commands
                        .drain(..)
                        .map(|command| command.name)
                        .collect(),
                );
            }
            if !grant.allows(crate::Capability::Flags) {
                refuse(
                    "flags",
                    extension.flags.drain(..).map(|flag| flag.name).collect(),
                );
            }
            if !grant.allows(crate::Capability::Providers) {
                refuse(
                    "providers",
                    extension
                        .providers
                        .drain(..)
                        .map(|provider| provider.name)
                        .collect(),
                );
            }
        }
        refused
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

    /// Tell the extensions something happened.
    pub async fn notify(&self, event: &str, payload: Value) -> Result<(), String> {
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "event", "event": event, "payload": payload }),
        )
        .await
    }

    /// Tell the extensions something happened and wait for what they say about it.
    pub async fn ask_event(&self, event: &str, payload: Value) -> Result<Vec<Value>, String> {
        Ok(self
            .ask_event_attributed(event, payload)
            .await?
            .into_iter()
            .map(|(_, answer)| answer)
            .collect())
    }

    /// The same question, with each answer paired to the extension that gave it.
    pub async fn ask_event_attributed(
        &self,
        event: &str,
        payload: Value,
    ) -> Result<Vec<(Option<String>, Value)>, String> {
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

        let results = answer
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let sources = answer
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(results
            .into_iter()
            .enumerate()
            .map(|(index, result)| {
                let source = sources
                    .get(index)
                    .and_then(Value::as_str)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string);
                (source, result)
            })
            .collect())
    }

    /// Tell one extension it is being let go, and wait for it to say it is done.
    pub async fn deactivate(&self, path: &str) -> Result<(), String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        let path = &resolved(Path::new(path));
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "deactivate", "id": id, "path": path }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("{path} did not finish deactivating in time"))?
            .map_err(|_| "the extension host stopped while deactivating".to_string())?;

        match answer.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(()),
        }
    }

    /// Run one of their tools, forwarding what it reports while it works, and wait for what it
    /// returns.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: &Value,
        progress: &micro_tools::Progress,
    ) -> Result<Vec<micro_types::ContentBlock>, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);
        self.updates
            .lock()
            .await
            .insert(id.clone(), progress.clone());

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

        let guard = CancelOnDrop::new(id.clone(), self.cancel.clone());
        let outcome = tokio::time::timeout(TOOL_TIMEOUT, receiver).await;

        guard.disarm();
        self.updates.lock().await.remove(&id);

        let answer = match outcome {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => return Err(format!("the extension host stopped while running {name}")),
            Err(_) => {
                let _ = self.cancel.send(id.clone());
                self.pending.lock().await.remove(&id);
                return Err(format!("{name} did not answer in time"));
            }
        };

        match answer.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(content_blocks(&answer)),
        }
    }

    /// Ask a registered markdown transformer to rewrite this text, and hand back what it produced.
    pub async fn transform_markdown(
        &self,
        markdown: &str,
        context: &Value,
    ) -> Result<String, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "transform_markdown",
                "id": id,
                "markdown": markdown,
                "context": context,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(TOOL_TIMEOUT, receiver)
            .await
            .map_err(|_| "no markdown transformer answered in time".to_string())?
            .map_err(|_| "the extension host stopped while transforming markdown".to_string())?;

        match answer.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(answer
                .get("markdown")
                .and_then(Value::as_str)
                .unwrap_or(markdown)
                .to_string()),
        }
    }

    /// Ask a registered component for its lines at this width.
    pub async fn render_component(
        &self,
        component_id: &str,
        width: usize,
    ) -> Result<Vec<String>, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "component",
                "id": id,
                "method": "render",
                "componentId": component_id,
                "width": width,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(COMPONENT_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("component {component_id} did not answer in time"))?
            .map_err(|_| "the extension host stopped while rendering a component".to_string())?;

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

    /// Offer a registered component a key, and say whether it consumed it.
    pub async fn send_component_input(
        &self,
        component_id: &str,
        data: &str,
        text: Option<&str>,
    ) -> Result<bool, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({
                "type": "component",
                "id": id,
                "method": "input",
                "componentId": component_id,
                "data": data,
                "text": text,
            }),
        )
        .await?;

        let answer = tokio::time::timeout(COMPONENT_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("component {component_id} did not answer in time"))?
            .map_err(|_| {
                "the extension host stopped while offering a component input".to_string()
            })?;

        Ok(answer
            .get("consume")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Tell a registered component to drop any cached rendering state of its own.
    pub async fn invalidate_component(&self, component_id: &str) -> Result<(), String> {
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "component", "method": "invalidate", "componentId": component_id }),
        )
        .await
    }

    /// Tell the host a registered component is no longer needed.
    pub async fn dispose_component(&self, component_id: &str) -> Result<(), String> {
        write_line(
            &mut *self.stdin.lock().await,
            &serde_json::json!({ "type": "component", "method": "dispose", "componentId": component_id }),
        )
        .await
    }

    /// Ask the host to run this tool's renderCall with these arguments and context, and register
    /// whatever Component it returns.
    pub async fn render_tool_call(
        &self,
        name: &str,
        args: &Value,
        fields: &ToolRenderFields,
    ) -> Result<RenderedTool, String> {
        self.render_tool(name, "call", args, None, fields).await
    }

    pub async fn render_tool_result(
        &self,
        name: &str,
        args: &Value,
        result: &Value,
        fields: &ToolRenderFields,
    ) -> Result<RenderedTool, String> {
        self.render_tool(name, "result", args, Some(result), fields)
            .await
    }

    async fn render_tool(
        &self,
        name: &str,
        kind: &str,
        args: &Value,
        result: Option<&Value>,
        fields: &ToolRenderFields,
    ) -> Result<RenderedTool, String> {
        let id = self.claim_id();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        let mut message = serde_json::json!({
            "type": "render_tool",
            "id": id,
            "kind": kind,
            "name": name,
            "args": args,
            "toolCallId": fields.tool_call_id,
            "cwd": fields.cwd,
            "executionStarted": fields.execution_started,
            "argsComplete": fields.args_complete,
            "isPartial": fields.is_partial,
            "expanded": fields.expanded,
            "showImages": fields.show_images,
            "isError": fields.is_error,
        });
        if let Some(result) = result {
            message["result"] = result.clone();
        }

        write_line(&mut *self.stdin.lock().await, &message).await?;

        let answer = tokio::time::timeout(COMPONENT_TIMEOUT, receiver)
            .await
            .map_err(|_| format!("{name}'s renderer did not answer in time"))?
            .map_err(|_| "the extension host stopped while rendering a tool call".to_string())?;

        Ok(RenderedTool {
            component_id: answer
                .get("componentId")
                .and_then(Value::as_str)
                .map(str::to_string),
            supported: answer
                .get("supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: answer
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
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
    pub async fn take_asks(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<FromHost>> {
        self.incoming.lock().await.take()
    }

    /// Tell the extensions the session is over, and let the process go.
    pub async fn shutdown(&self, reason: &str) {
        let _ = tokio::time::timeout(SHUTDOWN_WRITE_TIMEOUT, async {
            write_line(
                &mut *self.stdin.lock().await,
                &serde_json::json!({ "type": "shutdown", "reason": reason }),
            )
            .await
        })
        .await;

        let mut child = self.child.lock().await;
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await;
        stop_child(&mut child).await;
    }

    fn claim_id(&self) -> String {
        let next = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("micro-{next}")
    }
}

/// Stop a child and wait for it so a failed host cannot outlive its owner or remain unreaped.
async fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

/// A path as the confined host will see it.
///
/// The sandbox is written against resolved paths, and a rule for `/private/var/…` does not admit
/// the `/var/…` that reaches it through a symlink. Anything handed across that boundary — the
/// script, the directory it runs in, the extensions it is asked to load — goes in resolved form, or
/// the host is denied a file whose name it was just given.
fn resolved(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// The most of the host's complaint worth repeating, so a runaway stack trace cannot become the
/// whole message.
const COMPLAINT_LIMIT: usize = 4_000;

/// Keep what the host writes to its error stream. A host that dies on the way up says why there and
/// nowhere else, so without this its last words are lost and every failure reads the same.
async fn read_complaint(stderr: tokio::process::ChildStderr, complaint: Arc<Mutex<String>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut held = complaint.lock().await;
        if held.len() >= COMPLAINT_LIMIT {
            return;
        }
        if !held.is_empty() {
            held.push('\n');
        }
        held.push_str(&line);
    }
}

/// How long a dying host is given to finish saying why.
const COMPLAINT_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// `reason`, and what the host said for itself if it said anything.
///
/// A host that dies on the way up closes both its streams at once, so the reading of its error
/// stream is waited out here rather than raced: whatever it managed to say is the whole diagnosis.
async fn with_complaint(
    reason: &str,
    complaint: &Arc<Mutex<String>>,
    complaining: Option<tokio::task::JoinHandle<()>>,
) -> String {
    if let Some(complaining) = complaining {
        let _ = tokio::time::timeout(COMPLAINT_GRACE, complaining).await;
    }
    let said = complaint.lock().await;
    match said.trim().is_empty() {
        true => reason.to_string(),
        false => format!("{reason}: {}", said.trim()),
    }
}

/// Read everything the host says: answers go to whoever is waiting, anything else goes to the
/// caller to act on.
async fn read_host(
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    updates: Arc<Mutex<HashMap<String, micro_tools::Progress>>>,
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
            "tool_result"
            | "command_result"
            | "event_result"
            | "render_result"
            | "transform_markdown_result"
            | "component_result"
            | "render_tool_result"
            | "deactivated" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(sender) = pending.lock().await.remove(id) {
                    let _ = sender.send(message);
                }
            }

            "component_changed" => {
                let Some(component_id) = message.get("componentId").and_then(Value::as_str) else {
                    continue;
                };
                let _ = outgoing.send(FromHost::ComponentChanged {
                    component_id: component_id.to_string(),
                });
            }

            "tool_update" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(progress) = updates.lock().await.get(id) {
                    let text: String = content_blocks(&message)
                        .iter()
                        .map(micro_types::ContentBlock::as_text)
                        .collect();
                    progress.report(text);
                }
            }
            "action" => {
                let action = message
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let extension = asker(&message);
                let _ = outgoing.send(FromHost::Action {
                    action,
                    extension,
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
                let extension = asker(&message);
                let _ = outgoing.send(FromHost::Request {
                    id: id.to_string(),
                    request,
                    extension,
                    payload: message,
                });
            }
            "ui_request" => {
                let id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let extension = asker(&message);
                let _ = outgoing.send(FromHost::Ui {
                    id,
                    extension,
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

/// Which extension made this ask, when the host said.
fn asker(message: &Value) -> Option<String> {
    message
        .get("extension")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn text(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The `content` array a `tool_result` or `tool_update` message carries, read as the content blocks
/// the rest of micro works with.
fn content_blocks(message: &Value) -> Vec<micro_types::ContentBlock> {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .map(|block| match block.get("type").and_then(Value::as_str) {
            Some("image") => micro_types::ContentBlock::Image {
                data: text(block, "data"),
                mime_type: text(block, "mimeType"),
            },
            _ => micro_types::ContentBlock::text(text(block, "text")),
        })
        .collect()
}

/// Tells the host to stop a call this side gave up on before the call itself finished.
struct CancelOnDrop {
    id: String,
    cancel: tokio::sync::mpsc::UnboundedSender<String>,
    armed: bool,
}

impl CancelOnDrop {
    fn new(id: String, cancel: tokio::sync::mpsc::UnboundedSender<String>) -> Self {
        CancelOnDrop {
            id,
            cancel,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cancel.send(std::mem::take(&mut self.id));
        }
    }
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
pub fn install_host(home: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(home)
        .map_err(|error| format!("cannot use {}: {error}", home.display()))?;
    for (name, source) in HOST_SOURCE {
        std::fs::write(home.join(name), source)
            .map_err(|error| format!("cannot write the extension host: {error}"))?;
    }
    Ok(home.join(HOST_FILE))
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

fn extension_read_roots(home: &Path, paths: &[PathBuf], workspace: &Path) -> Vec<PathBuf> {
    let mut roots = vec![home.to_path_buf()];
    for path in paths {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if path.is_dir() {
            roots.push(path);
            continue;
        }
        let parent = path.parent().unwrap_or(path.as_path());
        let package_root = parent
            .ancestors()
            .find(|ancestor| *ancestor != workspace && ancestor.join("package.json").is_file());
        roots.push(package_root.unwrap_or(parent).to_path_buf());
    }
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_written_where_bun_can_run_it() {
        let home = std::env::temp_dir().join(format!("micro-host-{}", std::process::id()));
        let path = install_host(&home).unwrap();

        assert!(path.ends_with(HOST_FILE));

        for (name, source) in HOST_SOURCE {
            let written = std::fs::read_to_string(home.join(name)).unwrap();
            assert_eq!(&written, source, "{name} was not written whole");
        }
        let entry = std::fs::read_to_string(&path).unwrap();
        assert!(entry.contains("registerTool"), "the API is in there");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn nothing_to_load_is_not_a_host() {
        let home = std::env::temp_dir().join("micro-host-empty");
        let error = match Host::start(&home, &[], &home, false, false, "tui").await {
            Err(error) => error,
            Ok(_) => panic!("nothing to load is not a host"),
        };
        assert!(error.contains("no extensions"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_a_child_reaps_it() {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "exec sleep 60"])
            .kill_on_drop(true)
            .spawn()
            .expect("the child starts");

        stop_child(&mut child).await;

        assert!(child.try_wait().unwrap().is_some(), "the child was reaped");
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
        assert_eq!(
            extension.tools[0].parameters["properties"]["who"]["type"],
            "string"
        );
        assert_eq!(extension.commands[0].name, "hello");
        assert_eq!(extension.flags[0].r#type, "boolean");
        assert_eq!(extension.shortcuts[0].key, "ctrl+h");
        assert_eq!(extension.events, vec!["session_start"]);
        assert_eq!(described.errors[0].path, "/x/broken.ts");
    }

    /// The text a call_tool answer carries, joined the way the model would read it.
    fn as_text(blocks: &[micro_types::ContentBlock]) -> String {
        blocks
            .iter()
            .map(micro_types::ContentBlock::as_text)
            .collect()
    }

    /// Answer every `get_context` the host asks for, for as long as the test runs.
    fn answer_context_requests(host: Arc<Host>) {
        tokio::spawn(async move {
            let Some(mut asks) = host.take_asks().await else {
                return;
            };
            while let Some(asked) = asks.recv().await {
                if let FromHost::Request { id, request, .. } = asked {
                    if request == "get_context" {
                        let _ = host
                            .answer(&id, serde_json::json!({ "thinkingLevel": "off" }))
                            .await;
                    }
                }
            }
        });
    }

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "micro-host-run-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap_or(path)
    }

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
        // pi's argument order: the id this call was given, then the arguments.
        execute: async (toolCallId, args) => `hello ${args.who}`,
    });
    micro.registerCommand("wave", { description: "wave back", handler: async () => "waved" });
    micro.on("session_start", () => {});
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(
                &root,
                std::slice::from_ref(&extension),
                &root,
                false,
                true,
                "tui",
            )
            .await
            .expect("the host starts"),
        );
        answer_context_requests(Arc::clone(&host));

        let loaded = host.loaded();
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        assert_eq!(loaded.extensions.len(), 1);
        assert_eq!(host.tools().len(), 1);
        assert_eq!(host.tools()[0].name, "greet");
        assert_eq!(
            host.tools()[0].parameters["properties"]["who"]["type"],
            "string"
        );
        assert_eq!(host.commands()[0].name, "wave");
        assert_eq!(loaded.extensions[0].events, vec!["session_start"]);

        let answer = host
            .call_tool(
                "greet",
                &serde_json::json!({ "who": "world" }),
                &micro_tools::Progress::default(),
            )
            .await
            .expect("the tool answers");
        assert_eq!(as_text(&answer), "hello world");

        let command = host
            .call_command("wave", "")
            .await
            .expect("the command runs");
        assert_eq!(command, serde_json::json!("waved"));

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_extension_resolves_the_bundled_pi_tui_package() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("pi-tui");
        let home = root.join("home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&home).unwrap();
        let extension = workspace.join(".micro/extensions/pi-tui.ts");
        std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
        std::fs::write(
            &extension,
            r#"
import { visibleWidth } from "@earendil-works/pi-tui";
export default (micro) => {
    micro.registerCommand("probe", { handler: async () => String(visibleWidth("hello")) });
};
"#,
        )
        .unwrap();

        let host = Host::start(
            &home,
            std::slice::from_ref(&extension),
            &workspace,
            true,
            false,
            "tui",
        )
        .await
        .expect("the host starts");
        assert!(
            host.loaded().errors.is_empty(),
            "{:?}",
            host.loaded().errors
        );
        assert_eq!(host.call_command("probe", "").await.unwrap(), "5");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn what_an_extension_declares_about_itself_reaches_micro() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("declared");
        let declaring = root.join("declaring.ts");
        std::fs::write(
            &declaring,
            r#"
export const capabilities = ["tools", "exec"];
export default (micro) => {
    micro.registerTool({
        name: "thing",
        description: "does a thing",
        parameters: { type: "object", properties: {} },
        execute: async () => "done",
    });
};
"#,
        )
        .unwrap();
        let silent = root.join("silent.ts");
        std::fs::write(&silent, "export default () => {};").unwrap();

        let host = Host::start(
            &root,
            &[declaring.clone(), silent.clone()],
            &root,
            false,
            false,
            "tui",
        )
        .await
        .expect("the host starts");

        let loaded = host.loaded();
        assert_eq!(
            loaded.extensions[0].capabilities,
            Some(vec!["tools".to_string(), "exec".to_string()])
        );
        assert_eq!(loaded.extensions[1].capabilities, None);

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Letting an extension go runs its own `deactivate`.
    #[tokio::test]
    async fn deactivating_an_extension_runs_its_own_teardown_and_drops_its_registrations() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("deactivate");
        let extension = root.join("leaving.ts");
        let marker = root.join("left.txt");
        std::fs::write(
            &extension,
            format!(
                r#"
import {{ writeFileSync }} from "node:fs";
export const deactivate = () => {{
    writeFileSync({marker:?}, "put back");
}};
export default (micro) => {{
    micro.registerCommand("probe", {{ handler: async () => "here" }});
}};
"#,
                marker = marker.display().to_string()
            ),
        )
        .unwrap();

        let host = Arc::new(
            Host::start(
                &root,
                std::slice::from_ref(&extension),
                &root,
                false,
                true,
                "tui",
            )
            .await
            .expect("the host starts"),
        );
        answer_context_requests(Arc::clone(&host));

        assert_eq!(
            host.call_command("probe", "")
                .await
                .expect("the command runs"),
            serde_json::json!("here")
        );

        host.deactivate(&extension.display().to_string())
            .await
            .expect("it is let go");
        assert_eq!(
            std::fs::read_to_string(&marker).expect("its own teardown ran"),
            "put back"
        );

        let gone = host.call_command("probe", "").await;
        assert!(
            gone.is_err_and(|error| error.contains("probe")),
            "the command it registered is still there"
        );

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

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

        let host = Arc::new(
            Host::start(&root, &[extension], &root, false, true, "tui")
                .await
                .expect("the host starts"),
        );
        answer_context_requests(Arc::clone(&host));

        let error = host
            .call_tool(
                "explode",
                &serde_json::json!({}),
                &micro_tools::Progress::default(),
            )
            .await
            .expect_err("it throws");
        assert!(error.contains("it went wrong"), "{error}");

        let answer = host
            .call_tool(
                "fine",
                &serde_json::json!({}),
                &micro_tools::Progress::default(),
            )
            .await
            .unwrap();
        assert_eq!(as_text(&answer), "still here");

        host.shutdown("quit").await;
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

        let host = Host::start(
            &root,
            &[broken.clone(), working],
            &root,
            false,
            false,
            "tui",
        )
        .await
        .expect("the host starts");

        assert_eq!(host.loaded().errors.len(), 1, "{:?}", host.loaded().errors);
        assert!(host.loaded().errors[0].path.ends_with("broken.ts"));
        assert_eq!(host.tools().len(), 1, "the working one still registered");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_missing_declared_dependency_says_how_to_fix_it() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("missing-dependency");
        let extension = root.join("with-deps.ts");
        std::fs::write(
            &extension,
            r#"import { format } from "left-pad-fake-dependency";
export default (micro) => { micro.registerTool({ name: "uses_it", execute: async () => format(1) }); };
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{ "name": "with-deps", "dependencies": { "left-pad-fake-dependency": "^1.0.0" } }"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        assert_eq!(host.loaded().errors.len(), 1, "{:?}", host.loaded().errors);
        let error = &host.loaded().errors[0].error;
        assert!(error.contains("left-pad-fake-dependency"), "{error}");
        assert!(error.contains("bun install"), "{error}");
        assert!(error.contains(&root.display().to_string()), "{error}");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An extension missing a dependency nobody declared is left with the plain resolution error.
    #[tokio::test]
    async fn a_missing_undeclared_dependency_is_left_as_reported() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("missing-undeclared");
        let extension = root.join("guesswork.ts");
        std::fs::write(
            &extension,
            r#"import { anything } from "nobody-declared-this-one";
export default (micro) => { micro.registerTool({ name: "uses_it", execute: async () => anything() }); };
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        assert_eq!(host.loaded().errors.len(), 1, "{:?}", host.loaded().errors);
        let error = &host.loaded().errors[0].error;
        assert!(error.contains("nobody-declared-this-one"), "{error}");
        assert!(!error.contains("bun install"), "{error}");

        host.shutdown("quit").await;
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

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");
        let error = host
            .call_tool(
                "nothing-like-this",
                &serde_json::json!({}),
                &micro_tools::Progress::default(),
            )
            .await
            .expect_err("nobody registered it");
        assert!(error.contains("nothing-like-this"), "{error}");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a tool reports through `onUpdate` while it runs arrives as progress before its final
    /// answer does.
    #[tokio::test]
    async fn onupdate_is_forwarded_as_progress_while_the_call_is_in_flight() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("progress");
        let extension = root.join("streamer.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "narrate",
        description: "says what it is doing as it goes",
        execute: async (toolCallId, args, signal, onUpdate) => {
            onUpdate?.({ content: [{ type: "text", text: "step one" }] });
            onUpdate?.({ content: [{ type: "text", text: "step two" }] });
            return "done";
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, false, true, "tui")
                .await
                .expect("the host starts"),
        );
        answer_context_requests(Arc::clone(&host));

        let (reporting, mut reported) = tokio::sync::mpsc::unbounded_channel();
        let progress = micro_tools::Progress::new(reporting);

        let answer = host
            .call_tool("narrate", &serde_json::json!({}), &progress)
            .await
            .expect("the tool answers");
        assert_eq!(as_text(&answer), "done");

        let mut updates = Vec::new();
        while let Ok(update) = reported.try_recv() {
            updates.push(update);
        }
        assert_eq!(
            updates,
            vec!["step one".to_string(), "step two".to_string()]
        );

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A call this side has given up on is told to stop, not left running with nobody listening.
    #[tokio::test]
    async fn dropping_a_call_in_flight_tells_the_host_to_stop_it() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("cancel");
        let marker = root.join("aborted.txt");
        let extension = root.join("waits.ts");
        std::fs::write(
            &extension,
            format!(
                r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerTool({{
        name: "wait_forever",
        description: "never resolves unless the caller gives up on it",
        execute: async (toolCallId, args, signal) => {{
            return new Promise((resolve, reject) => {{
                signal?.addEventListener("abort", () => {{
                    writeFileSync({marker:?}, "aborted");
                    reject(new Error("aborted"));
                }});
            }});
        }},
    }});
}};
"#,
                marker = marker.display().to_string()
            ),
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, false, true, "tui")
                .await
                .expect("the host starts"),
        );
        answer_context_requests(Arc::clone(&host));

        let running = {
            let host = Arc::clone(&host);
            tokio::spawn(async move {
                let _ = host
                    .call_tool(
                        "wait_forever",
                        &serde_json::json!({}),
                        &micro_tools::Progress::default(),
                    )
                    .await;
            })
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        running.abort();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(marker.exists(), "the extension saw the abort signal");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A registered markdown transformer is round-tripped: asked with text and context, answering
    /// with what it rewrote it to.
    #[tokio::test]
    async fn a_registered_markdown_transformer_rewrites_what_it_is_asked_to() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("markdown");
        let extension = root.join("shout.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerMarkdownTransformer((markdown, context) => {
        return context.messageType === "assistant" ? markdown.toUpperCase() : markdown;
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let rewritten = host
            .transform_markdown(
                "hello there",
                &serde_json::json!({ "messageType": "assistant" }),
            )
            .await
            .expect("a transformer answers");
        assert_eq!(rewritten, "HELLO THERE");

        let unchanged = host
            .transform_markdown("hello there", &serde_json::json!({ "messageType": "user" }))
            .await
            .expect("a transformer answers");
        assert_eq!(unchanged, "hello there");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool's renderCall can hand back a live component, and micro can drive it by id.
    #[tokio::test]
    async fn a_tools_rendercall_registers_a_component_micro_can_drive() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("component");
        let extension = root.join("counter.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "counter",
        description: "shows a count that grows on input",
        execute: async () => "done",
        renderCall: () => {
            let count = 0;
            return {
                render(width) {
                    return [`count: ${count} (width ${width})`];
                },
                handleInput(data) {
                    count += 1;
                    return { consume: true };
                },
                invalidate() {
                    count = 0;
                },
            };
        },
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let fields = ToolRenderFields {
            tool_call_id: "call-1".into(),
            ..Default::default()
        };
        let rendered = host
            .render_tool_call("counter", &serde_json::json!({}), &fields)
            .await
            .expect("renderCall answers");
        assert!(rendered.supported, "{:?}", rendered.error);
        let component_id = rendered.component_id.expect("a component was registered");

        let lines = host.render_component(&component_id, 40).await.unwrap();
        assert_eq!(lines, vec!["count: 0 (width 40)"]);

        let consumed = host
            .send_component_input(&component_id, "x", None)
            .await
            .unwrap();
        assert!(consumed, "handleInput said it consumed the key");
        let lines = host.render_component(&component_id, 40).await.unwrap();
        assert_eq!(lines, vec!["count: 1 (width 40)"]);

        host.invalidate_component(&component_id).await.unwrap();
        let lines = host.render_component(&component_id, 40).await.unwrap();
        assert_eq!(lines, vec!["count: 0 (width 40)"], "invalidate reset it");

        host.dispose_component(&component_id).await.unwrap();
        let lines = host.render_component(&component_id, 40).await.unwrap();
        assert!(lines.is_empty(), "a disposed component draws nothing");

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_tool_with_no_rendercall_says_so_rather_than_erroring() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("no-renderer");
        let extension = root.join("plain.ts");
        std::fs::write(
            &extension,
            r#"export default (micro) => { micro.registerTool({ name: "plain", execute: async () => "ok" }); };"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let fields = ToolRenderFields {
            tool_call_id: "call-2".into(),
            ..Default::default()
        };
        let rendered = host
            .render_tool_call("plain", &serde_json::json!({}), &fields)
            .await
            .expect("the request is answered even with nothing to render");
        assert!(!rendered.supported);
        assert!(rendered.error.is_none(), "declaring none is not an error");
        assert!(rendered.component_id.is_none());

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A renderer's own `ctx.invalidate()` pushes `component_changed` unprompted.
    #[tokio::test]
    async fn a_tool_renderer_can_push_a_change_on_its_own_schedule() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("pushed");
        let extension = root.join("ticker.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "ticker",
        execute: async () => "done",
        renderCall: (args, theme, ctx) => {
            return {
                render() {
                    return ["tick"];
                },
                handleInput() {
                    ctx.invalidate();
                    return { consume: true };
                },
            };
        },
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let fields = ToolRenderFields {
            tool_call_id: "call-3".into(),
            ..Default::default()
        };
        let rendered = host
            .render_tool_call("ticker", &serde_json::json!({}), &fields)
            .await
            .expect("renderCall answers");
        let component_id = rendered.component_id.expect("a component was registered");

        let mut asks = host.take_asks().await.expect("the ask stream is here once");
        host.send_component_input(&component_id, "poke", None)
            .await
            .expect("the input reaches it");

        let changed = tokio::time::timeout(std::time::Duration::from_secs(5), asks.recv())
            .await
            .expect("a change arrived in time")
            .expect("the host is still there to say it");
        assert_eq!(changed, FromHost::ComponentChanged { component_id });

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool's renderCall draws itself the moment micro reports the lifecycle event every
    /// extension already gets.
    #[tokio::test]
    async fn a_tools_rendercall_draws_itself_when_the_call_starts() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("auto-render-call");
        let extension = root.join("weather.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "weather",
        description: "reports the weather",
        execute: async () => "sunny",
        renderCall: (args) => ({
            render: (width) => [`${args.city}: checking...`],
        }),
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let mut asks = host.take_asks().await.expect("the ask stream is here once");
        host.notify(
            "tool_execution_start",
            serde_json::json!({
                "toolCallId": "call_1",
                "toolName": "weather",
                "args": { "city": "lima" },
            }),
        )
        .await
        .expect("the notice reaches the host");

        let asked = tokio::time::timeout(std::time::Duration::from_secs(5), asks.recv())
            .await
            .expect("a ui_request arrived in time")
            .expect("the host is still there to say it");
        let FromHost::Ui { payload, .. } = asked else {
            panic!("expected a Ui ask, got {asked:?}");
        };
        assert_eq!(payload["method"], "tool_call_rendered");
        assert_eq!(payload["title"], "call_1");
        assert!(
            payload["detail"].as_str().is_some(),
            "a component id: {payload}"
        );
        assert_eq!(payload["options"], serde_json::json!(["lima: checking..."]));

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_tools_renderresult_draws_itself_when_the_result_arrives() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("auto-render-result");
        let extension = root.join("weather.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerTool({
        name: "weather",
        description: "reports the weather",
        execute: async () => "sunny",
        renderResult: (result) => ({
            render: () => [`says: ${result.content[0].text}`],
        }),
    });
};
"#,
        )
        .unwrap();

        let host = Host::start(&root, &[extension], &root, false, false, "tui")
            .await
            .expect("the host starts");

        let mut asks = host.take_asks().await.expect("the ask stream is here once");
        host.notify(
            "tool_execution_end",
            serde_json::json!({
                "toolCallId": "call_1",
                "toolName": "weather",
                "result": { "content": [{ "type": "text", "text": "sunny" }], "details": null },
                "isError": false,
            }),
        )
        .await
        .expect("the notice reaches the host");

        let asked = tokio::time::timeout(std::time::Duration::from_secs(5), asks.recv())
            .await
            .expect("a ui_request arrived in time")
            .expect("the host is still there to say it");
        let FromHost::Ui { payload, .. } = asked else {
            panic!("expected a Ui ask, got {asked:?}");
        };
        assert_eq!(payload["method"], "tool_result_rendered");
        assert_eq!(payload["title"], "call_1");
        assert_eq!(payload["options"], serde_json::json!(["says: sunny"]));

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn fork_waits_for_session_start_before_running_with_session() {
        if which_bun().is_none() {
            return;
        }
        let root = scratch("with-session");
        let extension = root.join("replace.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const result = await ctx.fork("entry-1", {
                withSession: async (replaced) => {
                    await replaced.sendUserMessage("hello from the replaced session");
                },
            });
            return JSON.stringify(result);
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, true, false, "tui")
                .await
                .expect("the host starts"),
        );

        let mut asks = host.take_asks().await.expect("the ask stream is here once");
        let watching = Arc::clone(&host);
        let seen_send_user_message = tokio::spawn(async move {
            while let Some(asked) = asks.recv().await {
                match asked {
                    FromHost::Request { id, request, .. } if request == "get_context" => {
                        let _ = watching
                            .answer(&id, serde_json::json!({ "thinkingLevel": "off" }))
                            .await;
                    }
                    FromHost::Request { id, request, .. } if request == "fork" => {
                        let _ = watching
                            .answer(&id, serde_json::json!({ "cancelled": false }))
                            .await;
                    }
                    FromHost::Action {
                        action, payload, ..
                    } if action == "send_user_message" => {
                        return payload
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string);
                    }
                    _ => {}
                }
            }
            None
        });

        let calling = Arc::clone(&host);
        let command = tokio::spawn(async move { calling.call_command("probe", "").await });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            !seen_send_user_message.is_finished(),
            "withSession ran before session_start was ever sent"
        );
        assert!(
            !command.is_finished(),
            "the command resolved before its fork was confirmed"
        );

        host.notify("session_start", serde_json::json!({ "reason": "fork" }))
            .await
            .expect("the notice reaches the host");

        let sent = tokio::time::timeout(std::time::Duration::from_secs(5), seen_send_user_message)
            .await
            .expect("withSession's sendUserMessage arrived in time")
            .expect("the watcher task did not panic");
        assert_eq!(sent.as_deref(), Some("hello from the replaced session"));

        let answered = tokio::time::timeout(std::time::Duration::from_secs(5), command)
            .await
            .expect("the command finished in time")
            .expect("the command task did not panic")
            .expect("the command did not error");
        let result: serde_json::Value =
            serde_json::from_str(answered.as_str().expect("a JSON string")).unwrap();
        assert_eq!(result["cancelled"], false);

        host.shutdown("quit").await;
        let _ = std::fs::remove_dir_all(&root);
    }
}
