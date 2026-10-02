//! The `codemode` tool: the model writes JavaScript that calls the other tools, and only what the
//! script outputs reaches the model. Scripts run as the body of an async function in a QuickJS
//! sandbox with no Node APIs, file system, network, or timers.
//!
//! Scripts call the run's tools through [`micro_tools::ToolCaller`], so every call goes through
//! the same checks as one the model made and is recorded on the `codemode` result. A tool that
//! declares an output schema resolves to its structured answer; any other tool resolves to its
//! text; a failed call rejects with the tool's error text.

mod declarations;
mod description;
mod identifier;
mod sandbox;
mod search;
mod source;

pub use declarations::schema_to_type;
pub use description::Mode;
pub use description::DEFAULT_INLINE_BUDGET;
pub use identifier::identifier;
pub use sandbox::StoreWrites;
pub use sandbox::MAX_STORE_TOTAL_CHARS;
pub use sandbox::MAX_STORE_VALUE_CHARS;

use async_trait::async_trait;
use declarations::render_sample;
use micro_tools::CallableTool;
use micro_tools::Loadout;
use micro_tools::LoadoutChanges;
use micro_tools::Tool;
use micro_tools::ToolCaller;
use micro_tools::ToolContext;
use micro_tools::ToolOutput;
use micro_types::ConstrainedSampling;
use micro_types::ContentBlock;
use micro_types::GrammarVariants;
use micro_types::ToolDefinition;
use micro_types::ToolExposure;
use micro_types::Usage;
use sandbox::CallStatus;
use sandbox::Execution;
use sandbox::Failure;
use sandbox::FailureKind;
use sandbox::OutputItem;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

pub const CODEMODE_TOOL_NAME: &str = "codemode";

/// The custom session entry one successful script's `store()` writes are kept in.
pub const STORE_ENTRY_TYPE: &str = "codemode-store";

/// The output budget when the script does not set one, in estimated tokens.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 10_000;
/// Characters per token when estimating.
const CHARS_PER_TOKEN: usize = 4;

/// The names the host's own globals take, so extra globals cannot shadow them.
const RESERVED_GLOBALS: &[&str] = &[
    "tools",
    "ALL_TOOLS",
    "console",
    "text",
    "image",
    "exit",
    "globalThis",
    "store",
    "load",
    "searchTools",
    "describeTool",
    "describeNamespace",
];

/// Where `store()` values are kept between scripts.
#[async_trait]
pub trait ScriptStore: Send + Sync {
    /// What `load()` sees: the values written on the session's current branch.
    async fn load(&self) -> Map<String, Value>;

    /// Keep one successful script's writes.
    async fn append(&self, writes: &StoreWrites) -> Result<(), String>;
}

/// Functions offered to scripts beyond the built-in ones, such as a namespace of operations the
/// host runs on the script's behalf.
#[async_trait]
pub trait ScriptGlobals: Send + Sync {
    /// The names scripts call: identifiers, or `namespace.member`, which groups members under one
    /// frozen object.
    fn names(&self) -> Vec<String>;

    /// One line for the `Globals:` list of the `codemode` description.
    fn describe(&self) -> String;

    /// Run `name` with every argument the script passed, as one array. `Ok(None)` resolves to
    /// `undefined`.
    async fn call(&self, name: &str, arguments: Value) -> Result<Option<Value>, String>;

    /// What a call that returned `result` spent on a model, for the tool result to count and
    /// show. Calls that spend nothing answer `None`.
    fn spent(&self, name: &str, result: &Value) -> Option<GlobalSpend> {
        let _ = (name, result);
        None
    }
}

/// What one global call spent on a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlobalSpend {
    /// The call as the result names it, such as `classify typesafe/jev-latest`.
    pub label: String,
    pub usage: Usage,
    /// US dollars.
    pub cost: f64,
    /// Images the call generated, which the script shows with `image()`.
    pub images: usize,
}

/// The data of a `codemode-store` entry for these writes.
pub fn store_entry(writes: &StoreWrites) -> Value {
    json!({ "set": writes.set, "delete": writes.delete })
}

/// Apply one `codemode-store` entry's data on top of `store`. Data of another shape is ignored.
pub fn apply_store_entry(store: &mut Map<String, Value>, data: &Value) {
    let (Some(set), Some(deleted)) = (
        data.get("set").and_then(Value::as_object),
        data.get("delete").and_then(Value::as_array),
    ) else {
        return;
    };
    if !deleted.iter().all(Value::is_string) {
        return;
    }
    for key in deleted.iter().filter_map(Value::as_str) {
        store.remove(key);
    }
    for (key, value) in set {
        store.insert(key.clone(), value.clone());
    }
}

