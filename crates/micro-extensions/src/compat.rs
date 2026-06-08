//! A place for pi's own runtime modules to resolve to.

use std::path::Path;
use std::path::PathBuf;

const PI_TUI_FILES: &[(&str, &str)] = &[
    ("index.ts", include_str!("../host/compat/tui/index.ts")),
    ("tui.ts", include_str!("../host/compat/tui/tui.ts")),
    ("utils.ts", include_str!("../host/compat/tui/utils.ts")),
    ("keys.ts", include_str!("../host/compat/tui/keys.ts")),
    (
        "keybindings.ts",
        include_str!("../host/compat/tui/keybindings.ts"),
    ),
    ("fuzzy.ts", include_str!("../host/compat/tui/fuzzy.ts")),
    (
        "kill-ring.ts",
        include_str!("../host/compat/tui/kill-ring.ts"),
    ),
    (
        "undo-stack.ts",
        include_str!("../host/compat/tui/undo-stack.ts"),
    ),
    (
        "word-navigation.ts",
        include_str!("../host/compat/tui/word-navigation.ts"),
    ),
    (
        "autocomplete.ts",
        include_str!("../host/compat/tui/autocomplete.ts"),
    ),
    (
        "layout-node.ts",
        include_str!("../host/compat/tui/layout-node.ts"),
    ),
    ("latex.ts", include_str!("../host/compat/tui/latex.ts")),
    (
        "terminal-image.ts",
        include_str!("../host/compat/tui/terminal-image.ts"),
    ),
    (
        "components/box.ts",
        include_str!("../host/compat/tui/components/box.ts"),
    ),
    (
        "components/spacer.ts",
        include_str!("../host/compat/tui/components/spacer.ts"),
    ),
    (
        "components/text.ts",
        include_str!("../host/compat/tui/components/text.ts"),
    ),
    (
        "components/input.ts",
        include_str!("../host/compat/tui/components/input.ts"),
    ),
    (
        "components/stack.ts",
        include_str!("../host/compat/tui/components/stack.ts"),
    ),
    (
        "components/h-stack.ts",
        include_str!("../host/compat/tui/components/h-stack.ts"),
    ),
    (
        "components/v-stack.ts",
        include_str!("../host/compat/tui/components/v-stack.ts"),
    ),
    (
        "components/truncated-text.ts",
        include_str!("../host/compat/tui/components/truncated-text.ts"),
    ),
    (
        "components/loader.ts",
        include_str!("../host/compat/tui/components/loader.ts"),
    ),
    (
        "components/scroll-view.ts",
        include_str!("../host/compat/tui/components/scroll-view.ts"),
    ),
    (
        "components/select-list.ts",
        include_str!("../host/compat/tui/components/select-list.ts"),
    ),
    (
        "components/settings-list.ts",
        include_str!("../host/compat/tui/components/settings-list.ts"),
    ),
    (
        "components/cancellable-loader.ts",
        include_str!("../host/compat/tui/components/cancellable-loader.ts"),
    ),
    (
        "components/editor.ts",
        include_str!("../host/compat/tui/components/editor.ts"),
    ),
    (
        "components/markdown.ts",
        include_str!("../host/compat/tui/components/markdown.ts"),
    ),
];

const PI_CODING_AGENT_FILES: &[(&str, &str)] = &[(
    "index.ts",
    include_str!("../host/compat/coding-agent/index.ts"),
)];

const PI_AGENT_CORE_FILES: &[(&str, &str)] = &[(
    "index.ts",
    include_str!("../host/compat/agent-core/index.ts"),
)];

const PI_AI_FILES: &[(&str, &str)] = &[
    ("index.ts", include_str!("../host/compat/ai/index.ts")),
    ("compat.ts", include_str!("../host/compat/ai/compat.ts")),
    ("oauth.ts", include_str!("../host/compat/ai/oauth.ts")),
    (
        "providers/all.ts",
        include_str!("../host/compat/ai/providers-all.ts"),
    ),
];

/// `get-east-asian-width` is the one real npm dependency pi-tui's vendored `utils.ts` has.
const EAST_ASIAN_WIDTH_FILES: &[(&str, &str)] = &[(
    "index.ts",
    include_str!("../host/compat/east-asian-width/index.ts"),
)];

