# micro

micro is a terminal coding agent with local, inspectable session logs.

It supports multiple model providers, runs commands in an operating-system sandbox, and records requests, usage, tool calls, and policy decisions as the session runs.

## Installation

The release installer supports macOS on Apple Silicon and Linux on x86-64 or ARM64. Linux release binaries require glibc 2.35 or later; musl builds are not provided. Windows is not supported. Install the latest public release with:

```bash
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  https://raw.githubusercontent.com/rmonvfer/micro/main/scripts/install.sh | bash
```

If GitHub requires authentication for the repository, use:

```bash
gh auth login
gh api -H "Accept: application/vnd.github.raw+json" repos/rmonvfer/micro/contents/scripts/install.sh | bash
```

The installer verifies the release checksum, installs versioned distributions under `~/.local/share/micro/dist`, and links `micro` from `~/.local/bin`. It prints the required shell configuration when that directory is not already on `PATH`. Set `MICRO_INSTALL_DIR` or `MICRO_DIST_DIR` to override those locations, and set `MICRO_VERSION` to a release tag to install a specific version.

Managed installations check for updates once every 24 hours when starting an interactive session. Run `micro update` to update immediately, set `auto_update` to `false` in the configuration, or set `MICRO_NO_AUTO_UPDATE=1` for one launch.

To install from a source checkout instead:

```bash
cargo install --path crates/micro-cli
```

Source installations are not managed by the release updater. Run the checkout without installing it with:

```bash
cargo run --bin micro -- "explain this repository"
```

After installation, connect a provider and start a session:

```bash
micro auth login anthropic
micro "explain this repository"
```

## What it does

- Opens an interactive terminal interface or runs a single prompt with `--print`.
- Works with Anthropic, OpenAI, Google, OpenRouter, GitHub Copilot, and other compatible providers.
- Saves conversations as append-only JSONL logs on your machine.
- Reports provider usage and estimated cost by turn with `micro bill`.
- Provides a local prompt-prefix diagnostic for cache misses with `micro why-miss`.
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

Sessions enqueue the serialized provider request for storage as a content-addressed blob. `--raw` verifies a retained body against the recorded hash before printing it. A process crash can lose queued records that have not reached disk. Sessions without a retained body are reconstructed and printed only when reconstruction produces the same hash.

Inside the TUI, `/bill`, `/why-miss [turn]`, and `/request <turn> [--raw]` open local inspection views. In `/bill`, select a model turn and press Enter for its prompt-source and usage breakdown. These views do not add messages to the conversation.

## Command sandbox

The default policy is `workspace-write`: commands may write inside the workspace and cannot use the network. Built-in file tools keep `.git` and `.micro` read-only. macOS applies the same protected-path rule to shell commands; Linux confines shell writes to the workspace but cannot exclude those descendants from a writable workspace.

```bash
micro --sandbox read-only
micro --sandbox workspace-write
micro --sandbox full
micro sandbox try -- touch ../outside.txt
```

See [Security model](docs/security.md) and [Command sandbox](docs/sandbox.md) before using `full` or trusting project-provided configuration.

## Extension host

TypeScript extensions run in a Bun process with an empty inherited environment, no network access, and read-only access to the workspace and loaded packages. Host API calls pass through the capability broker, and brokered command execution still uses the session sandbox. On a platform where micro cannot enforce the host sandbox, extensions do not run.

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

See [Contributing](CONTRIBUTING.md) for development setup and required checks.

micro does not collect telemetry or upload session logs automatically. Model requests go to the provider selected for the session, and `/share` explicitly uploads a transcript as a secret GitHub gist. Remote pairing transfers its secret through a QR code, and remote-control payloads are encrypted before they pass through the configured relay; see the [remote-control threat model](docs/remote-control.md#encryption-and-relay) before using a custom relay.

## License

micro is licensed under the [MIT License](LICENSE). The `micro-sandbox` crate contains code derived from OpenAI Codex and is licensed under Apache-2.0; see its [license](crates/micro-sandbox/LICENSE) and [notice](crates/micro-sandbox/NOTICE).
