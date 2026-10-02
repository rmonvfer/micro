use super::*;
use micro_tools::NestedOutcome;
use micro_types::ToolNamespace;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// A one-pixel PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// The tools a script in these tests can call.
#[derive(Default)]
struct Tools {
    calls: AtomicUsize,
    /// Set when a slow call is dropped before it finished.
    abandoned: Arc<AtomicBool>,
}

fn definition(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        parameters: json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        constrained_sampling: None,
    }
}

fn callable(name: &str, description: &str) -> CallableTool {
    CallableTool {
        definition: definition(name, description),
        exposure: ToolExposure::Direct,
        namespace: None,
        annotations: None,
        output_schema: None,
    }
}

/// Sets a flag when dropped before it was disarmed.
struct Abandoned(Arc<AtomicBool>, bool);

impl Drop for Abandoned {
    fn drop(&mut self) {
        if self.1 {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

#[async_trait]
impl ToolCaller for Tools {
    fn callable(&self) -> Vec<CallableTool> {
        let mut bash = callable("bash", "Run a command");
        bash.output_schema = Some(json!({
            "type": "object",
            "properties": { "output": { "type": "string" }, "exit_code": { "type": "number" } },
            "required": ["output", "exit_code"],
        }));
        let mut search = callable("mcp__docs-site__search", "Search the documentation pages");
        search.exposure = ToolExposure::Deferred;
        search.namespace = Some(ToolNamespace {
            name: "mcp__docs-site".into(),
            description: Some("Product documentation".into()),
            instructions: Some("Search before reading.".into()),
        });
        vec![
            callable("read", "Read a file"),
            bash,
            callable("fail", "Always fails"),
            callable("slow", "Takes its time"),
            search,
        ]
    }

    async fn arrived(&self) {}

    async fn call(&self, name: &str, arguments: Value) -> NestedOutcome {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let output = match name {
            "read" => ToolOutput::text(format!(
                "contents of {}",
                arguments["path"].as_str().unwrap_or("?")
            )),
            "bash" => ToolOutput::error("exit code 2\nboom")
                .with_structured(json!({ "output": "boom\n", "exit_code": 2 })),
            "fail" => ToolOutput::error("it broke"),
            "slow" => {
                let mut guard = Abandoned(Arc::clone(&self.abandoned), true);
                tokio::time::sleep(Duration::from_millis(
                    arguments["ms"].as_u64().unwrap_or(200),
                ))
                .await;
                guard.1 = false;
                ToolOutput::text("slept")
            }
            "mcp__docs-site__search" => ToolOutput::text("found it"),
            other => ToolOutput::error(format!("tool not found: {other}")),
        };
        NestedOutcome {
            id: format!("call/{n}"),
            output,
        }
    }
}

/// Keeps `store()` values in memory, as the session would.
#[derive(Default)]
struct MemoryStore {
    entries: Mutex<Vec<Value>>,
}

#[async_trait]
impl ScriptStore for MemoryStore {
    async fn load(&self) -> Map<String, Value> {
        let mut store = Map::new();
        for entry in self.entries.lock().unwrap().iter() {
            apply_store_entry(&mut store, entry);
        }
        store
    }

    async fn append(&self, writes: &StoreWrites) -> Result<(), String> {
        self.entries.lock().unwrap().push(store_entry(writes));
        Ok(())
    }
}

async fn run_with(codemode: &Codemode, tools: &Tools, code: &str) -> ToolOutput {
    let context = ToolContext::new("cm", micro_tools::Progress::default()).with_tools(tools);
    codemode.call(&json!({ "code": code }), &context).await
}

async fn run(code: &str) -> ToolOutput {
    run_with(&Codemode::new(), &Tools::default(), code).await
}

/// The output after the header.
fn said(output: &ToolOutput) -> String {
    output.content[1..]
        .iter()
        .map(ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn output_returns_and_logs_come_back_in_order() {
    let output = run("console.log('one', 2); text({ three: 3 }); return [4];").await;
    assert!(!output.is_error, "{output:?}");
    let header = output.content[0].as_text();
    assert!(header.starts_with("Script completed\nWall time "), "{header}");
    assert!(header.ends_with(" seconds\nOutput:\n"), "{header}");
    assert_eq!(said(&output), "one 2\n{\"three\":3}\n[4]");
}

#[tokio::test]
async fn tools_are_called_and_a_failure_rejects_with_its_text() {
    let output = run(r#"
        const file = await tools.read({ path: "a.txt" });
        try { await tools.fail({}); } catch (error) { text("caught: " + error.message); }
        return file;
    "#)
    .await;
    assert_eq!(said(&output), "caught: it broke\ncontents of a.txt");
}

#[tokio::test]
async fn a_tool_with_an_output_schema_resolves_to_its_data_even_when_it_failed() {
    let output = run("const ran = await tools.bash({}); return ran.exit_code + ' ' + ran.output;").await;
    assert_eq!(said(&output), "2 boom\n");
}

#[tokio::test]
async fn calls_from_promise_all_run_side_by_side() {
    let started = Instant::now();
    let output = run(r#"
        const all = await Promise.all([1, 2, 3].map(() => tools.slow({ ms: 300 })));
        return all.length;
    "#)
    .await;
    assert_eq!(said(&output), "3");
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "three 300 ms calls took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn calls_still_running_when_the_script_ends_are_cancelled() {
    let tools = Tools::default();
    let output = run_with(
        &Codemode::new(),
        &tools,
        "tools.slow({ ms: 5000 }); return 'left early';",
    )
    .await;
    assert_eq!(said(&output), "left early");
    assert!(tools.abandoned.load(Ordering::SeqCst), "the slow call was dropped");
}

#[tokio::test]
async fn a_misspelled_tool_names_the_close_match() {
    let output = run("await tools.Read({ path: 'x' });").await;
    assert!(output.is_error);
    let said = said(&output);
    assert!(said.contains("tools.Read does not exist. Did you mean tools.read?"), "{said}");
    assert!(said.contains("No tool calls were made."), "{said}");
    assert!(output.content[0].as_text().starts_with("Script failed"));
}

#[tokio::test]
async fn odd_tool_names_are_called_by_their_identifier_or_their_name() {
    let output = run(r#"
        const a = await tools.mcp__docs_site__search({});
        const b = await tools["mcp__docs-site__search"]({});
        return a + b;
    "#)
    .await;
    assert_eq!(said(&output), "found itfound it");
}

#[tokio::test]
async fn a_syntax_error_fails_the_script() {
    let output = run("return (;").await;
    assert!(output.is_error);
    assert!(said(&output).contains("SyntaxError"), "{}", said(&output));
}

#[tokio::test]
async fn a_thrown_error_keeps_the_output_before_it_and_lists_the_calls() {
    let output = run(r#"
        text("before");
        await tools.read({ path: "x" });
        throw new RangeError("too far");
    "#)
    .await;
    assert!(output.is_error);
    let said = said(&output);
    assert!(said.starts_with("before\nScript error:\nRangeError: too far"), "{said}");
    assert!(said.contains("they are not undone): read (ok)"), "{said}");
    assert!(!said.contains("codemode-prelude"), "{said}");
}

#[tokio::test]
async fn a_script_past_its_deadline_is_stopped() {
    let output = run("// @options: {\"timeout_ms\": 200}\nwhile (true) {}").await;
    assert!(output.is_error);
    assert!(said(&output).contains("Script timed out"), "{}", said(&output));
}

#[tokio::test]
async fn a_promise_that_can_never_settle_fails_at_once() {
    let started = Instant::now();
    let output = run("await new Promise(() => {}); return 'never';").await;
    assert!(output.is_error);
    assert!(said(&output).contains("can never settle"), "{}", said(&output));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn exit_ends_the_script_successfully() {
    let output = run("text('bye'); exit(); text('unreachable');").await;
    assert!(!output.is_error);
    assert_eq!(said(&output), "bye");
}

#[tokio::test]
async fn there_are_no_timers_or_node_apis() {
    let output = run("return [typeof setTimeout, typeof require, typeof process, typeof fetch].join();").await;
    assert_eq!(said(&output), "undefined,undefined,undefined,undefined");
}

#[tokio::test]
async fn images_are_checked_before_they_are_shown() {
    let output = run(&format!(
        "image('data:image/jpeg;base64,{PNG}'); image({{ type: 'image', data: '{PNG}', mimeType: 'image/png' }});"
    ))
    .await;
    assert!(!output.is_error, "{output:?}");
    let images: Vec<_> = output
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Image { mime_type, .. } => Some(mime_type.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(images, vec!["image/png", "image/png"], "the detected type wins");

    for (bad, why) in [
        ("'https://example.com/a.png'", "remote image URLs"),
        ("'data:image/png;base64,abc'", "not valid base64"),
        ("'data:image/png;base64,AAAA'", "not a PNG, JPEG, GIF, or WebP"),
        ("{ type: 'text', text: 'x' }", "only accepts MCP image blocks"),
    ] {
        let output = run(&format!("image({bad});")).await;
        assert!(output.is_error, "{bad}");
        assert!(said(&output).contains(why), "{bad}: {}", said(&output));
    }
}

#[tokio::test]
async fn long_output_keeps_both_ends_and_the_rest_goes_to_a_file() {
    let output = run("// @options: {\"max_output_tokens\": 10}\ntext('a'.repeat(100) + 'z'.repeat(100));").await;
    let said = said(&output);
    assert!(said.starts_with("Warning: truncated output (original token count: 50)"), "{said}");
    assert!(said.contains("aaaaaaaaaaaaaaaaaaaa…40 tokens truncated…zzzzzzzzzzzzzzzzzzzz"), "{said}");
    let path = said
        .split("[Full output: ")
        .nth(1)
        .and_then(|rest| rest.split(" (read with").next())
        .expect("the file is named");
    assert_eq!(std::fs::read_to_string(path).unwrap().len(), 200);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn stored_values_last_across_scripts_but_only_from_scripts_that_succeed() {
    let store = Arc::new(MemoryStore::default());
    let codemode = Codemode::new().with_store(store.clone());
    let tools = Tools::default();

    run_with(&codemode, &tools, "store('cursor', { page: 2 }); store('gone', 1);").await;
    run_with(&codemode, &tools, "store('gone', undefined);").await;
    run_with(&codemode, &tools, "store('cursor', 'lost'); throw new Error('no');").await;
    let output = run_with(
        &codemode,
        &tools,
        "return [load('cursor'), load('gone') === undefined];",
    )
    .await;
    assert_eq!(said(&output), "[{\"page\":2},true]");
    assert_eq!(store.entries.lock().unwrap().len(), 2);

    let too_big = run_with(
        &codemode,
        &tools,
        &format!("store('big', 'x'.repeat({MAX_STORE_VALUE_CHARS}));"),
    )
    .await;
    assert!(said(&too_big).contains("store() is for small state"), "{}", said(&too_big));
}

#[tokio::test]
async fn scripts_find_tools_that_are_not_listed() {
    let output = run(r#"
        const found = await searchTools("documentation search", { limit: 2 });
        const namespace = await describeNamespace("docs-site");
        const described = await describeTool("mcp__docs_site__search");
        const missing = await describeTool("nothing");
        return {
            found: found.map((tool) => tool.name),
            namespace,
            described: described.includes("mcp__docs_site__search(args:"),
            missing: missing === undefined,
            all: ALL_TOOLS.length,
        };
    "#)
    .await;
    let value: Value = serde_json::from_str(&said(&output)).expect(&said(&output));
    assert_eq!(value["found"], json!(["mcp__docs_site__search"]));
    assert_eq!(value["namespace"]["name"], "mcp__docs-site");
    assert_eq!(value["namespace"]["instructions"], "Search before reading.");
    assert_eq!(value["namespace"]["tools"], json!(["mcp__docs_site__search"]));
    assert_eq!(value["described"], true);
    assert_eq!(value["missing"], true);
    assert_eq!(value["all"], 5);
}

#[tokio::test]
async fn bad_input_is_explained_without_running_anything() {
    let tools = Tools::default();
    let output = run_with(&Codemode::new(), &tools, "// @options: {\"what\": 1}\nreturn 1;").await;
    assert!(output.is_error);
    assert!(output.text_content().contains("`what`"));
    assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn without_a_run_to_call_through_there_are_no_tools() {
    let context = ToolContext::new("cm", micro_tools::Progress::default());
    let output = Codemode::new()
        .call(&json!({ "code": "return ALL_TOOLS.length;" }), &context)
        .await;
    assert_eq!(said(&output), "0");
}

/// A global the host offers beyond the built-in ones.
struct Echo;

#[async_trait]
impl ScriptGlobals for Echo {
    fn names(&self) -> Vec<String> {
        vec!["helpers.echo".to_string()]
    }

    fn describe(&self) -> String {
        "`helpers.echo(...values)`: hands the values back".to_string()
    }

    async fn call(&self, _name: &str, arguments: Value) -> Result<Option<Value>, String> {
        Ok(Some(arguments))
    }
}

#[tokio::test]
async fn further_globals_are_reachable_and_guarded() {
    let codemode = Codemode::new().with_globals(Arc::new(Echo)).unwrap();
    let output = run_with(&codemode, &Tools::default(), "return await helpers.echo(1, 'two');").await;
    assert_eq!(said(&output), "[1,\"two\"]");
    let missing = run_with(&codemode, &Tools::default(), "helpers.ecko();").await;
    assert!(said(&missing).contains("Did you mean helpers.echo?"), "{}", said(&missing));
    assert!(codemode.definition().description.contains("`helpers.echo(...values)`"));

    struct Shadowing;
    #[async_trait]
    impl ScriptGlobals for Shadowing {
        fn names(&self) -> Vec<String> {
            vec!["tools".to_string()]
        }
        fn describe(&self) -> String {
            String::new()
        }
        async fn call(&self, _name: &str, _arguments: Value) -> Result<Option<Value>, String> {
            Ok(None)
        }
    }
    assert!(Codemode::new().with_globals(Arc::new(Shadowing)).is_err());
}

#[test]
fn store_entries_apply_in_order_and_odd_ones_are_ignored() {
    let mut store = Map::new();
    apply_store_entry(&mut store, &json!({ "set": { "a": 1, "b": 2 }, "delete": [] }));
    apply_store_entry(&mut store, &json!({ "set": { "c": 3 }, "delete": ["a"] }));
    apply_store_entry(&mut store, &json!({ "set": "nonsense" }));
    assert_eq!(Value::Object(store), json!({ "b": 2, "c": 3 }));
}

#[test]
fn namespaces_are_named_in_every_spelling() {
    for query in ["mcp__dev-radius", "mcp__dev_radius", "dev-radius", "dev_radius"] {
        assert!(is_namespace_name("mcp__dev-radius", query), "{query}");
    }
    assert!(!is_namespace_name("mcp__dev-radius", "radius"));
}
