//! Runs one script in a QuickJS VM of its own, on a thread of its own.
//!
//! The VM has no timers, file system, network, or modules: the script reaches the outside world
//! only through the calls the prelude routes to the host. Tool arguments and results cross the
//! boundary as JSON text. A fresh thread and VM per script keeps ending one simple: a script that
//! spins, even one that only spins the microtask queue, is interrupted and cannot affect the next.

use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt as _;
use rquickjs::function::Rest;
use rquickjs::CatchResultExt as _;
use rquickjs::Context;
use rquickjs::Ctx;
use rquickjs::Function;
use rquickjs::Object;
use rquickjs::Persistent;
use rquickjs::Runtime;
use rquickjs::Value as JsValue;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

const PRELUDE: &str = include_str!("prelude.js");

/// The most one value kept with `store()` may hold, in characters of JSON.
pub const MAX_STORE_VALUE_CHARS: usize = 256 * 1024;
/// The most every stored value together may hold, in characters of JSON, keys included.
pub const MAX_STORE_TOTAL_CHARS: usize = 1024 * 1024;

/// The heap a script may use. Allocations beyond it fail inside the script as
/// `InternalError: out of memory`.
const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
/// The stack QuickJS may use, so deep recursion throws a catchable `RangeError` long before the
/// thread's own stack runs out.
const MAX_STACK_BYTES: usize = 1024 * 1024;
/// The stack of the thread the VM runs on.
const THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

/// One item of a script's output, in the order the script produced it. `data` is base64.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputItem {
    Text(String),
    Image { data: String, mime_type: String },
}

/// How a script failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The script threw or failed to parse.
    Script,
    /// The deadline passed.
    Timeout,
    /// The VM failed outside the script's control.
    Sandbox,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: FailureKind,
    pub name: Option<String>,
    pub message: String,
    /// `Name: message` followed by the script's frames.
    pub stack: Option<String>,
}

impl Failure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Failure {
            kind,
            name: None,
            message: message.into(),
            stack: None,
        }
    }
}

/// The keys a successful script changed with `store()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreWrites {
    pub set: Map<String, Value>,
    /// Keys stored as `undefined`.
    pub delete: Vec<String>,
}

impl StoreWrites {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.delete.is_empty()
    }
}

/// How a tool call a script made ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    Ok,
    Error,
    /// Still running when the script ended.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    pub name: String,
    pub status: CallStatus,
}

/// What running a script came to. `output` is kept for a failed script too, up to the failure.
#[derive(Debug, Clone, PartialEq)]
pub struct Execution {
    pub outcome: Result<Completed, Failure>,
    pub output: Vec<OutputItem>,
    /// The tool calls the script made, in the order it made them.
    pub calls: Vec<CallRecord>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completed {
    /// What the script returned, `None` for `undefined` and for `exit()`.
    pub value: Option<Value>,
    pub writes: StoreWrites,
}

/// A tool a script can call, as the prelude lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTool {
    pub name: String,
    /// What the script calls it: the name made into an identifier.
    pub identifier: String,
    /// Listed in `ALL_TOOLS`.
    pub description: String,
}

/// Everything one script is run with.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub tools: Vec<ScriptTool>,
    /// Functions the script calls by name, such as `searchTools`; `a.b` names are grouped under
    /// `a`. Every argument the script passes reaches the host as one array.
    pub globals: Vec<String>,
    /// What `load()` starts with.
    pub store: Map<String, Value>,
    /// The deadline for the whole script, tool calls included.
    pub timeout: Option<Duration>,
}

/// Where a script's calls are answered.
#[async_trait]
pub trait Host: Send + Sync {
    /// Run a tool for the script. `Ok` is the JSON the call resolves to, `None` for `undefined`;
    /// `Err` rejects it with that message.
    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<Option<Value>, String>;

    /// Run one of the globals, with every argument the script passed as one array.
    async fn call_global(&self, name: &str, arguments: Value) -> Result<Option<Value>, String>;
}

/// What the VM thread tells the host.
enum FromScript {
    Call {
        id: u64,
        tool: bool,
        name: String,
        arguments: Option<String>,
    },
    Output(OutputItem),
    Done(Result<(Option<String>, String), String>),
    Crash(String),
}

