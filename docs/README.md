# micro documentation

micro is a terminal coding agent. It can run interactively, print one response for a script, or expose a JSONL RPC interface. Sessions are stored locally as append-only logs.

![micro running an interactive coding session in a terminal](https://raw.githubusercontent.com/rmonvfer/micro/main/assets/micro-intro.png)

If this is your first time using micro, follow [Getting started](getting-started.md). The [CLI reference](cli-reference.md) is the quickest way to look up a command or flag.

## Start here

- [Getting started](getting-started.md): install micro, authenticate, run a first prompt, and resume a session.
- [CLI reference](cli-reference.md): command-line options, subcommands, and interactive slash commands.

## Use micro

- [Sessions](sessions.md): saved conversations, billing, budgets, request inspection, and cache-miss analysis.
- [Tools and integrations](tools.md): built-in tools, workspace boundaries, tool selection, and deferred tool search.
- [MCP servers](mcp.md): `mcp.json`, HTTP servers, OAuth sign-in, exposure, and `micro mcp`.
- [Codemode](codemode.md): let the model write scripts that call tools and models side by side.
- [Project context](project-context.md): instruction files, skills, system prompts, and prompt templates.
- [Providers and models](providers.md): subscription and API-key sign-in, model selection, live listings, and custom endpoints.
- [Image and classifier models](models.md): image generation and classification for extensions and scripts.
- [Local models with llama.cpp](llama-cpp.md): connect a llama.cpp router and manage its models.
- [Remote control](remote-control.md): pair a phone and publish an active terminal session.

## Configure and extend it

- [Configuration](configuration.md): settings, paths, models, trust defaults, and MCP servers.
- [Extensions](extensions.md): write, install, and permission TypeScript extensions.
- [Virtual models](virtual-models.md): route each request to a physical model from an extension.
- [Testing extensions](extension-testing.md): compatibility and terminal test harnesses.

## Security and internals

- [Security model](security.md): project trust, command confinement, and extension capabilities.
- [Command sandbox](sandbox.md): policies, platform support, and refusal behavior.
- [RPC mode](rpc.md): control micro from another process over newline-delimited JSON.
- [Ledger format](ledger.md): the JSONL schema and the events recorded for each session.
- [Architecture](architecture.md): crate boundaries, request flow, persistence, and streaming.