/// The `codemode` tool.
pub struct Codemode {
    mode: Mode,
    inline_budget: Option<usize>,
    store: Option<Arc<dyn ScriptStore>>,
    globals: Option<Arc<dyn ScriptGlobals>>,
}

impl Default for Codemode {
    fn default() -> Self {
        Codemode::new()
    }
}

impl Codemode {
    pub fn new() -> Self {
        Codemode {
            mode: Mode::On,
            inline_budget: Some(DEFAULT_INLINE_BUDGET),
            store: None,
            globals: None,
        }
    }

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// The token budget the tool declarations in the description share.
    pub fn with_inline_budget(mut self, budget: usize) -> Self {
        self.inline_budget = Some(budget);
        self
    }

    /// Keep `store()` values across scripts. Without a store, writes last only for one script.
    pub fn with_store(mut self, store: Arc<dyn ScriptStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Offer scripts further functions. Says which name cannot be used, if one cannot.
    pub fn with_globals(mut self, globals: Arc<dyn ScriptGlobals>) -> Result<Self, String> {
        for name in globals.names() {
            let parts: Vec<&str> = name.split('.').collect();
            let valid = parts.len() <= 2
                && parts.iter().all(|part| identifier::is_identifier(part))
                && !RESERVED_GLOBALS.contains(&parts[0]);
            if !valid {
                return Err(format!("invalid global name \"{name}\""));
            }
        }
        self.globals = Some(globals);
        Ok(self)
    }

    fn extra_globals(&self) -> Vec<String> {
        self.globals
            .as_ref()
            .map(|globals| vec![globals.describe()])
            .unwrap_or_default()
    }
}

#[async_trait]
impl Tool for Codemode {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: CODEMODE_TOOL_NAME.into(),
            description: description::description(&[], self.inline_budget, &self.extra_globals()),
            parameters: json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Raw JavaScript source." },
                },
                "required": ["code"],
            }),
            // Capable models write the script as raw text instead of a JSON-escaped string.
            constrained_sampling: Some(ConstrainedSampling::Grammar {
                variants: GrammarVariants {
                    openai_lark: Some(source::SOURCE_GRAMMAR.to_string()),
                    openai_regex: None,
                },
            }),
        }
    }

    /// Scripts must not start other scripts.
    fn exposure(&self) -> ToolExposure {
        ToolExposure::ModelOnly
    }

    fn prepare_loadout(&self, loadout: &Loadout) -> LoadoutChanges {
        description::prepare_loadout(
            loadout,
            self.mode,
            self.inline_budget,
            &self.extra_globals(),
        )
    }

    async fn execute(&self, _arguments: &Value) -> Result<String, String> {
        Err("codemode runs inside a session, which lends it the tools to call".to_string())
    }

    async fn call(&self, arguments: &Value, context: &ToolContext<'_>) -> ToolOutput {
        let started = Instant::now();
        let Some(input) = arguments.get("code").and_then(Value::as_str) else {
            return ToolOutput::error("missing required argument: code");
        };
        let parsed = match source::parse(input) {
            Ok(parsed) => parsed,
            Err(error) => return ToolOutput::error(error),
        };

        let caller = context.tools();
        if let Some(caller) = caller.filter(|_| reaches_for_unlisted_tools(&parsed.code)) {
            caller.arrived().await;
        }
        let callable: Vec<CallableTool> = caller
            .map(|caller| caller.callable())
            .unwrap_or_default()
            .into_iter()
            .filter(|tool| tool.definition.name != CODEMODE_TOOL_NAME)
            .collect();
        let host = ScriptHost::new(caller, callable, self.globals.clone());

        let mut globals = vec![
            "searchTools".to_string(),
            "describeTool".to_string(),
            "describeNamespace".to_string(),
        ];
        globals.extend(self.globals.iter().flat_map(|globals| globals.names()));
        let store = match &self.store {
            Some(store) => store.load().await,
            None => Map::new(),
        };
        let setup = sandbox::Setup {
            tools: host
                .callable
                .iter()
                .map(|tool| sandbox::ScriptTool {
                    name: tool.definition.name.clone(),
                    identifier: identifier(&tool.definition.name),
                    description: host.samples[&tool.definition.name].clone(),
                })
                .collect(),
            globals,
            store,
            timeout: parsed.options.timeout_ms.map(Duration::from_millis),
        };

        let execution = sandbox::execute(&parsed.code, setup, &host).await;
        let spends = host.take_spends();
        let shown = execution
            .output
            .iter()
            .filter(|item| matches!(item, OutputItem::Image { .. }))
            .count();
        let mut kept = Ok(());
        if let (Ok(completed), Some(store)) = (&execution.outcome, &self.store) {
            if !completed.writes.is_empty() {
                kept = store.append(&completed.writes).await;
            }
        }
        let mut output = answer(
            execution,
            parsed
                .options
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
            started,
        )
        .await;
        if let Err(error) = kept {
            output.content.push(ContentBlock::text(format!(
                "Note: the values this script stored were not kept: {error}"
            )));
        }
        account(&mut output, &spends, shown);
        output
    }
}

