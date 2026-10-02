# Extensions

Extensions add tools, slash commands, key bindings, event handlers, providers, and terminal UI components. They are TypeScript modules loaded by a separate Bun process.

Bun is required only when extensions are enabled. The Rust agent continues to work without it.

## Write a single-file extension

Create `.micro/extensions/hello.ts` in a project:

```ts
export const capabilities = ["commands", "ui"];

export default (micro) => {
  micro.registerCommand("hello", {
    description: "say hello",
    handler: async (args, ctx) => {
      ctx.ui.notify(`hello ${args || "world"}`);
      return "done";
    },
  });
};
```

Trust the project and start micro. `/hello Ramon` runs the command.

A single file needs no build or installation step. User extensions placed in micro's own `extensions/` directory load for every project.

Load another file for one run with:

```bash
micro --extension ./path/to/extension.ts
```

## Capabilities

An extension declares which parts of the host API it needs:

```ts
export const capabilities = ["tools", "commands", "exec", "ui"];
```

Packages may declare the same list in `package.json`:

```json
{
  "name": "@scope/name",
  "micro": {
    "extensions": ["./src/index.ts"],
    "capabilities": ["tools", "exec"]
  }
}
```

Available capability names are:

```text
tools              commands           events
exec               builtin_tools      provider_stream
send_user_message  send_message       session_write
session_control    context            ui
providers          flags
```

Read-only getters do not require a capability. Host operations outside the declared set return a named error. The session continues and the request is recorded as an `extension_crossing` event.

Extensions without a manifest use a compatibility path. micro determines the capabilities they request and may ask for a one-time decision. The answer is saved in `capabilities.json`.

Capabilities control access to micro's host API. Bun runs in a separate process sandbox with no inherited environment, no network access, and read-only access to the active workspace and loaded packages. See [Security model](security.md).

## Register a tool

```ts
import { Type } from "@earendil-works/pi-ai";

export const capabilities = ["tools"];

export default (micro) => {
  micro.registerTool({
    name: "greet",
    label: "Greeting",
    description: "Generate a greeting",
    parameters: Type.Object({
      name: Type.String({ description: "Name to greet" }),
    }),
    async execute(_callId, params) {
      return {
        content: [{ type: "text", text: `Hello, ${params.name}` }],
        details: {},
      };
    },
  });
};
```

Registered tools are offered to the model unless the active tool allowlist excludes them.

### Tool exposure

`exposure` controls how the model reaches a tool. "Callable" means callable from other tools through `ctx.executeTool()`, as [`codemode`](codemode.md) scripts call them. `direct`, the default, is declared to the model and callable. `model-only` is declared but never callable, for tools that orchestrate other tools or ask the user. `codemode` is callable and listed in the `codemode` description, but not declared. `deferred` is callable and found by `tool_search` or a script's `searchTools()`, but never listed. `hidden` is registered but unreachable.

`namespace: { name, description, instructions }` groups related tools under one heading in the `codemode` description; scripts read `instructions` with `describeNamespace(name)`. `annotations` carry the MCP hints `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint`; they are not verified, and `getAllTools()` reports them with each tool's exposure and namespace.

A tool that declares `outputSchema` returns its answer as data in `structuredContent`, which scripts receive instead of the text. A result with `isError: true` reaches the model as an error while the call itself succeeded.

```ts
micro.registerTool({
  name: "issue_count",
  description: "Count open issues",
  parameters: Type.Object({ label: Type.String() }),
  exposure: "codemode",
  namespace: { name: "tracker", description: "The issue tracker" },
  outputSchema: { type: "object", properties: { open: { type: "number" } }, required: ["open"] },
  async execute(_callId, params) {
    const open = await countIssues(params.label);
    return { content: [{ type: "text", text: `${open} open` }], structuredContent: { open } };
  },
});
```

### Call other tools

