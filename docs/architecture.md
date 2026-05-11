# Architecture

micro is a Rust workspace split by runtime responsibility. Provider translation, agent control flow, tools, persistence, extensions, and the terminal interface are separate crates.

## Main request path

```text
micro-cli
  -> micro-agent
    -> micro-provider
      -> model API
```

`micro-cli` resolves the workspace, model, credential, settings, trust decision, sandbox policy, tools, and session. It then builds the runtime used by interactive, print, or RPC mode.

`micro-agent` owns the model/tool loop. It builds a provider-neutral `Context`, records request metadata, streams a response, executes requested tools, appends tool results, and repeats until the model returns a response with no more tool calls.

`micro-provider` translates the shared context into the selected provider protocol and translates the streamed response back into `StreamEvent` values.

## Core crates

| Crate | Responsibility |
| --- | --- |
| `micro-types` | Messages, content blocks, usage, stream events, agent events, and ledger events. |
| `micro-dirs` | Configuration and data-path resolution. |
| `micro-config` | Settings, project trust, and persisted configuration. |
| `micro-auth` | Stored credentials and environment fallback. |
| `micro-models` | Bundled, user, and live model catalogs. |
| `micro-context` | System prompt, project instructions, skills, and compaction. |
| `micro-provider` | Provider request formats and streaming parsers. |
| `micro-tools` | Model-callable tools. |
| `micro-sandbox` | Command wrapping and file-policy checks. |
| `micro-agent` | Provider-neutral agent loop. |
| `micro-session` | Append-only conversations, ledger events, metadata, and blobs. |
| `micro-commands` | Slash-command parsing and outcomes. |
| `micro-extensions` | Bun extension host and capability broker. |
| `micro-mcp` | MCP server processes and tools. |
| `micro-skills` | Skill discovery and loading. |
| `micro-prompts` | Prompt-template discovery and argument expansion. |
| `micro-mermaid` | Mermaid diagram rendering requested by a response. |
| `micro-remote` | Phone pairing, encryption, relay protocol, and session mirroring. |
| `micro-rpc` | JSON-line control protocol. |
| `micro-tui` | Terminal rendering and input. |
| `micro-cli` | Startup, subcommands, runtime assembly, and print mode. |
| `micro-testkit` | Scripted providers, fake tools, and session fixtures. |

Lower-level crates do not depend on the UI or CLI.

## Provider boundary

Providers receive a `Model`, a `Context`, and a credential. `Context` contains the system prompt, conversation messages, and JSON-schema tool definitions. It describes tools but cannot execute them.

That boundary keeps provider differences out of the agent loop. Anthropic Messages, OpenAI-compatible completions, OpenAI Responses, Google APIs, Vertex, and Bedrock each implement request assembly and stream parsing. The agent consumes one event vocabulary.

The provider also exposes request-body assembly without sending it. micro uses the same path to hash requests before sending and to reconstruct them later from the ledger.

## Agent loop

For each turn, the agent:

1. builds the current context;
2. applies context and extension hooks;
3. records `turn_request` data;
4. asks the provider to stream a response;
5. emits `AgentEvent` values for the caller;
6. records usage and the completed assistant message;
7. executes valid tool calls;
8. appends tool results and starts another provider turn when needed.

A response cut off by the output-token limit does not execute incomplete tool calls. Tool errors are returned to the model as error results so the turn can recover.

Transient provider failures may be retried before any output is shown. Once content has streamed to the user, micro does not issue the same request again automatically.

## Streaming

`StreamEvent` carries deltas rather than rebuilt partial messages. Consumers that need the response-so-far use `PartialResponse` to accumulate them.

Each provider stream ends with one `Done` or `Error` event. The agent layer similarly ends with `AgentEnd`.

This lets the TUI, print renderer, RPC server, and tests consume the same loop.

## Persistence

The agent sends messages, compaction records, and ledger events through one recorder channel. A separate task appends them to the session file in receive order.

Writing during the run limits data loss if the process exits unexpectedly. The agent does not wait for individual filesystem writes, but shutdown waits for the recorder channel to drain.

Compaction adds a summary and moves the active conversation head. Original messages remain in the log.

See [Ledger format](ledger.md).

## Project startup

The CLI decides project trust before loading `.micro/` resources. It then resolves the sandbox and builds tools around that policy.

This keeps untrusted extensions, settings, prompts, and skills out of the runtime rather than adding checks at each later use.

Extension capabilities are enforced separately at the host boundary. Commands requested through the host are passed through the same sandbox policy as commands requested by the model.

## Tests

`micro-testkit` provides a scripted provider and deterministic tools for agent-loop tests. CLI tests run the built binary against scratch configuration and workspaces. Terminal behavior is tested under a pseudo-terminal and, when necessary, by replaying escape sequences into a screen grid.

See [Testing extensions](extension-testing.md) for the extension-specific harnesses.