/// Whether a script looks for tools beyond those already here, such as those of MCP servers that
/// are still connecting, so it is worth waiting for them.
fn reaches_for_unlisted_tools(code: &str) -> bool {
    [
        "mcp__",
        "ALL_TOOLS",
        "searchTools",
        "describeTool",
        "describeNamespace",
    ]
    .iter()
    .any(|needle| code.contains(needle))
}

/// Answers a script's calls: tools through the run, discovery globals from the tool list.
struct ScriptHost<'a> {
    caller: Option<&'a dyn ToolCaller>,
    callable: Vec<CallableTool>,
    /// Each tool's description and declaration, by tool name.
    samples: HashMap<String, String>,
    globals: Option<Arc<dyn ScriptGlobals>>,
    /// What the script's global calls spent, in the order they finished.
    spends: std::sync::Mutex<Vec<GlobalSpend>>,
}

impl<'a> ScriptHost<'a> {
    fn take_spends(&self) -> Vec<GlobalSpend> {
        std::mem::take(
            &mut *self
                .spends
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    fn new(
        caller: Option<&'a dyn ToolCaller>,
        callable: Vec<CallableTool>,
        globals: Option<Arc<dyn ScriptGlobals>>,
    ) -> Self {
        let samples = callable
            .iter()
            .map(|tool| {
                (
                    tool.definition.name.clone(),
                    render_sample(&description::declaration_of(tool)),
                )
            })
            .collect();
        ScriptHost {
            caller,
            callable,
            samples,
            globals,
            spends: std::sync::Mutex::default(),
        }
    }

    fn find(&self, name: &str) -> Option<&CallableTool> {
        self.callable
            .iter()
            .find(|tool| tool.definition.name == name || identifier(&tool.definition.name) == name)
    }

    /// `{ name, description }` as `ALL_TOOLS` lists a tool.
    fn listing(&self, tool: &CallableTool) -> Value {
        json!({
            "name": identifier(&tool.definition.name),
            "description": self.samples[&tool.definition.name],
        })
    }

    fn search_tools(&self, arguments: &[Value]) -> Result<Option<Value>, String> {
        let query = arguments
            .first()
            .and_then(Value::as_str)
            .ok_or("searchTools() expects a query string")?;
        let options = arguments.get(1).filter(|options| !options.is_null());
        let limit = match options.and_then(|options| options.get("limit")) {
            None | Some(Value::Null) => search::DEFAULT_LIMIT,
            Some(limit) => limit
                .as_u64()
                .filter(|limit| *limit > 0)
                .ok_or("searchTools() limit must be a positive integer")?
                as usize,
        };
        let namespace = match options.and_then(|options| options.get("namespace")) {
            None | Some(Value::Null) => None,
            Some(Value::String(namespace)) => Some(namespace.as_str()),
            Some(_) => return Err("searchTools() namespace must be a string".to_string()),
        };
        let documents: Vec<search::Document> = self
            .callable
            .iter()
            .filter(|tool| match namespace {
                None => true,
                Some(wanted) => tool
                    .namespace
                    .as_ref()
                    .is_some_and(|namespace| is_namespace_name(&namespace.name, wanted)),
            })
            .map(|tool| {
                search::document(
                    &tool.definition.name,
                    &tool.definition.description,
                    &tool.definition.parameters,
                    tool.namespace.as_ref(),
                )
            })
            .collect();
        let found: Vec<Value> = search::rank(query, &documents, limit)
            .iter()
            .filter_map(|name| self.find(name))
            .map(|tool| self.listing(tool))
            .collect();
        Ok(Some(Value::Array(found)))
    }

    fn describe_tool(&self, arguments: &[Value]) -> Result<Option<Value>, String> {
        let name = arguments
            .first()
            .and_then(Value::as_str)
            .ok_or("describeTool() expects a tool name")?;
        Ok(self
            .find(name)
            .map(|tool| Value::String(self.samples[&tool.definition.name].clone())))
    }

    fn describe_namespace(&self, arguments: &[Value]) -> Result<Option<Value>, String> {
        let wanted = arguments
            .first()
            .and_then(Value::as_str)
            .ok_or("describeNamespace() expects a namespace name")?;
        let mut found = None;
        let mut tools = Vec::new();
        for tool in &self.callable {
            let Some(namespace) = tool
                .namespace
                .as_ref()
                .filter(|namespace| is_namespace_name(&namespace.name, wanted))
            else {
                continue;
            };
            found.get_or_insert_with(|| namespace.clone());
            tools.push(Value::String(identifier(&tool.definition.name)));
        }
        Ok(found.map(|namespace| {
            let mut described = json!({ "name": namespace.name, "tools": tools });
            if let Some(description) = namespace.description {
                described["description"] = json!(description);
            }
            if let Some(instructions) = namespace.instructions {
                described["instructions"] = json!(instructions);
            }
            described
        }))
    }
}

/// Whether `query` names the namespace: its name, its identifier (`mcp__dev-radius` is
/// `mcp__dev_radius`), or the part after its last `__` in either form.
fn is_namespace_name(namespace: &str, query: &str) -> bool {
    let id = identifier(namespace);
    let query_id = identifier(query);
    let suffix = |name: &str| name.rfind("__").map(|at| name[at + 2..].to_string());
    namespace == query
        || id == query_id
        || suffix(namespace).as_deref() == Some(query)
        || suffix(&id).as_deref() == Some(query_id.as_str())
}

#[async_trait]
impl sandbox::Host for ScriptHost<'_> {
    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let (Some(caller), Some(tool)) = (self.caller, self.find(name)) else {
            return Err(format!("Unknown tool \"{name}\""));
        };
        let outcome = caller
            .call(
                &tool.definition.name,
                arguments.unwrap_or_else(|| json!({})),
            )
            .await;
        script_value(tool, outcome.output).map(Some)
    }