const TYPEBOX_FILES: &[(&str, &str)] = &[
    (
        "index.mjs",
        include_str!("../host/compat/typebox/index.mjs"),
    ),
    (
        "compile.mjs",
        include_str!("../host/compat/typebox/compile.mjs"),
    ),
    (
        "value.mjs",
        include_str!("../host/compat/typebox/value.mjs"),
    ),
];

const MARKED_FILES: &[(&str, &str)] =
    &[("index.mjs", include_str!("../host/compat/marked/index.mjs"))];

/// pi's own runtime is published under two npm scopes.
const SCOPES: &[&str] = &["@earendil-works", "@mariozechner"];

struct Package {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

const PACKAGES: &[Package] = &[
    Package {
        name: "pi-tui",
        files: PI_TUI_FILES,
    },
    Package {
        name: "pi-coding-agent",
        files: PI_CODING_AGENT_FILES,
    },
    Package {
        name: "pi-agent-core",
        files: PI_AGENT_CORE_FILES,
    },
    Package {
        name: "pi-ai",
        files: PI_AI_FILES,
    },
];

fn package_json(name: &str) -> String {
    format!(r#"{{"name":"{name}","version":"0.0.0","type":"module"}}"#)
}

fn write_package(
    node_modules: &Path,
    scope: Option<&str>,
    name: &str,
    files: &[(&str, &str)],
) -> Result<(), String> {
    let package_dir = match scope {
        Some(scope) => node_modules.join(scope).join(name),
        None => node_modules.join(name),
    };
    std::fs::create_dir_all(&package_dir)
        .map_err(|error| format!("cannot use {}: {error}", package_dir.display()))?;
    std::fs::write(package_dir.join("package.json"), package_json(name))
        .map_err(|error| format!("cannot write {}'s package.json: {error}", name))?;
    for (relative, source) in files {
        let path = package_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot use {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, source)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Put this layer's `node_modules` where [`node_path`] says it is, and say where that is.
pub fn install(home: &Path) -> Result<PathBuf, String> {
    let node_modules = home.join("node_modules");
    for scope in SCOPES {
        for package in PACKAGES {
            write_package(&node_modules, Some(scope), package.name, package.files)?;
        }
    }
    write_package(
        &node_modules,
        None,
        "get-east-asian-width",
        EAST_ASIAN_WIDTH_FILES,
    )?;

    write_package(&node_modules, None, "typebox", TYPEBOX_FILES)?;
    write_package(&node_modules, Some("@sinclair"), "typebox", TYPEBOX_FILES)?;

    write_package(&node_modules, None, "marked", MARKED_FILES)?;
    write_catalog_json(&node_modules)?;
    Ok(node_modules)
}

/// `ai/catalog.json`, beside `ai/index.ts` in the `pi-ai` package directory under both scopes: pi-
/// ai's own.
fn write_catalog_json(node_modules: &Path) -> Result<(), String> {
    let catalog = micro_models::Catalog::bundled();
    let json = micro_models::catalog_json(&catalog, None);
    let text = serde_json::to_string(&json)
        .map_err(|error| format!("cannot build the model catalog: {error}"))?;

    for scope in SCOPES {
        let path = node_modules.join(scope).join("pi-ai").join("catalog.json");
        std::fs::write(&path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn node_path(home: &Path, compat_node_modules: &Path) -> Result<std::ffi::OsString, String> {
    let npm_node_modules = home.join("npm").join("node_modules");
    std::env::join_paths([
        compat_node_modules.as_os_str(),
        npm_node_modules.as_os_str(),
    ])
    .map_err(|error| format!("cannot build NODE_PATH: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "micro-compat-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn every_package_is_written_under_both_scopes() {
        let home = scratch("both-scopes");
        let node_modules = install(&home).unwrap();
        assert_eq!(node_modules, home.join("node_modules"));

        for scope in SCOPES {
            for package in PACKAGES {
                let package_dir = node_modules.join(scope).join(package.name);
                assert!(
                    package_dir.join("package.json").is_file(),
                    "{} is missing a package.json",
                    package_dir.display()
                );
                for (relative, source) in package.files {
                    let written = std::fs::read_to_string(package_dir.join(relative)).unwrap();
                    assert_eq!(
                        &written, source,
                        "{}/{relative} was not written whole",
                        package.name
                    );
                }
            }
        }
        assert!(node_modules
            .join("get-east-asian-width/package.json")
            .is_file());

        let _ = std::fs::remove_dir_all(&home);
    }

    /// `ai/catalog.json` sits under both scopes, beside the `pi-ai` files `PACKAGES` already
    /// writes, and it is the real bundled catalog.
    #[test]
    fn the_model_catalog_is_written_beside_pi_ai_under_both_scopes() {
        let home = scratch("catalog-json");
        let node_modules = install(&home).unwrap();

        let expected = micro_models::catalog_json(&micro_models::Catalog::bundled(), None);
        for scope in SCOPES {
            let path = node_modules.join(scope).join("pi-ai").join("catalog.json");
            let written: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(written, expected, "{}", path.display());
            assert!(written["providers"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("anthropic")));
            assert!(!written["models"].as_array().unwrap().is_empty());
        }

        let _ = std::fs::remove_dir_all(&home);
    }

    /// `providers-all.ts` reads `./catalog.json` synchronously.
    #[tokio::test]
    async fn providers_all_reads_the_real_catalog_json_synchronously() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("catalog-json-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("catalog-json-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
const { default: catalog } = await import("@earendil-works/pi-ai/catalog.json", { with: { type: "json" } });
console.log(JSON.stringify({
  hasProviders: Array.isArray(catalog.providers) && catalog.providers.length > 0,
  hasModels: Array.isArray(catalog.models) && catalog.models.length > 0,
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .current_dir(&home)
            .env_clear()
            .env("NODE_PATH", &node_path)
            .env("HOME", &home)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["hasProviders"], true);
        assert_eq!(printed["hasModels"], true);

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// typebox is written unscoped and under `@sinclair`, matching every specifier a real extension
    /// might use.
    #[test]
    fn typebox_is_written_under_its_own_name_and_its_sinclair_alias() {
        let home = scratch("typebox");
        let node_modules = install(&home).unwrap();

        for package_dir in [
            node_modules.join("typebox"),
            node_modules.join("@sinclair").join("typebox"),
        ] {
            assert!(
                package_dir.join("package.json").is_file(),
                "{}",
                package_dir.display()
            );
            for (relative, source) in TYPEBOX_FILES {
                let written = std::fs::read_to_string(package_dir.join(relative)).unwrap();
                assert_eq!(
                    &written,
                    source,
                    "{}/{relative} was not written whole",
                    package_dir.display()
                );
            }
        }

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn the_vendored_typebox_actually_works() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("typebox-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("typebox-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { Type } from "typebox";
import { Compile } from "typebox/compile";
const schema = Type.Object({ who: Type.String() });
const check = Compile(schema);
console.log(JSON.stringify({ schema, valid: check.Check({ who: "x" }), invalid: check.Check({ who: 5 }) }));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["schema"]["type"], "object");
        assert_eq!(printed["valid"], true);
        assert_eq!(printed["invalid"], false);

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[test]
    fn marked_is_written_under_its_own_name() {
        let home = scratch("marked");
        let node_modules = install(&home).unwrap();

        let package_dir = node_modules.join("marked");
        assert!(
            package_dir.join("package.json").is_file(),
            "{}",
            package_dir.display()
        );
        for (relative, source) in MARKED_FILES {
            let written = std::fs::read_to_string(package_dir.join(relative)).unwrap();
            assert_eq!(
                &written,
                source,
                "{}/{relative} was not written whole",
                package_dir.display()
            );
        }

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn the_vendored_marked_actually_works() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("marked-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("marked-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { Marked } from "marked";
const marked = new Marked();
console.log(marked.parse("hello world **bold**\n\n# heading"));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed = String::from_utf8_lossy(&output.stdout);
        assert!(printed.contains("<h1>heading</h1>"), "{printed}");
        assert!(printed.contains("<strong>bold</strong>"), "{printed}");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[tokio::test]
    async fn pi_ais_shim_reaches_the_vendored_typebox() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("pi-ai-typebox");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("pi-ai-typebox-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { Type } from "@earendil-works/pi-ai";
console.log(JSON.stringify(Type.String()));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["type"], "string");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[test]
    fn node_path_carries_both_directories() {
        let home = scratch("node-path");
        let node_modules = install(&home).unwrap();
        let joined = node_path(&home, &node_modules).unwrap();
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert!(parts.contains(&node_modules));
        assert!(parts.contains(&home.join("npm").join("node_modules")));
        let _ = std::fs::remove_dir_all(&home);
    }

    fn write_real_micro_session(dir: &Path, workspace: &str) -> PathBuf {
        let log = dir.join("1700000000000.jsonl");
        std::fs::write(
            &log,
            [
                r#"{"id":"1","parent_id":null,"timestamp":1700000000000,"message":{"role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1700000000000}}"#,
                r#"{"id":"2","parent_id":"1","timestamp":1700000001000,"message":{"role":"assistant","content":[{"type":"text","text":"hi there"}],"provider":"anthropic","model":"claude-opus-5","usage":{"input":10,"output":5,"cache_read":0,"cache_write":0},"stop_reason":"stop","timestamp":1700000001000}}"#,
                r#"{"id":"3","parent_id":"2","timestamp":1700000002000,"message":{"role":"tool_result","tool_call_id":"call_1","tool_name":"bash","content":[{"type":"text","text":"output"}],"is_error":false,"timestamp":1700000002000}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("1700000000000.meta.json"),
            serde_json::json!({
                "id": "sess-1", "created_at": 1_700_000_000_000i64, "updated_at": 1_700_000_002_000i64,
                "workspace": workspace, "model_id": "opus", "title": "hello", "message_count": 3
            })
            .to_string(),
        )
        .unwrap();
        log
    }

    /// `SessionManager.open()` against a real micro-native session file.
    #[tokio::test]
    async fn session_manager_reads_a_real_micro_session_and_forks_it_correctly() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("session-manager-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let session_dir = scratch("session-manager-e2e-session");
        let log = write_real_micro_session(&session_dir, "/tmp/a-workspace");

        let script_dir = scratch("session-manager-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { SessionManager } from "@earendil-works/pi-coding-agent";

const sm = SessionManager.open(process.argv[2]);
const branch = sm.getBranch();
const toolResult = branch[2].message as Record<string, unknown>;
const assistant = branch[1].message as Record<string, unknown>;
const usage = assistant.usage as Record<string, unknown>;

const branchedFile = sm.createBranchedSession("2");
const reopened = SessionManager.open(branchedFile as string);

console.log(JSON.stringify({
  branchLength: branch.length,
  parentIds: branch.map((e) => e.parentId),
  toolCallId: toolResult.toolCallId,
  toolName: toolResult.toolName,
  isError: toolResult.isError,
  stopReason: assistant.stopReason,
  cacheRead: usage.cacheRead,
  cwd: sm.getCwd(),
  sessionId: sm.getSessionId(),
  branchedEntries: reopened.getEntries().map((e) => ({ id: e.id, parentId: e.parentId })),
  branchedLeafId: reopened.getLeafId(),
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .arg(&log)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            printed["branchLength"], 3,
            "the whole chain, not just the leaf: {printed}"
        );
        assert_eq!(printed["parentIds"], serde_json::json!([null, "1", "2"]));
        assert_eq!(printed["toolCallId"], "call_1");
        assert_eq!(printed["toolName"], "bash");
        assert_eq!(printed["isError"], false);
        assert_eq!(printed["stopReason"], "stop");
        assert_eq!(printed["cacheRead"], 0);
        assert_eq!(printed["cwd"], "/tmp/a-workspace");
        assert_eq!(printed["sessionId"], "sess-1");
        assert_eq!(
            printed["branchedEntries"],
            serde_json::json!([{"id": "1", "parentId": null}, {"id": "2", "parentId": "1"}]),
            "the fork keeps the whole path up to the branch point, not just its leaf: {printed}"
        );
        assert_eq!(printed["branchedLeafId"], "2");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&session_dir);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[tokio::test]
    async fn pi_ais_pure_logic_actually_works() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("pi-ai-pure-logic");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("pi-ai-pure-logic-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import {
	StringEnum, Type, uuidv7, contentText, isContextOverflow, isRetryableAssistantError,
	retryAssistantCall, parseStreamingJson, createAssistantMessageEventStream,
	validateToolArguments, envApiKeyAuth, defaultProviderAuthContext, createProvider,
	calculateCost, clampThinkingLevel,
} from "@earendil-works/pi-ai";

const results: Record<string, unknown> = {};
const usage = () => ({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } });
const assistant = (overrides: Record<string, unknown>) => ({ role: "assistant", content: [], api: "openai-responses", provider: "openai", model: "x", usage: usage(), stopReason: "error", timestamp: Date.now(), ...overrides });

results.stringEnumType = StringEnum(["a", "b"] as const).type;
results.uuidv7Valid = /^[0-9a-f-]{36}$/i.test(uuidv7());
results.contentText = contentText([{ type: "text", text: "a" }, { type: "thinking", thinking: "skip" } as any, { type: "text", text: "b" }]);
results.overflow = isContextOverflow(assistant({ errorMessage: "prompt is too long: 300000 tokens > 200000 maximum" }) as any);
results.retryable = isRetryableAssistantError(assistant({ errorMessage: "503 service unavailable" }) as any);

let attempts = 0;
const retried = await retryAssistantCall(
	async () => { attempts++; return attempts < 2 ? assistant({ errorMessage: "500 internal error" }) as any : assistant({ stopReason: "stop" }) as any; },
	{ enabled: true, maxRetries: 3, baseDelayMs: 1 },
	undefined,
);
results.retriedAttempts = attempts;
results.retriedStop = retried.stopReason;

results.streamingJson = parseStreamingJson('{"path": "a.ts", "done": true, "tail": "still comin');

const tool = { name: "add", description: "d", parameters: Type.Object({ text: Type.String(), count: Type.Number() }) };
results.validated = validateToolArguments(tool as any, { type: "toolCall", id: "1", name: "add", arguments: { text: "hi", count: "3" } } as any);

process.env.PURE_LOGIC_TEST_KEY = "secret-xyz";
const auth = envApiKeyAuth("Test", ["PURE_LOGIC_TEST_KEY"]);
const authResult = await auth.resolve({ ctx: defaultProviderAuthContext(), credential: undefined, signal: new AbortController().signal });
results.envAuthKey = authResult?.auth.apiKey;

let dispatchedTo: string | undefined;
const model = { id: "m1", name: "M1", api: "openai-responses", provider: "test", baseUrl: "https://example.invalid", reasoning: true, thinkingLevelMap: { high: "high" }, input: ["text"], cost: { input: 3, output: 15, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 };
const provider = createProvider({
	id: "test-provider", auth: { apiKey: auth }, models: [model as any],
	api: {
		stream: (m) => { dispatchedTo = m.id; const s = createAssistantMessageEventStream(); s.end(assistant({ model: m.id, stopReason: "stop" }) as any); return s; },
		streamSimple: (m) => { dispatchedTo = m.id; const s = createAssistantMessageEventStream(); s.end(assistant({ model: m.id, stopReason: "stop" }) as any); return s; },
	},
});
await provider.stream(provider.getModels()[0], { messages: [] }).result();
results.providerDispatchedTo = dispatchedTo;
results.cost = calculateCost(model as any, { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, totalTokens: 1500, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } });
results.clampedThinking = clampThinkingLevel(model as any, "xhigh");

console.log(JSON.stringify(results));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["stringEnumType"], "string");
        assert_eq!(printed["uuidv7Valid"], true);
        assert_eq!(printed["contentText"], "a\nb");
        assert_eq!(printed["overflow"], true);
        assert_eq!(printed["retryable"], true);
        assert_eq!(
            printed["retriedAttempts"], 2,
            "one failed attempt, one retry that succeeds"
        );
        assert_eq!(printed["retriedStop"], "stop");
        assert_eq!(
            printed["streamingJson"],
            serde_json::json!({"path": "a.ts", "done": true, "tail": "still comin"}),
            "a well-formed prefix survives even though the trailing string never closed"
        );
        assert_eq!(
            printed["validated"],
            serde_json::json!({"text": "hi", "count": 3}),
            "typebox coerces the stringified count into a number"
        );
        assert_eq!(printed["envAuthKey"], "secret-xyz");
        assert_eq!(printed["providerDispatchedTo"], "m1");
        assert_eq!(printed["cost"]["input"], 0.003);
        assert!(
            (printed["cost"]["output"].as_f64().unwrap() - 0.0075).abs() < 1e-12,
            "1000 * $3/M + 500 * $15/M: {}",
            printed["cost"]["output"]
        );
        assert_eq!(
            printed["clampedThinking"], "high",
            "xhigh isn't in this model's map, clamps down to the nearest supported level"
        );

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[tokio::test]
    async fn pi_ai_compat_registry_dispatches_and_refuses_unregistered_apis() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("pi-ai-compat-registry");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("pi-ai-compat-registry-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { registerApiProvider, streamSimple, unregisterApiProviders } from "@earendil-works/pi-ai/compat";
import { createAssistantMessageEventStream } from "@earendil-works/pi-ai";

const model = { id: "m1", name: "M1", api: "custom-api", provider: "test", baseUrl: "https://example.invalid", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 } as any;
const assistant = { role: "assistant", content: [{ type: "text", text: "hi" }], api: "custom-api", provider: "test", model: "m1", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } }, stopReason: "stop", timestamp: Date.now() };

registerApiProvider({ api: "custom-api", stream: () => { throw new Error("unused"); }, streamSimple: () => { const s = createAssistantMessageEventStream(); s.end(assistant as any); return s; } }, "src-1");
const dispatched = await streamSimple(model, { messages: [] }).result();

unregisterApiProviders("src-1");
let refusedMessage: string | undefined;
try {
	streamSimple(model, { messages: [] });
} catch (error) {
	refusedMessage = error instanceof Error ? error.message : String(error);
}

console.log(JSON.stringify({ dispatchedText: dispatched.content[0], refusedMessage }));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            printed["dispatchedText"],
            serde_json::json!({"type": "text", "text": "hi"})
        );
        assert_eq!(
            printed["refusedMessage"],
            "No API provider registered for api: custom-api"
        );

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// `../host/compat/ai/oauth.ts` and `../host/compat/ai/providers-all.ts` import cleanly for
    /// their side effect alone.
    #[tokio::test]
    async fn pi_ai_subpath_side_effect_imports_succeed() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("pi-ai-subpaths");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("pi-ai-subpaths-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import "@earendil-works/pi-ai/oauth";
import "@earendil-works/pi-ai/providers/all";
console.log(JSON.stringify({ ok: true }));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["ok"], true);

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    #[tokio::test]
    async fn pi_agent_core_uuid_telemetry_and_agent_boundary() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("pi-agent-core-surface");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("pi-agent-core-surface-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { uuidv7 } from "@earendil-works/pi-ai";
import { uuidv7 as coreUuidv7, InMemoryTelemetryContext, NOOP_TELEMETRY_CONTEXT, Agent } from "@earendil-works/pi-agent-core";

const telemetry = new InMemoryTelemetryContext();
await telemetry.startSpan({ name: "outer" }, async (span) => {
	span.addEvent("tick", { count: 1 });
});
const spans = telemetry.getSpans();

const agent = new Agent({ model: "whatever" });
let agentError: string | undefined;
try {
	(agent as any).run();
} catch (error) {
	agentError = error instanceof Error ? error.message : String(error);
}

console.log(JSON.stringify({
	sameUuidFn: coreUuidv7 === uuidv7,
	spanName: spans[0]?.name,
	spanEvents: spans[0]?.events.length,
	spanSettled: spans[0]?.settled,
	noopWorked: await NOOP_TELEMETRY_CONTEXT.startSpan({ name: "n" }, () => "done"),
	agentErrorMentionsCredentials: agentError?.includes("credentials") ?? false,
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["sameUuidFn"], true);
        assert_eq!(printed["spanName"], "outer");
        assert_eq!(printed["spanEvents"], 1);
        assert_eq!(printed["spanSettled"], true);
        assert_eq!(printed["noopWorked"], "done");
        assert_eq!(printed["agentErrorMentionsCredentials"], true);

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// `Input` and `getKeybindings` were stubs.
    #[tokio::test]
    async fn input_and_keybindings_are_real_not_stubbed() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("input-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("input-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { getKeybindings, Input } from "@earendil-works/pi-tui";

const kb = getKeybindings();
const submitKey = kb.getKeys("tui.input.submit");

const input = new Input();
input.handleInput("hi");
input.handleInput("\x7f"); // backspace: real deleteCharBackward, not a no-op stub
let submitted: string | undefined;
input.onSubmit = (value) => { submitted = value; };
input.handleInput("\r"); // matches tui.input.submit via the real keybinding lookup

console.log(JSON.stringify({
  submitKeyIsEnter: submitKey.includes("enter"),
  valueAfterBackspace: input.getValue(),
  submitted,
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["submitKeyIsEnter"], true);
        assert_eq!(printed["valueAfterBackspace"], "h");
        assert_eq!(printed["submitted"], "h");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// `pi-coding-agent`'s `keyHint`/`BorderedLoader` reach across into `pi-tui` for real.
    #[tokio::test]
    async fn coding_agent_reaches_pi_tui_for_real_keybindings_and_a_bordered_loader() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("coding-agent-pi-tui");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("coding-agent-pi-tui-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { BorderedLoader, keyHint, rawKeyHint } from "@earendil-works/pi-coding-agent";

const hint = await keyHint("tui.select.cancel", "cancel");
const raw = rawKeyHint("ctrl+x", "do a thing");

const theme = { fg: (_color: string, text: string) => text };
const loader = new BorderedLoader({}, theme, "Working...", { cancellable: true });
const before = loader.render(20);

let aborted = false;
loader.onAbort = () => {
  aborted = true;
};
const handled = loader.handleInput("\x1b");

console.log(JSON.stringify({
  hintMentionsEscape: hint.toLowerCase().includes("esc"),
  hintHasDescription: hint.includes("cancel"),
  raw,
  borderLineIsBoxDrawing: /^─+$/.test(before[0]),
  hasMessageLine: before.some((line) => line.includes("Working...")),
  hasCancelHint: before.some((line) => line.includes("cancel")),
  handledConsumed: handled?.consume === true,
  aborted,
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["hintMentionsEscape"], true, "{printed}");
        assert_eq!(printed["hintHasDescription"], true, "{printed}");
        assert_eq!(printed["raw"], "ctrl+x do a thing");
        assert_eq!(printed["borderLineIsBoxDrawing"], true, "{printed}");
        assert_eq!(printed["hasMessageLine"], true, "{printed}");
        assert_eq!(printed["hasCancelHint"], true, "{printed}");
        assert_eq!(printed["handledConsumed"], true, "{printed}");
        assert_eq!(printed["aborted"], true, "{printed}");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// `pi-coding-agent`'s `CustomEditor` genuinely subclasses `pi-tui`'s real `Editor`.
    #[tokio::test]
    async fn custom_editor_subclasses_the_real_pi_tui_editor() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("custom-editor-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("custom-editor-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r#"
import { CustomEditor, getSelectListTheme } from "@earendil-works/pi-coding-agent";

class ModalEditor extends CustomEditor {}

const tui = { requestRender() {}, terminal: { rows: 24 } };
const theme = { borderColor: (t: string) => t, selectList: getSelectListTheme() };

const editor = new ModalEditor(tui, theme, {});

editor.handleInput("h");
editor.handleInput("i");

let escaped = false;
editor.onEscape = () => {
  escaped = true;
};
editor.handleInput("\x1b");

console.log(JSON.stringify({
  isCustomEditor: editor instanceof CustomEditor,
  text: editor.getText(),
  escaped,
}));
"#,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed["isCustomEditor"], true);
        assert_eq!(printed["text"], "hi");
        assert_eq!(printed["escaped"], true, "{printed}");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    /// `getMarkdownTheme()`'s answer satisfies pi-tui's real `Markdown` component fully.
    #[tokio::test]
    async fn markdown_theme_satisfies_the_real_markdown_component() {
        let Some(bun) = crate::host::which_bun() else {
            return;
        };
        let home = scratch("markdown-theme-e2e");
        let node_modules = install(&home).unwrap();
        let node_path = node_path(&home, &node_modules).unwrap();

        let script_dir = scratch("markdown-theme-e2e-script");
        let script = script_dir.join("check.ts");
        std::fs::write(
            &script,
            r##"
import { getMarkdownTheme } from "@earendil-works/pi-coding-agent";
import { Markdown } from "@earendil-works/pi-tui";

const markdown = new Markdown("Heading\n=======\n\n[a link](https://example.com)", 0, 0, getMarkdownTheme());
const lines = markdown.render(60);
console.log(JSON.stringify({ lineCount: lines.length, hasContent: lines.some((l: string) => l.length > 0) }));
"##,
        )
        .unwrap();

        let output = tokio::process::Command::new(&bun)
            .arg("run")
            .arg("--no-install")
            .arg(&script)
            .env("NODE_PATH", &node_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(printed["lineCount"].as_u64().unwrap() > 0, "{printed}");
        assert_eq!(printed["hasContent"], true, "{printed}");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&script_dir);
    }
}