A tool's `ctx.tools` lists the tools it may call, and `ctx.executeTool(name, args)` calls one through the same checks as a call the model makes: `tool_call` and `tool_result` handlers, the sandbox, and approval. It resolves to `{ toolCall, result, isError }` and does not throw when the tool fails. Each nested call emits `tool_execution_start` and `tool_execution_end` with `parentToolCallId`, and the calling tool's result records the calls as `nestedCalls`, bounded to 256 calls and 32 KiB of arguments. Tokens a nested call reports in `usage` are added to the calling tool's result.

```ts
async execute(_callId, params, _signal, _onUpdate, ctx) {
  const { result, isError } = await ctx.executeTool("read", { path: params.path });
  return { content: isError ? [{ type: "text", text: "unreadable" }] : result.content };
}
```

## Run commands

`micro.exec` runs a command through micro's command sandbox:

```ts
const result = await micro.exec("git", ["status", "--short"]);
```

The result includes stdout, stderr, exit status, and sandbox-denial fields. The extension needs the `exec` capability.

The Bun host has its own process sandbox and cannot write files directly. Use brokered APIs such as `micro.exec` or session methods when an extension needs to change state. `micro.exec` requires the `exec` capability, and the requested command runs under the session's command policy. Under `workspace-write`, it cannot write outside the workspace or use the network.

On a platform where micro cannot enforce the Bun-host sandbox, extensions do not run.

## Events

Register a handler with `micro.on(name, handler)`.

Common lifecycle events include:

- `session_start` and `shutdown`;
- `agent_start`, `agent_end`, and `agent_settled`;
- `turn_start` and `turn_end`;
- `message_start`, `message_update`, and `message_end`;
- `tool_execution_start`, `tool_execution_update`, and `tool_execution_end`.

An exception in one handler is reported without preventing other handlers from running.

A `tool_call` handler runs before a tool does. Returning `{ block: true, reason }` stops the call, and the model reads the reason in place of the tool's output. Adding `terminate: true` also ends the run when every call in that batch was blocked this way, so micro does not ask the model again; a batch where any call ran or was blocked without it continues as usual.

```ts
micro.on("tool_call", (event) => {
  if (event.toolName === "bash") {
    return { block: true, reason: "Shell access is off for this task", terminate: true };
  }
});
```

## Terminal UI

`ctx.ui` provides notifications, prompts, selectors, editors, status text, widgets, headers, footers, overlays, autocomplete, and custom editor components.

A component implements `render(width)` and may implement `handleInput(data)`. The component remains in the extension process; micro requests rendered lines over the host pipe.

UI calls require the `ui` capability. Headless modes cannot satisfy interactive prompts.

## Install packages

Use a package when the extension has several files or its own dependencies:

```bash
micro install npm:@scope/name
micro install git:github.com/user/repo
micro install ./some/directory
micro install --local ./some/directory
```

Global packages load in every project. `--local` installs for the current project.

List and remove packages with:

```bash
micro list
micro remove npm:@scope/name
micro remove --local ./some/directory
```

Dependencies are fetched during installation, not during an agent session.

## Deactivation

An extension may export a cleanup function:

```ts
export const deactivate = () => watcher.close();
```

micro calls it when the package is removed. Registered tools, commands, and UI components are withdrawn whenever the extension host stops, including an unexpected host exit.

## pi compatibility

micro accepts `micro.extensions` and `pi.extensions` entries in `package.json`. Extensions may import the `@earendil-works/pi-*` and older `@mariozechner/*` package names supplied by the host compatibility layer.

The compatibility suite checks how the examples under `examples/extensions` load against the real micro binary and fails if any example cannot complete a plain turn. APIs tied to pi's own agent loop, session runtime, interactive mode, or terminal image protocols do not have micro equivalents and return named runtime errors.

See the [extension examples on GitHub](https://github.com/rmonvfer/micro/tree/main/examples/extensions) and [Testing extensions](extension-testing.md).

## Load failures

A failed extension does not stop the session. micro prints:

```text
note: <path> was not loaded: <reason>
```

In `--print` mode, an invoked command whose extension handler throws causes a non-zero process exit and writes the error to stderr.
