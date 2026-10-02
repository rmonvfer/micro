# Codemode

The `codemode` tool lets the model write a JavaScript script that calls micro's other tools. Only what the script outputs reaches the model, so a script can run calls side by side and filter large results before the model reads them.

micro offers `codemode` when the tool selection names it, as in `micro --tools read,bash,codemode`, and when an MCP server has the default `codemode` exposure (see [MCP servers](mcp.md#control-exposure)). `--exclude-tools codemode` withholds it.

## Scripts

The tool input is raw JavaScript, run as the body of an async function in a QuickJS sandbox, so top-level `await` and `return` work. The sandbox has no Node APIs, file system, network, or timers; a script reaches the outside world only through tools.

A script may start with an options line:

```js
// @options: {"max_output_tokens": 2000, "timeout_ms": 60000}
const [manifest, lock] = await Promise.all([
  tools.read({ path: "Cargo.toml" }),
  tools.read({ path: "Cargo.lock" }),
]);
return { manifest: manifest.length, lock: lock.length };
```

`max_output_tokens` (10000 by default) limits the output. Longer output keeps its start and end, and the full text is written to a temporary file named in the result. `timeout_ms` is a deadline for the whole script, tool calls included; there is none by default.

The result starts with `Script completed` or `Script failed`, the wall time, and the output. A failed script keeps the output it produced before failing, followed by `Script error:`, the error, and the tool calls it made, which are not undone. Calls still running when the script ends are cancelled, and promises it never awaited are dropped.

## Globals

`tools.<name>(args)` calls a tool. `text(value)` adds output, strings as they are and other values as JSON; `console.log()` and its siblings do the same, and a top-level `return` adds its value. `image(value)` adds an image from a base64 `data:` URL, an `{ image_url }` object, or an image block `{ type: "image", data, mimeType }` such as MCP tools return; PNG, JPEG, GIF, and WebP are accepted, and remote URLs are not. `exit()` ends the script successfully.

`ALL_TOOLS` lists every callable tool as `{ name, description }`. `searchTools(query, { limit?, namespace? })` ranks them by relevance with BM25 (eight by default), `describeTool(name)` resolves to a tool's description and TypeScript declaration, and `describeNamespace(name)` to `{ name, description?, instructions?, tools }` for a group of tools such as one MCP server. A namespace is named as `mcp__dev-radius`, `mcp__dev_radius`, `dev-radius`, or `dev_radius`.

Reading a tool that does not exist throws an error naming the close matches, so `tools.Bash` suggests `tools.bash`. Use `"name" in tools` to check for a tool.

## Call tools

A tool is a method of `tools` named by its identifier: characters that are not valid in a JavaScript identifier become `_`, so `mcp__dev-radius__search` is `tools.mcp__dev_radius__search`. `tools["mcp__dev-radius__search"]` works too. Each method takes one object with the tool's arguments.

Scripts call tools through the session, so a call goes through the same checks as one the model makes: extension `tool_call` and `tool_result` handlers, the sandbox, and approval. Each call is reported under the `codemode` call and recorded on its result.

What a call resolves to depends on the tool. `bash` resolves to `{ output, truncated, full_output_path?, exit_code, wall_time_seconds }`, also for a non-zero exit code; its `output` holds up to 1 MiB, and longer output keeps its first and last 512 KiB with `truncated` set and the whole output in `full_output_path`. MCP tools resolve to their `CallToolResult`, `isError` and `structuredContent` included. Extension tools that declare an `outputSchema` resolve to their `structuredContent`. Other tools resolve to their text. A call that fails, is refused, or gets invalid arguments rejects with an `Error` carrying the tool's error text; `Promise.allSettled()` keeps the results of the calls that succeed.

```js
const results = await Promise.allSettled(["a.rs", "b.rs"].map((path) => tools.read({ path })));
const missing = results.filter((result) => result.status === "rejected").length;
const built = await tools.bash({ command: "cargo check --quiet" });
return { missing, exit: built.exit_code };
```

## Which tools are listed

The `codemode` description lists callable tools with their TypeScript declarations, grouped by namespace. Tools with `deferred` exposure, which includes MCP tools, are not listed, so the description stays the same while servers connect; scripts find them with `searchTools()`, `describeNamespace()`, or `ALL_TOOLS`. A script that mentions `mcp__`, `ALL_TOOLS`, or one of the search helpers first waits for servers still connecting. Listed declarations share a budget of 3000 estimated tokens, set with `codemode.inline_budget`.

`codemode.mode` decides how the declared tools are presented while `codemode` is offered. With `on` (the default) they stay declared, and each description says how a script calls the tool and what the call resolves to. With `only` they are left out of requests and listed in the `codemode` description instead, so the model calls them through scripts.

```json
{ "codemode": { "mode": "only", "inline_budget": 5000 } }
```

## Store values

`store(key, value)` keeps a JSON value for later scripts in the same session; storing `undefined` deletes the key, and `load(key)` reads a value back. Writes are kept only when the script succeeds: each successful script that stored something appends a `codemode-store` entry to the session, so a resumed session keeps the values and each branch sees only those written along it.

The store is for small state such as IDs, cursors, and summaries. One value may hold 262144 characters of JSON, and all values together 1048576. Show images with `image()` instead of storing them.

## Limits

A script's VM has 256 MB of memory; running out throws `InternalError: out of memory`, so filter or aggregate data instead of accumulating it. A script waiting on a promise that can never settle, with no tool call pending, fails at once, since there are no timers. Scripts cannot start other `codemode` scripts.
