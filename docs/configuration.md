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
├── mcp.json                     MCP servers
├── mcp-auth.json                MCP server sign-ins
├── llama-cpp.json               llama.cpp router connection
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
├── mcp.log                      MCP server logs
├── llama-cpp-context.json       llama.cpp context windows
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

| Variable         | Setting    |
| ---------------- | ---------- |
| `MICRO_MODEL`    | `model`    |
| `MICRO_PROVIDER` | `provider` |
| `MICRO_THINKING` | `thinking` |
| `MICRO_THEME`    | `theme`    |

Command-line options take precedence over environment variables, which take precedence over `config.json`.

## Settings reference

`config.json` accepts these keys. `/settings` shows the effective value and source.

| Key | Default | Meaning |
| --- | --- | --- |
| `model` | unset | Model query resolved at startup. |
| `provider` | unset | Provider used when the model query does not choose one. |
| `thinking` | `off` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, or `max`. |
| `theme` | `system` | `system`, `dark`, `light`, a user theme's name, or a `light/dark` pair. |
| `tui_mode` | `fullscreen` | `regular` or `fullscreen`. |
| `live_models` | `false` | Refresh provider model listings at startup before selecting a model. |
| `auto_compact` | `true` | Compact when context approaches the model limit. |
| `auto_update` | `true` | Check managed installations for updates. |
| `update_check_interval_hours` | `24` | Minimum hours between automatic update checks. |
| `hide_thinking` | `true` | Hide reasoning content in the transcript. |
| `show_images` | `true` | Render supported images in capable terminals. |
| `image_width_cells` | `60` | Maximum rendered image width in terminal cells. |
| `auto_resize_images` | `true` | Shrink images to the available terminal width. |
| `block_images` | `false` | Reject image attachments. |
| `skill_commands` | `true` | Advertise discovered skills to the model. |
| `content_padding` | `1` | Horizontal padding around prompt and lower UI content. |
| `output_pad` | `1` | Columns of padding beside the transcript's text, `0` or `1`. Message bands always span the full width. |
| `interface_padding` | `0` | Padding between the interface and terminal edges. |
| `steering_mode` | `one-at-a-time` | `one-at-a-time` or `all` for queued steering messages. |
| `tree_filter_mode` | `default` | `default`, `no-tools`, `user-only`, `labeled-only`, or `all`. |
| `fullscreen_exit_output` | `transcript` | `transcript` or `resume-hint`. |
| `fullscreen_scrollbar` | `auto` | `auto`, `always`, or `hidden`. |
| `clear_on_shrink` | `false` | Clear stale terminal rows when the interface becomes shorter. |
| `mermaid` | `streaming` | `off`, `final`, or `streaming`. |
| `autocomplete_max_items` | `5` | Maximum command-completion rows. |
| `show_hardware_cursor` | `false` | Keep the terminal's hardware cursor visible. |
| `terminal_progress` | `true` | Show progress while a turn runs. |
| `quiet_startup` | `false` | `true` hides the startup introduction; `"header"` keeps the mark with the version and key hints and the note that micro can explain itself, and hides the loaded resources. |
| `collapse_changelog` | `false` | Collapse changelog display. |
| `warnings` | `true` | Show runtime warnings. |
| `cache_miss_notices` | `false` | Report cache writes that did not record a cache read, and each prompt-cache refresh with its cost. |
| `double_escape` | `tree` | `tree`, `fork`, or `none` when Escape is pressed twice on an empty prompt. |
| `follow_up_mode` | `queue` | `queue` or `interrupt` for input submitted during a turn. |
| `default_project_trust` | `ask` | `ask`, `always`, or `never`. |
| `http_idle_timeout` | `120` | Seconds without provider output before a request fails. |
| `http_proxy` | unset | Proxy URL applied as `HTTP_PROXY` and `HTTPS_PROXY` to micro's HTTP clients. |
| `scoped_models` | `[]` | Model queries allowed in the workspace. Empty permits the full catalog. |
| `tool_search_threshold` | `15` | Number of extension tools declared directly before they move behind `tool_search`. |
| `default_tools` | unset | Built-in tools a session starts with; `+name` and `-name` adjust the defaults. See [Tools](tools.md#select-tools). |
| `compaction` | unset | Compaction budgets in tokens, with per-model overrides. |
| `image_limits` | 2000×2000, 4.5 MiB | Size limits for images sent to a model, with per-model overrides. |
| `cache_warming` | `streaming` | `off`, `streaming`, or `idle`: when to keep a provider's prompt cache from expiring. |
| `prompt_cache_lifetimes` | `{}` | Prompt cache lifetimes in seconds, keyed by `provider/model` or provider. |
| `anthropic_extra_usage` | `true` | Warn about per-token use of Anthropic subscription credentials in a third-party client. |
| `transport` | `sse` | `sse` or `auto` for the ChatGPT Codex backend. |
| `sandbox` | unset | Command policy; runtime default is `workspace-write`. |
| `budget` | `0` | Session cost limit in USD. Zero disables it. |
| `extensions` | `[]` | Additional extension paths or package sources. |
| `codemode.mode` | `on` | `on` keeps declared tools declared while `codemode` is offered; `only` lists them in the `codemode` description instead. |
| `codemode.inline_budget` | `3000` | Estimated tokens the tool declarations in the `codemode` description may use. |
| `copy_on_select` | `true` | Copy text to the clipboard as soon as the mouse selects it. When off, `ctrl+x` copies the selection. |
| `half_page_scroll` | `false` | Move the transcript half a page at a time with Page Up and Page Down. |
| `external_editor` | unset | Command `ctrl+g` opens the prompt in, ahead of `$VISUAL` and `$EDITOR`, read the way a shell reads it, such as `"code --wait"`. |
| `terminal_hyperlinks` | `"auto"` | `true`, `false`, or `"auto"` to override OSC 8 hyperlink detection. |
| `terminal_images` | `"auto"` | `"kitty"`, `"iterm2"`, `false`, or `"auto"` to override inline image protocol detection. |
| `terminal_true_color` | `"auto"` | `true`, `false`, or `"auto"` to override 24-bit color detection. |

Unknown keys are preserved when micro rewrites the file but have no effect in a version that does not recognize them.

## Common settings

### model and provider

`model` is resolved the same way as `--model`: exact ID, qualified ID, alias, or unique partial match. `provider` is used when the model query does not select one.

`/model` and `/thinking` change only the current session. `ctrl+s` in their pickers, or `--default` before the argument, also writes the choice to `config.json`: `model` and `provider` for a model, `thinking` for a level.

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

Public release checks do not require a token. For a private repository or authenticated API access, micro reads `MICRO_GITHUB_TOKEN`, `GITHUB_TOKEN`, or `GH_TOKEN`. If none is set, it reuses the current `gh auth` token. The credential must be able to read the repository's releases.

The installer also reads `MICRO_REPOSITORY`, `MICRO_VERSION`, `MICRO_INSTALL_DIR`, and `MICRO_DIST_DIR`. They select the GitHub repository, release tag, executable link directory, and versioned distribution directory. `MICRO_GITHUB_TOKEN`, `GITHUB_TOKEN`, and `GH_TOKEN` provide release-download authentication.

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

When extensions add more tools than this threshold, micro exposes them through `tool_search` instead of sending every tool definition on every request. The default is `15`. Set it to `0` to declare every extension tool directly. How MCP tools are reached is decided by each server's [`exposure`](mcp.md#control-exposure) instead.

### codemode

`codemode.mode` and `codemode.inline_budget` shape the [`codemode`](codemode.md) tool's description. With `only`, the tools micro would declare are left out of requests and listed for scripts instead.

```json
{ "codemode": { "mode": "only", "inline_budget": 3000 } }
```

### Themes

The `system` theme, the default, builds micro's colors from the terminal's own. micro asks the terminal for its default foreground and background and its sixteen ANSI colors, takes each color's hue from one ANSI color (errors from red, links from blue), and sets its lightness so it stands out from the background; pastel palettes stay pastel. Body text keeps at least a 4.5:1 contrast ratio, and uses the terminal's own foreground when that is strong enough. When the terminal reports only its background, micro uses its own hues; when it reports nothing, the built-in `dark` or `light` theme is used. The terminal is asked again whenever it regains focus, so switching it between light and dark rebuilds the theme.

`dark` and `light` select a built-in theme, and a pair such as `"solarized-light/solarized-dark"` picks one side by the terminal's background. Any other name loads `themes/<name>.json` from the configuration directory. A theme file has a `name`, which cannot be `system` or contain `/`, a `colors` object with every token, optional `vars` that colors can name, and an optional `appearance` of `"dark"` or `"light"` saying which background it was made for; without it, micro infers the appearance from the `text` color. A color is a hex value (`"#0af"` or `"#00aaff"`), `"oklch(62% 0.1 200)"`, `"okhsl(250 60% 55%)"`, a 256-color index such as `39`, a variable name, or `""` for the terminal's default color.

```json
{
  "name": "harbor",
  "appearance": "dark",
  "vars": { "sea": "okhsl(220 70% 65%)" },
  "colors": { "accent": "sea", "text": "", "border": "#456" }
}
```

`/theme` switches between `system`, `dark`, `light`, and `auto`, which follows the terminal's background with the built-in themes, for the rest of the session.

### Terminal capabilities

micro detects whether the terminal takes OSC 8 hyperlinks, which inline image protocol it speaks, and whether it shows 24-bit color. A multiplexer or a remote session can hide what the terminal underneath supports, so `terminal_hyperlinks`, `terminal_images`, and `terminal_true_color` replace detection with a fixed answer. Force a capability only when every hop to the terminal supports it; unsupported escape sequences corrupt the screen. Without 24-bit color, every theme color is drawn with the nearest entry of the 256-color palette.

```json
{ "terminal_hyperlinks": true, "terminal_images": false }
```

Where hyperlinks work, the paths in `read`, `write`, `edit`, `multi_edit`, and `ls` titles open the file they name.

### cache_miss_notices

When enabled, micro reports turns that write a prompt cache without reading from it, and each refresh that kept a prompt cache warm, with its cost. Use `micro why-miss` for a local prefix and conversation diagnostic after the run.

### compaction

By default automatic compaction fires when the conversation passes 80% of the model's context window and keeps the most recent 30% verbatim. `reserve_tokens` replaces the trigger with an absolute reserve: compaction fires once the conversation needs more than the context window less this many tokens. `keep_recent_tokens` sets how many tokens of recent conversation survive automatic and manual compaction.

`model_overrides` tunes the budgets for particular models, keyed by exact `provider/model`. Each value falls back on its own from the model override to the ordinary value to the default share, and values must be non-negative integers. The budgets in force follow the current model, so switching models changes the next check without changing anything already compacted.

```json
{
  "compaction": {
    "reserve_tokens": 16384,
    "keep_recent_tokens": 20000,
    "model_overrides": {
      "some-provider/big-model": { "reserve_tokens": 400000 }
    }
  }
}
```

For a model with a one-million-token window, this override fires compaction above 600,000 tokens and keeps the ordinary 20,000 recent tokens.

### image_limits

Images a user attaches, images `read` returns, and images other tools return are fitted to the current model's limits as they join the conversation. An image within the limits is sent byte for byte; a larger one is scaled down, keeping its proportions and EXIF orientation, and re-encoded as whichever of PNG and JPEG is smaller, then shrunk further until its base64 fits `max_bytes`. A resized image is followed by a note giving its original size so the model can map coordinates back. An image that cannot be made small enough is replaced by a note.

Each image is fitted once and stored fitted in the session, so switching models never rewrites history and a cached prompt prefix stays valid. `max_bytes` limits the base64 payload. Omitted limits default to 2000 by 2000 pixels, 4.5 MiB, and JPEG quality 80. `model_overrides` sets limits for particular models, each falling back to the ordinary value.

```json
{
  "image_limits": {
    "model_overrides": {
      "acme/vision-model": { "max_width": 1568, "max_height": 1568, "max_bytes": 524288, "jpeg_quality": 75 }
    }
  }
}
```

### cache_warming

A provider's prompt cache expires a few minutes after its last use, so a tool that runs longer than that makes the next request pay to write the whole prompt again. With `streaming`, micro replays the last request with a one-token output cap shortly before the cache would expire, for as long as the run is going. `idle` also keeps warming between runs, for up to 30 minutes; `off` never warms.

A refresh goes out at 90% of the cache lifetime, leaving at least ten seconds of margin, and only when it is expected to save at least $0.05: the extra cost of a cache miss, weighted by the chance that another request comes before expiry (certain while a run is going, 15% while idle), less the cost of the refresh. Warming stops after an hour, when a new request is sent, when the conversation is compacted or replaced, or when the model changes. Anthropic requests with extended thinking are never replayed, because the output cap changes the thinking budget their cache is keyed on.

A model is eligible only when its cache lifetime is known. Anthropic's five-minute lifetime is built in; `prompt_cache_lifetimes` declares others, keyed by exact `provider/model` or by provider. Each refresh is recorded in the session ledger as `cache_warm` and counts toward the session bill, but never enters the conversation; with `cache_miss_notices` on, the transcript shows each one with its cost. `/session` shows the mode, the next decision with its economics, or why nothing is being warmed, and extensions can override each decision with [`cache_warming_decision`](extensions.md#events).

```json
{ "cache_warming": "idle", "prompt_cache_lifetimes": { "openai": 300 } }
```

### http_proxy

Routes micro's own HTTP traffic (provider requests, sign-in, model listings, sharing, and updates) through one proxy. At startup micro sets `HTTP_PROXY` and `HTTPS_PROXY` to this URL, except for a scheme whose variable the environment already sets in either case, so a proxy exported in the shell still wins. Commands the model runs inherit the same variables. The setting is read only from `config.json`, never from a project.

```json
{ "http_proxy": "http://proxy.internal:3128" }
```

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

MCP servers are configured in `mcp.json` beside `config.json`, and in a trusted project's `.micro/mcp.json`, using the `mcpServers` shape other MCP clients share:

```json
{
  "mcpServers": {
    "notes": {
      "command": "/usr/local/bin/notes-mcp",
      "args": ["--stdio"],
      "env": { "NOTES_HOME": "/srv/notes" }
    }
  }
}
```

See [MCP servers](mcp.md) for HTTP servers, OAuth, exposure, and the `micro mcp` commands.

## Project configuration

A trusted project may provide `.micro/settings.json`, `.micro/mcp.json`, extensions, skills, prompts, themes, `SYSTEM.md`, and `APPEND_SYSTEM.md`. Project `settings.json` accepts `sandbox` and `default_tools`; other user settings remain controlled by `config.json`, environment variables, and command-line options.

`--approve` and `--no-approve` override trust for one run. `/trust on` and `/trust off` save a decision for later runs.

See [Project context](project-context.md) for instruction discovery, system prompts, skills, and prompt templates.

## Sessions

The data directory contains one JSONL log, one metadata sidecar, and a blob directory per session. See [Sessions](sessions.md) for CLI operations and [Ledger format](ledger.md) for the schema.