    async fn call_global(&self, name: &str, arguments: Value) -> Result<Option<Value>, String> {
        let listed = arguments.as_array().cloned().unwrap_or_default();
        match name {
            "searchTools" => self.search_tools(&listed),
            "describeTool" => self.describe_tool(&listed),
            "describeNamespace" => self.describe_namespace(&listed),
            other => match &self.globals {
                Some(globals) => {
                    let result = globals.call(other, arguments).await;
                    if let Ok(Some(value)) = &result {
                        if let Some(spent) = globals.spent(other, value) {
                            self.spends
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(spent);
                        }
                    }
                    result
                }
                None => Err(format!("Unknown global \"{other}\"")),
            },
        }
    }
}

/// What a script receives for a call: a tool that declares an output schema resolves to its
/// structured answer, also for an error that carries one; any other tool resolves to its text.
/// Other failures reject with the tool's error text.
fn script_value(tool: &CallableTool, output: ToolOutput) -> Result<Value, String> {
    if tool.output_schema.is_some() {
        if let Some(structured) = output.structured {
            return Ok(structured);
        }
    }
    let text = output.text_content();
    match output.is_error {
        true if text.is_empty() => Err(format!("Tool \"{}\" failed", tool.definition.name)),
        true => Err(text),
        false => Ok(Value::String(text)),
    }
}

