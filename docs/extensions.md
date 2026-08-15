# Extensions

An extension is a TypeScript file that micro loads at startup and hands an API to. It can
add tools the model may call, commands you may type, and handlers that run when something
happens. It can also draw its own pieces of the interface.

Extensions run in a Bun process of their own. Nothing in an extension reaches the model or
the workspace directly: it asks, micro decides, and micro acts. That is what keeps a
third-party file under the same rules as everything else.

## Writing one

An extension exports a default function. micro calls it once with the API.

```ts
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

Drop that in `.micro/extensions/hello.ts` in your project and run `micro`. Typing `/hello`
runs it. There is no build step and no install step for a single file — micro reads it
where it lies.

A project's own extensions load only once you have trusted the project. Extensions in
`~/.micro/extensions` are yours and load everywhere.

## What the API offers

`micro.registerTool` adds a tool the model may call. `micro.registerCommand` adds a slash
command. `micro.registerShortcut` binds a key. `micro.registerFlag` adds a command-line
flag, which `micro.getFlag` reads back. `micro.on` listens for something happening.

For the session itself, `getModel`, `getThinkingLevel`, `getSessionName` and
`getActiveTools` report what is in force, and `setModel`, `setThinkingLevel`,
`setSessionName` and `setActiveTools` change it. `sendUserMessage` submits a prompt as
though it had been typed. `exec` runs a command and reports what it printed. `appendEntry`
puts something in the session that the model never sees.

A getter that pi answers immediately answers immediately here too. `getActiveTools()`
returns an array, not a promise, because an extension written for pi calls it without
`await` and expects to be able to filter the result.

## Events

`micro.on(name, handler)` runs a handler when something happens. A handler that throws is
reported and the others still run, so one extension cannot silence another.

The lifecycle of a run reports `agent_start`, then `turn_start` and `turn_end` around each
exchange, then `agent_end` and `agent_settled` when there is nothing left to do. An
interrupted run still reports its end, so a handler tracking whether the agent is busy does
not get stuck believing a turn never finished.

While a response arrives, `message_start`, `message_update` and `message_end` carry it. A
tool call reports `tool_execution_start`, `tool_execution_update` and `tool_execution_end`.
Session lifetime reports `session_start` and `shutdown`.

Message fields are spelled the way pi spells them — `toolCallId`, `isError`, `stopReason` —
so a handler reading a message finds what it is looking for.

## Drawing

`ctx.ui` is how an extension reaches the interface. `notify`, `setStatus` and `select`,
`confirm`, `input` and `editor` ask the person something and wait for the answer.

`setWidget`, `setHeader` and `setFooter` take a component and put it on the screen.
`custom` opens a component as a focused overlay that holds the keyboard until it is done.
`setEditorComponent` replaces the input area entirely, which is how a vim-style editor is
written. `addAutocompleteProvider` offers completions for words beginning with a character
you register.

A component is an object with a `render(width)` that returns lines, and optionally a
`handleInput(data)` that says whether it consumed the key. The object stays in the
extension's own process; micro calls it by name across the pipe and draws what it answers
with. A component may also redraw itself on its own schedule.

`ctx.ui.theme` carries the colours and the text styles: `theme.fg(name, text)` for colour,
and `bold`, `italic`, `underline`, `inverse` and `strikethrough` for the rest.

## Installing a package

An extension that is more than one file, or that has dependencies of its own, is a package.

```
micro install npm:@scope/name
micro install git:github.com/user/repo
micro install ./some/directory
micro install -l ./some/directory
```

`-l` installs into the current project rather than for every project. `micro list` shows
what is installed and `micro remove` takes one away.

Installing fetches the package's own dependencies. That happens when you install, never
while micro is running: an import inside a session resolves to what was installed, and
nothing is fetched behind your back. If an extension fails to load because a package it
declares is missing, micro says which package and which directory to install it in.

A package says what to load in its own `package.json`:

```json
{
  "name": "@scope/name",
  "pi": { "extensions": ["./src/index.ts"] }
}
```

`micro`, `ohm` and `pi` are all read as the same field, so a package published for any of
them loads here.

## Extensions written for pi

An extension written for pi runs here unmodified, including one that imports from pi's own
packages:

```ts
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@earendil-works/pi-ai";
import { Box, Text } from "@earendil-works/pi-tui";
```

micro writes a `node_modules` tree at startup holding `@earendil-works/pi-agent-core`,
`pi-ai`, `pi-coding-agent` and `pi-tui`, along with the older `@mariozechner/*` spellings of
the same names, and points the Bun process at it. Type-only imports cost nothing, since Bun
removes them before anything is resolved.

What is behind those names is real rather than a shim. pi-tui's layout, editor,
keybindings, autocomplete and LaTeX rendering all work. pi-ai's schema helpers, retry,
streaming-JSON repair and provider registry all work, and a provider an extension registers
streams through micro's own clients. pi-coding-agent reads and migrates sessions, builds
context, and hands back micro's real read, write, edit, bash, grep, find and ls tools.

A few things have no counterpart here: pi's own agent loop and session runtime, its
interactive mode, and its terminal image protocols. Those throw a named error when called
rather than at import, so an extension that never reaches for them is unaffected.

## When something does not load

micro prints `note: <path> was not loaded: <reason>` and carries on. One broken extension
never stops the others, and never stops the session.

In `--print` mode a command whose handler throws fails the run with a non-zero exit and the
error on stderr, so a script can tell a crashed extension from a finished one.