/// What the host answers a call with: the JSON text it resolves to, or the message it rejects
/// with.
struct Settle {
    id: u64,
    ok: bool,
    payload: Option<String>,
}

/// The JavaScript the script's source is wrapped in. The prefix shares the first line with the
/// script, so the line numbers in errors match the script as written.
fn wrapped(code: &str) -> String {
    format!("(async (tools, console) => {{{code}\n}})")
}

type Pending<'a> = Pin<Box<dyn Future<Output = (u64, Result<Option<Value>, String>)> + Send + 'a>>;

/// Run `code` as the body of an async function. Never fails itself: a failed script comes back
/// as a failed outcome. Calls still running when the script ends are dropped, which cancels them.
pub async fn execute(code: &str, setup: Setup, host: &dyn Host) -> Execution {
    let interrupted = Arc::new(AtomicBool::new(false));
    let (to_host, mut from_script) = tokio::sync::mpsc::unbounded_channel();
    let (to_script, from_host) = std::sync::mpsc::channel::<Settle>();

    let started = {
        let interrupted = Arc::clone(&interrupted);
        let source = wrapped(code);
        let tools = setup.tools.clone();
        let globals = setup.globals.clone();
        let store = setup.store.clone();
        std::thread::Builder::new()
            .name("codemode".into())
            .stack_size(THREAD_STACK_BYTES)
            .spawn(move || {
                let crashed = to_host.clone();
                if let Err(error) = run_vm(
                    &source,
                    &tools,
                    &globals,
                    &store,
                    from_host,
                    to_host,
                    interrupted,
                ) {
                    let _ = crashed.send(FromScript::Crash(error));
                }
            })
    };
    // Whatever ends the execution, the VM is told to stop.
    let _stop = StopOnDrop(Arc::clone(&interrupted));

    let mut output = Vec::new();
    let mut calls: Vec<CallRecord> = Vec::new();
    if let Err(error) = started {
        return Execution {
            outcome: Err(Failure::new(
                FailureKind::Sandbox,
                format!("cannot start the script: {error}"),
            )),
            output,
            calls,
        };
    }

    let deadline = setup.timeout.map(|timeout| Instant::now() + timeout);
    let mut pending: FuturesUnordered<Pending<'_>> = FuturesUnordered::new();
    // The record of each tool call still running, by call id.
    let mut running: HashMap<u64, usize> = HashMap::new();

    let outcome = loop {
        let sleep = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            message = from_script.recv() => match message {
                Some(FromScript::Output(item)) => output.push(item),
                Some(FromScript::Call { id, tool, name, arguments }) => {
                    let parsed = match arguments.as_deref().map(serde_json::from_str::<Value>) {
                        None => Ok(None),
                        Some(Ok(value)) => Ok(Some(value)),
                        Some(Err(error)) => Err(error.to_string()),
                    };
                    if tool {
                        running.insert(id, calls.len());
                        calls.push(CallRecord { name: name.clone(), status: CallStatus::Cancelled });
                    }
                    pending.push(Box::pin(async move {
                        let answer = match parsed {
                            Err(error) => Err(error),
                            Ok(arguments) if tool => host.call_tool(&name, arguments).await,
                            Ok(arguments) => {
                                host.call_global(&name, arguments.unwrap_or_else(|| json!([]))).await
                            }
                        };
                        (id, answer)
                    }));
                }
                Some(FromScript::Done(Ok((value, writes)))) => {
                    break completed(value.as_deref(), &writes);
                }
                Some(FromScript::Done(Err(error))) => break Err(script_failure(&error)),
                Some(FromScript::Crash(message)) => {
                    break Err(Failure::new(FailureKind::Sandbox, message));
                }
                None => {
                    break Err(Failure::new(
                        FailureKind::Sandbox,
                        "the script's VM stopped before the script settled",
                    ));
                }
            },
            Some((id, answer)) = pending.next(), if !pending.is_empty() => {
                if let Some(index) = running.remove(&id) {
                    calls[index].status = match answer {
                        Ok(_) => CallStatus::Ok,
                        Err(_) => CallStatus::Error,
                    };
                }
                let settle = match answer {
                    Ok(value) => Settle { id, ok: true, payload: value.map(|value| value.to_string()) },
                    Err(error) => Settle { id, ok: false, payload: Some(error) },
                };
                let _ = to_script.send(settle);
            }
            () = sleep => {
                let timeout = setup.timeout.unwrap_or_default();
                break Err(Failure::new(
                    FailureKind::Timeout,
                    format!("Execution timed out after {} ms", timeout.as_millis()),
                ));
            }
        }
    };

    interrupted.store(true, Ordering::SeqCst);
    drop(to_script);
    drop(pending);
    Execution {
        outcome,
        output,
        calls,
    }
}

