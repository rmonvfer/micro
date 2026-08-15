# micro

micro is a terminal coding agent with local, inspectable session logs.

It supports multiple model providers, runs commands in an operating-system sandbox, and records requests, usage, tool calls, and policy decisions as the session runs.

```bash
cargo install --path crates/micro-cli
micro auth login anthropic
micro "explain this repository"
```

Read the [documentation](https://rmonvfer.github.io/micro/) for installation, configuration, security details, and extension development.

Run it directly from a checkout with:

```bash
cargo run --bin micro -- "explain this repository"
```

## What it does

- Opens an interactive terminal interface or runs a single prompt with `--print`.
- Works with Anthropic, OpenAI, Google, OpenRouter, GitHub Copilot, and other compatible providers.
- Saves conversations as append-only JSONL logs on your machine.
- Reports provider usage and estimated cost by turn with `micro bill`.
- Explains prompt-cache misses with `micro why-miss`.
- Uses Seatbelt on macOS and Landlock with seccomp on Linux to restrict commands.
- Loads TypeScript extensions in a confined Bun process with explicit host capabilities.

The core agent is a native Rust binary. Bun is only needed for TypeScript extensions.

## Basic use

Start an interactive session:

```bash
micro
```

Start with a prompt:

```bash
micro "find the cause of the failing test"
```

Run once and print the final response to standard output:

```bash
micro --print "summarize the error handling in src/"
micro -p -q "list the public functions in micro-agent" > api.txt
```

Select a model with `-m`:

```bash
micro -m opus "review this patch"
micro models sonnet
```

Resume the latest session for the current workspace:

```bash
micro --continue
```

## Session records

micro writes each session while it is running. The log contains the conversation and a versioned ledger of requests, provider usage, sandbox refusals, extension capability checks, and other runtime events.

```bash
micro sessions list
micro sessions show <SESSION_ID> --turn 4
micro sessions show <SESSION_ID> --turn 4 --raw
micro sessions export <SESSION_ID>
micro bill <SESSION_ID>
micro why-miss <SESSION_ID> 4
```

New sessions retain the serialized provider request as a content-addressed blob. `--raw` verifies that body against the recorded hash before printing it. Older sessions are reconstructed and printed only when reconstruction produces the same hash.

Inside the TUI, `/bill`, `/why-miss [turn]`, and `/request <turn> [--raw]` open local inspection views. In `/bill`, select a model turn and press Enter for its prompt-source and usage breakdown. These views do not add messages to the conversation.

## Command sandbox

The default policy is `workspace-write`: commands may write inside the workspace, cannot write to `.git` or `.micro`, and cannot use the network.

```bash
micro --sandbox read-only
micro --sandbox workspace-write
micro --sandbox full
micro sandbox try -- touch ../outside.txt
```

See [Security model](docs/security.md) and [Command sandbox](docs/sandbox.md) before using `full` or trusting project-provided configuration.

## Extension host

TypeScript extensions run in a Bun process with an empty inherited environment, no network or write access, and a filesystem read allowlist limited to the host and loaded extension packages. Host API calls still pass through the capability broker, and brokered command execution still uses the session sandbox. On a platform where micro cannot enforce the host sandbox, extensions do not run.

## Documentation

Read the [documentation site](https://rmonvfer.github.io/micro/) or browse the Markdown in [`docs/`](docs/README.md).

- [Getting started](docs/getting-started.md)
- [CLI reference](docs/cli-reference.md)
- [Sessions, billing, and cache analysis](docs/sessions.md)
- [Tools and integrations](docs/tools.md)
- [Project context](docs/project-context.md)
- [Providers and models](docs/providers.md)
- [Configuration](docs/configuration.md)
- [Security model](docs/security.md)
- [Extensions](docs/extensions.md)
- [Remote control](docs/remote-control.md)
- [RPC mode](docs/rpc.md)
- [Ledger format](docs/ledger.md)
- [Architecture](docs/architecture.md)

micro does not collect telemetry or upload session logs. Model requests go to the provider selected for the session. Remote-control traffic goes through the configured relay as encrypted payloads.
