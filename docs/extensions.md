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

Capabilities control access to micro's host API. Bun itself runs in a separate process sandbox with no inherited environment, network, or write access and with reads limited to the host and loaded extension packages. See [Security model](security.md).

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

## Run commands

`ctx.exec` runs a command through micro's command sandbox:

```ts
const result = await ctx.exec("git", ["status", "--short"]);
```

The result includes stdout, stderr, exit status, and sandbox-denial fields. The extension needs the `exec` capability.

The Bun host has its own read-allowlisted sandbox. `ctx.exec` is separate: it requires the `exec` capability and the requested command still runs under the session's command policy. If the session is `workspace-write`, an extension cannot use `ctx.exec` to write outside the workspace or use the network.

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

The compatibility suite runs the examples under `examples/extensions` against the real micro binary. APIs tied to pi's own agent loop, session runtime, interactive mode, or terminal image protocols do not have micro equivalents and return named runtime errors.

See the [extension examples on GitHub](https://github.com/rmonvfer/micro/tree/main/examples/extensions) and [Testing extensions](extension-testing.md).

## Load failures

A failed extension does not stop the session. micro prints:

```text
note: <path> was not loaded: <reason>
```

In `--print` mode, an invoked command whose extension handler throws causes a non-zero process exit and writes the error to stderr.