/// Tells the VM to stop when the execution is dropped, also when the caller gives up on it.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn completed(value: Option<&str>, writes: &str) -> Result<Completed, Failure> {
    let value = match value.map(serde_json::from_str::<Value>) {
        None => None,
        Some(Ok(value)) => Some(value),
        Some(Err(error)) => {
            return Err(Failure::new(
                FailureKind::Sandbox,
                format!("the script's value could not be read: {error}"),
            ))
        }
    };
    let entries: Vec<Vec<String>> = serde_json::from_str(writes).unwrap_or_default();
    let mut store = StoreWrites::default();
    for entry in entries {
        match entry.as_slice() {
            [key] => store.delete.push(key.clone()),
            [key, json] => {
                if let Ok(value) = serde_json::from_str(json) {
                    store.set.insert(key.clone(), value);
                }
            }
            _ => {}
        }
    }
    Ok(Completed {
        value,
        writes: store,
    })
}

fn script_failure(error: &str) -> Failure {
    let parsed: Value = serde_json::from_str(error).unwrap_or_else(|_| json!({ "message": error }));
    let field = |key: &str| parsed.get(key).and_then(Value::as_str).map(str::to_string);
    Failure {
        kind: FailureKind::Script,
        name: field("name"),
        message: field("message").unwrap_or_default(),
        stack: field("stack"),
    }
}

/// The VM thread: set up the prelude, start the script, then settle calls as the host answers
/// them until the host stops listening.
fn run_vm(
    source: &str,
    tools: &[ScriptTool],
    globals: &[String],
    store: &Map<String, Value>,
    from_host: std::sync::mpsc::Receiver<Settle>,
    to_host: UnboundedSender<FromScript>,
    interrupted: Arc<AtomicBool>,
) -> Result<(), String> {
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
    runtime.set_max_stack_size(MAX_STACK_BYTES);
    {
        let interrupted = Arc::clone(&interrupted);
        runtime.set_interrupt_handler(Some(Box::new(move || interrupted.load(Ordering::Relaxed))));
    }
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;

    let tools_json = Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "jsName": tool.identifier,
                    "description": tool.description,
                })
            })
            .collect(),
    )
    .to_string();
    let globals_json = json!(globals).to_string();
    let store_json = Value::Object(
        store
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.to_string())))
            .collect(),
    )
    .to_string();
    let limits_json = json!({
        "storeValueChars": MAX_STORE_VALUE_CHARS,
        "storeTotalChars": MAX_STORE_TOTAL_CHARS,
    })
    .to_string();

    let api = context.with(|ctx| -> Result<Option<Api>, String> {
        let bridge = bridge(&ctx, to_host.clone())?;
        let prelude: Function = ctx
            .eval_with_options(PRELUDE, eval_options("codemode-prelude.js"))
            .catch(&ctx)
            .map_err(|error| format!("the prelude failed: {error}"))?;
        let api: Object = prelude
            .call((bridge, tools_json, globals_json, store_json, limits_json))
            .catch(&ctx)
            .map_err(|error| format!("the prelude failed: {error}"))?;
        let get = |name: &str| -> Result<Persistent<Function<'static>>, String> {
            let function: Function = api.get(name).map_err(|error| error.to_string())?;
            Ok(Persistent::save(&ctx, function))
        };
        let (settle, run, stalled) = (get("settle")?, get("run")?, get("stalled")?);

        let script: Function = match ctx.eval_with_options(source, eval_options("codemode.js")) {
            Ok(script) => script,
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let _ = to_host.send(FromScript::Done(Err(describe_exception(&thrown))));
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        };
        let _ = run.clone().restore(&ctx).and_then(|run| {
            let started: rquickjs::Result<JsValue> = run.call((script,));
            started
        });
        Ok(Some(Api { settle, stalled }))
    })?;
    let Some(api) = api else {
        return Ok(());
    };
    drain(&runtime, &context, &api, &interrupted);

    while let Ok(settle) = from_host.recv() {
        if interrupted.load(Ordering::SeqCst) {
            break;
        }
        context.with(|ctx| {
            if let Ok(function) = api.settle.clone().restore(&ctx) {
                let _: rquickjs::Result<JsValue> =
                    function.call((settle.id as f64, settle.ok, settle.payload));
            }
        });
        drain(&runtime, &context, &api, &interrupted);
    }
    Ok(())
}

