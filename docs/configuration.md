# Configuration

micro reads user configuration, optional project configuration, environment variables, and command-line overrides. Missing files use defaults.

## Paths

`MICRO_DIR` puts all configuration and generated data under one directory:

```bash
MICRO_DIR=/tmp/micro-test micro
```

Without `MICRO_DIR`, an existing `~/.micro` remains in use. New installations use the XDG layout:

```text
~/.config/micro/                 user configuration
├── auth.json                    stored provider credentials
├── models.json                  model and provider overrides
├── trust.json                   saved project trust decisions
├── capabilities.json            saved extension capability decisions
├── config.json                  settings
├── SYSTEM.md                    replacement system prompt
├── APPEND_SYSTEM.md             text appended to the system prompt
├── themes/
├── prompts/
└── skills/

~/.local/share/micro/            generated data
├── sessions/
├── npm/
├── git/
├── extensions/
└── remote-control.json
```

`$XDG_CONFIG_HOME` and `$XDG_DATA_HOME` replace `~/.config` and `~/.local/share` when set.

## config.json

Every key is optional. A small configuration may look like:

```json
{
  "model": "opus",
  "provider": "anthropic",
  "thinking": "medium",
  "theme": "dark",
  "live_models": true,
  "sandbox": "workspace-write",
  "budget": 5.0
}
```

Use `/settings` to see active values and their sources. Use `/set` in an interactive session to inspect or change one.

For a one-run override:

```bash
micro -c theme=dracula -c show_images=false
```

The value is parsed as JSON when possible. Unquoted text is stored as a string. A key may use dotted notation.

The main environment overrides are:

| Variable            | Setting       |
| ------------------- | ------------- |
| `MICRO_MODEL`       | `model`       |
| `MICRO_PROVIDER`    | `provider`    |
| `MICRO_THINKING`    | `thinking`    |
| `MICRO_THEME`       | `theme`       |
| `MICRO_LIVE_MODELS` | `live_models` |

Command-line options take precedence over environment variables, which take precedence over `config.json`.

## Common settings

### model and provider

`model` is resolved the same way as `--model`: exact ID, qualified ID, alias, or unique partial match. `provider` is used when the model query does not select one.

### thinking

Sets the default reasoning effort. Supported values include `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`; the selected model may support only part of that range.

### updates

Release-installer installations check GitHub for a newer release before an interactive session. The default is enabled once every 24 hours. Source, package-manager, and manually copied binaries are never replaced automatically.

```json
{
  "auto_update": true,
  "update_check_interval_hours": 24
}
```

Set `auto_update` to `false` to disable checks, or use `MICRO_NO_AUTO_UPDATE=1` for one run. `micro update` checks and installs a release immediately.

Private release checks read `MICRO_GITHUB_TOKEN`, `GITHUB_TOKEN`, or `GH_TOKEN`. If none is set, micro reuses the current `gh auth` token. The credential must be able to read releases from the repository.

### sandbox

The default is:

```json
{ "sandbox": "workspace-write" }
```

A detailed policy may add writable roots or network access:

```json
{
  "sandbox": {
    "mode": "workspace-write",
    "writable_roots": ["/srv/cache"],
    "allow_network": true
  }
}
```

Project settings may also select a sandbox after the project is trusted. `--sandbox` takes precedence. See [Command sandbox](sandbox.md).

### default_project_trust

Controls what happens when a project has `.micro/` resources and no saved decision:

- `ask`: prompt in interactive mode;
- `always`: load project resources;
- `never`: ignore project resources.

The default is `ask`. See [Security model](security.md).

### budget

Sets a per-session cost ceiling in US dollars. `0` disables the ceiling. The total includes earlier runs resumed under the same session ID.

### tool_search_threshold

When MCP servers add more tools than this threshold, micro exposes them through `tool_search` instead of sending every tool definition on every request. The default is `15`. Set it to `0` to describe every tool directly.

### cache_miss_notices

When enabled, micro reports turns that write a prompt cache without reading from it. Use `micro why-miss` for a detailed comparison after the run.

## auth.json

`micro auth login` writes stored credentials here. Use the CLI rather than editing it manually:

```bash
micro auth login <PROVIDER>
micro auth logout <PROVIDER>
micro auth status
```

The file is created with user-only permissions. A stored credential takes precedence over the provider's environment variable.

## models.json

This file adds models or changes entries from the bundled catalog.

```json
{
  "providers": {
    "ollama": {
      "base_url": "http://localhost:11434/v1",
      "api": "openai-completions",
      "models": [
        {
          "id": "qwen3-coder:30b",
          "name": "Qwen3 Coder 30B",
          "aliases": ["local"]
        }
      ]
    }
  }
}
```

After saving the file:

```bash
micro -m local "explain this crate"
```

Provider-level fields apply to models below them. A model entry may set `id`, `name`, `api`, `base_url`, `context_window`, `max_output_tokens`, `reasoning`, `input`, `headers`, `aliases`, and `cost`. `cost` may contain `input`, `output`, `cache_read`, and `cache_write` prices per million tokens.

See [Providers and models](providers.md).

## MCP servers

Configure MCP servers under `mcp_servers`:

```json
{
  "mcp_servers": {
    "notes": {
      "command": "/usr/local/bin/notes-mcp",
      "args": ["--stdio"],
      "env": { "NOTES_HOME": "/srv/notes" },
      "startup_timeout": 20,
      "tool_timeout": 120
    }
  }
}
```

`command` is required. `args`, `env`, `cwd`, `enabled`, `startup_timeout`, and `tool_timeout` are optional. Environment entries are added to the inherited process environment.

A server that fails to start is reported and skipped. Other tools remain available.

MCP servers are configured programs and are not launched inside the command sandbox.

## Project configuration

A trusted project may provide `.micro/settings.json`, extensions, skills, prompts, themes, `SYSTEM.md`, and `APPEND_SYSTEM.md`.

`--approve` and `--no-approve` override trust for one run. `/trust on` and `/trust off` save a decision for later runs.

See [Project context](project-context.md) for instruction discovery, system prompts, skills, and prompt templates.

## Sessions

The data directory contains one JSONL log, one metadata sidecar, and a blob directory per session. See [Sessions](sessions.md) for CLI operations and [Ledger format](ledger.md) for the schema.