/// A value as `text()` shows it: strings as they are, anything else as compact JSON.
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn failure_text(failure: &Failure, calls: &[sandbox::CallRecord]) -> String {
    let head = match failure.kind {
        FailureKind::Script => failure.stack.clone().unwrap_or_else(|| {
            format!(
                "{}: {}",
                failure.name.as_deref().unwrap_or("Error"),
                failure.message
            )
        }),
        FailureKind::Timeout => format!("Script timed out: {}", failure.message),
        FailureKind::Sandbox => format!("Script sandbox failed: {}", failure.message),
    };
    let summary = match calls.is_empty() {
        true => "No tool calls were made.".to_string(),
        false => format!(
            "Tool calls made before the failure (they are not undone): {}",
            calls
                .iter()
                .map(|call| format!(
                    "{} ({})",
                    call.name,
                    match call.status {
                        CallStatus::Ok => "ok",
                        CallStatus::Error => "error",
                        CallStatus::Cancelled => "cancelled",
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!("{head}\n\n{summary}")
}

/// The tool's answer: a header, the script's output within its token budget, and the error of a
/// failed script.
async fn answer(execution: Execution, max_output_tokens: u64, started: Instant) -> ToolOutput {
    let mut items: Vec<ContentBlock> = execution
        .output
        .into_iter()
        .map(|item| match item {
            OutputItem::Text(text) => ContentBlock::text(text),
            OutputItem::Image { data, mime_type } => ContentBlock::Image { data, mime_type },
        })
        .collect();
    let failed = execution.outcome.is_err();
    match &execution.outcome {
        Ok(completed) => {
            if let Some(value) = &completed.value {
                items.push(ContentBlock::text(value_text(value)));
            }
        }
        Err(failure) => items.push(ContentBlock::text(format!(
            "Script error:\n{}",
            failure_text(failure, &execution.calls)
        ))),
    }

    let items = truncate(items, max_output_tokens).await;
    let header = format!(
        "{}\nWall time {:.1} seconds\nOutput:\n",
        match failed {
            true => "Script failed",
            false => "Script completed",
        },
        started.elapsed().as_secs_f64()
    );
    let mut content = vec![ContentBlock::text(header)];
    content.extend(items);
    ToolOutput {
        content,
        is_error: failed,
        ..ToolOutput::default()
    }
}

/// Count what the script's model calls spent into the tool result, list each call's cost, and
/// say so when generated images were never shown.
fn account(output: &mut ToolOutput, spends: &[GlobalSpend], shown: usize) {
    if spends.is_empty() {
        return;
    }
    let usage = spends
        .iter()
        .fold(Usage::default(), |sum, spent| sum.plus(spent.usage));
    let cost: f64 = spends.iter().map(|spent| spent.cost).sum();
    output.usage = Some(output.usage.map_or(usage, |own| own.plus(usage)));
    output.cost = Some(output.cost.unwrap_or_default() + cost);

    let lines: Vec<String> = spends
        .iter()
        .map(|spent| {
            format!(
                "- {}: {} tokens, ${:.6}",
                spent.label,
                spent.usage.total_tokens(),
                spent.cost
            )
        })
        .collect();
    output.content.push(ContentBlock::text(format!(
        "Model calls (${cost:.6} in all):
{}",
        lines.join(
            "
"
        )
    )));

    let generated: usize = spends.iter().map(|spent| spent.images).sum();
    if generated > 0 && shown == 0 {
        output.content.push(ContentBlock::text(format!(
            "Note: the script generated {generated} image(s) but showed none. Pass the image blocks of `result.output` to `image()` to show them; they are not saved anywhere else."
        )));
    }
}

/// Keep the text within the token budget: when the combined text is longer, it becomes one block
/// that keeps its start and end, followed by the images. The full text goes to a temporary file.
async fn truncate(items: Vec<ContentBlock>, max_tokens: u64) -> Vec<ContentBlock> {
    let texts: Vec<&str> = items
        .iter()
        .filter(|item| matches!(item, ContentBlock::Text { .. }))
        .map(ContentBlock::as_text)
        .collect();
    let combined = texts.join("\n");
    let length = combined.chars().count();
    let budget = (max_tokens as usize).saturating_mul(CHARS_PER_TOKEN);
    if texts.is_empty() || length <= budget {
        return items;
    }
    let head_chars = budget / 2;
    let tail_chars = budget - head_chars;
    let removed = length - head_chars - tail_chars;
    let head: String = combined.chars().take(head_chars).collect();
    let tail: String = combined.chars().skip(length - tail_chars).collect();
    let mut text = format!(
        "Warning: truncated output (original token count: {})\nTotal output lines: {}\n\n{head}…{} tokens truncated…{tail}",
        length.div_ceil(CHARS_PER_TOKEN),
        combined.split('\n').count(),
        removed.div_ceil(CHARS_PER_TOKEN),
    );
    match micro_tools::spill("micro-codemode", &combined).await {
        Ok(path) => text.push_str(&format!(
            "\n\n[Full output: {} (read with offset/limit)]",
            path.display()
        )),
        Err(error) => text.push_str(&format!("\n\n[Could not save the full output: {error}]")),
    }
    let mut kept = vec![ContentBlock::text(text)];
    kept.extend(
        items
            .into_iter()
            .filter(|item| matches!(item, ContentBlock::Image { .. })),
    );
    kept
}

#[cfg(test)]
mod tests;