/// The prelude's functions, kept across calls into the VM.
struct Api {
    settle: Persistent<Function<'static>>,
    stalled: Persistent<Function<'static>>,
}

fn eval_options(filename: &str) -> rquickjs::context::EvalOptions {
    let mut options = rquickjs::context::EvalOptions::default();
    options.strict = false;
    options.filename = Some(filename.to_string());
    options
}

/// Run every queued job, then fail a script that waits on nothing that can ever resume it.
fn drain(runtime: &Runtime, context: &Context, api: &Api, interrupted: &AtomicBool) {
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return;
        }
        match runtime.execute_pending_job() {
            Ok(true) => continue,
            Ok(false) => break,
            Err(_) => continue,
        }
    }
    context.with(|ctx| {
        if let Ok(stalled) = api.stalled.clone().restore(&ctx) {
            let _: rquickjs::Result<JsValue> = stalled.call(());
        }
    });
}

/// The function the prelude reaches the host through. It only ever receives primitives.
fn bridge<'js>(
    ctx: &Ctx<'js>,
    to_host: UnboundedSender<FromScript>,
) -> Result<Function<'js>, String> {
    Function::new(ctx.clone(), move |arguments: Rest<JsValue<'js>>| {
        let arguments = arguments.0;
        let at = |index: usize| arguments.get(index).filter(|value| !value.is_undefined());
        let text = |index: usize| at(index).and_then(js_string);
        let kind = text(0).unwrap_or_default();
        let message = match kind.as_str() {
            "call" | "global" => Some(FromScript::Call {
                id: at(1).and_then(JsValue::as_number).unwrap_or_default() as u64,
                tool: kind == "call",
                name: text(2).unwrap_or_default(),
                arguments: text(3),
            }),
            "output" => Some(FromScript::Output(match text(1).as_deref() {
                Some("image") => OutputItem::Image {
                    data: text(2).unwrap_or_default(),
                    mime_type: text(3).unwrap_or_default(),
                },
                _ => OutputItem::Text(text(2).unwrap_or_default()),
            })),
            "done" => Some(FromScript::Done(
                match at(1).and_then(JsValue::as_bool).unwrap_or(false) {
                    true => Ok((text(2), text(3).unwrap_or_else(|| "[]".to_string()))),
                    false => Err(text(2).unwrap_or_default()),
                },
            )),
            _ => None,
        };
        if let Some(message) = message {
            let _ = to_host.send(message);
        }
    })
    .map_err(|error| error.to_string())
}

fn js_string(value: &JsValue<'_>) -> Option<String> {
    value.as_string().and_then(|text| text.to_string().ok())
}

/// A thrown value as the JSON the prelude reports script errors in.
fn describe_exception(thrown: &JsValue<'_>) -> String {
    let Some(object) = thrown.as_object() else {
        let message = js_string(thrown).unwrap_or_else(|| "the script threw".to_string());
        return json!({ "message": message }).to_string();
    };
    let field = |key: &str| -> Option<String> {
        object
            .get::<_, JsValue>(key)
            .ok()
            .as_ref()
            .and_then(js_string)
    };
    let name = field("name").unwrap_or_else(|| "Error".to_string());
    let message = field("message").unwrap_or_default();
    let head = match message.is_empty() {
        true => name.clone(),
        false => format!("{name}: {message}"),
    };
    let stack = match field("stack").map(|stack| stack.trim_end().to_string()) {
        Some(stack) if !stack.is_empty() => format!("{head}\n{stack}"),
        _ => head,
    };
    json!({ "name": name, "message": message, "stack": stack }).to_string()
}
