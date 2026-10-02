# Changelog

## [Unreleased]

- Offer only the thinking levels the current model supports, as its catalog entry records them: `/thinking` lists and accepts only those, `shift+tab` cycles through them and says when the model does not reason, the footer leaves thinking out for such models, and a level the model lacks (from settings, `--thinking`, an extension, a virtual model's router, or a model switch) becomes the nearest supported one. Anthropic, Codex and Gemini requests send `xhigh`, `max` and other levels as the model names them, the catalog takes each model's levels from models.dev and OpenRouter, and RPC adds `get_available_thinking_levels` and spells the level `xhigh`.

## [0.2.1] - 2026-10-02

### Fixed

- Update rustls to 0.23.45 for RUSTSEC-2026-0285, which accepted TLS 1.3 handshake messages across encryption level boundaries.

## [0.2.0] - 2026-10-02

### MCP

- Configure MCP servers in `mcp.json`, globally and in trusted projects, instead of `mcp_servers` in settings. Entries use the `mcpServers` shape shared with other MCP clients and may carry a `description`.
- Connect to MCP servers over streamable HTTP, with `url` and `headers`, alongside stdio.
- Sign in to MCP servers with OAuth: discovery, dynamic client registration with `oauth.clientName`, PKCE with a loopback callback and a clickable sign-in link, `oauth.authServerMetadataUrl`, RFC 9207 issuer checks, refresh, and step-up sign-in that keeps granted scopes. Credentials are stored per server name and URL, readable only by their owner.
- Send a provider's micro credential to an HTTP MCP server with `"auth": { "provider": "<name>" }`, allowed in the global `mcp.json` and from extensions you install, never from a project.
- Add `micro mcp add|remove|list|login|logout` and the `/mcp` command.
- Make `codemode` the default MCP exposure: servers connect in the background and their tools are called from scripts, which search for them. `codemode-deferred` is another name for it, `deferred` servers are found by `tool_search`, and `hidden` leaves a server unconnected.
- Expose single MCP tools apart from their server with `toolExposure`, keyed by tool name or `*` pattern.
- Connect MCP servers side by side; `tool_search` and scripts wait for servers still connecting in the background.
- List servers whose tools are not declared in an `mcp_servers` system prompt section, checked at the start of each prompt; a changed section is added to the conversation instead of rewriting the system prompt, so the cached prefix survives.
- Keep deferred MCP tools that `tool_search` found callable after resuming a session: a call waits for its server to reconnect. `default_tools` leaves tools from servers connecting in the background alone.
- Let extensions add MCP servers with `micro.registerMcpServer()`, remove them with `unregisterMcpServer()`, and list them with `getMcpServers()`, under the `mcp_servers` capability. Registered servers may use `auth.provider`, which project `mcp.json` files may not.
- Name MCP tools with `-` written as `_` (`mcp__my_server__x`), give colliding tool names a hash suffix, reject server names that differ only in `-` and `_`, and title calls `server/tool`.
- Retry connecting to an HTTP MCP server twice after a network failure or a transient status (408, 429, 5xx), and resume an answer stream that breaks off from its last event.
- Append MCP servers' log messages and stdio servers' standard error to `mcp.log` in the data directory, rotated at 5 MB.

### Codemode and tools

- Add the `codemode` tool: the model writes JavaScript that runs in an embedded QuickJS sandbox and calls the other tools, side by side with `Promise.all`, and only the script's output reaches the model. Scripts get `text()`, `image()`, `console.*`, `exit()`, `store()`/`load()` kept per branch, `ALL_TOOLS`, `searchTools()`, `describeTool()`, and `describeNamespace()`, an `// @options:` line for `max_output_tokens` and `timeout_ms`, and errors that name close matches. Offer it with `--tools`; configure it with `codemode.mode` and `codemode.inline_budget`.
- Give `codemode` scripts a `models` global: list models by type and availability, classify and generate images with the session's credentials, at most four calls at a time. Malformed calls say what was expected, each call's cost is listed in the result, and unshown generated images are pointed out.
- Give scripts `bash` output as data: `{ output, truncated, full_output_path?, exit_code, wall_time_seconds }`, up to 1 MiB with the first and last 512 KiB of longer output.
- Draw `codemode` calls with their script, the calls they make as they run, and output cut by wrapped lines while collapsed.
- Add tool exposure levels `direct`, `model-only`, `codemode`, `deferred`, and `hidden`, with tool namespaces, annotations, `outputSchema` with `structuredContent`, and `isError` results, for extension tools as well as built-in and MCP ones.
- Let tools call other tools while they run, through `ctx.executeTool()` for extensions. Nested calls go through the same hooks and checks, are reported with their parent's id, are recorded on the calling tool's result, and add their usage to it.
- Count what tools spend on models, including `codemode` model calls and nested `ctx.executeTool()` calls that report a cost, toward the session cost as `tool_cost` ledger events.
- Add the `default_tools` setting (user and trusted project) with `+name`/`-name` entries; `/reload` turns on tools newly added to it, and `--tools` still overrides it.
- Save truncated `bash` output in full to a private temporary file and name its path in the result.

### Models

- List image and classifier models beside chat models: OpenRouter's image and decision models under `openrouter`, and TypeSafe's Jev on TypeSafe, OpenRouter, Cloudflare Workers AI, Vercel AI Gateway and OpenCode. `micro models --type image|classifier` lists them; `/model` still offers only chat models.
- Generate images and classify through `ctx.modelRegistry.generateImages()` and `ctx.modelRegistry.classify()`, with credentials resolved at call time. Their usage is recorded as `model_call` ledger events and counts toward the session cost, `/session` and `micro bill`. `micro_provider::ModelRuntime` offers the same to Rust callers.
- Register virtual models with `micro.registerVirtualModel()`: a router picks a physical model and thinking level for every request, keeps state on the session branch, and the footer shows the routed model. `/session` lists cost per physical model, and turns are billed at the rates of the model that answered.
- Connect to a llama.cpp router with `micro llama connect`, and list, search Hugging Face, download, load and unload its models with `micro llama`. Loaded models are chat models under `llama.cpp` and classifiers read from next-token label probabilities; their runtime context windows are remembered between runs.
- Keep `/model` and `/thinking` choices to the current session, and save one as the default with `ctrl+s` in their pickers or `--default` before the argument.

### Sign-in

- Sign in to Anthropic with a Claude Pro/Max subscription, by browser or by pasting the code Anthropic shows when the browser runs elsewhere. Subscription requests carry Claude Code's identity.
- Sign in with ChatGPT for the `openai` provider, and sign in to `openai-codex` by browser or device code.
- Sign in to OpenRouter (pasting the redirect URL over SSH), xAI with SuperGrok or X Premium, and Kimi Code.
- Use Anthropic workload identity federation from `ANTHROPIC_FEDERATION_RULE_ID`, `ANTHROPIC_ORGANIZATION_ID` and `ANTHROPIC_IDENTITY_TOKEN_FILE`.
- Add `micro auth check`, `micro auth print-api-key` and `micro auth print-bearer-token`.
- Refresh expiring OAuth tokens under the credential file lock.
- Label `/login` and `/logout` entries as `not configured`, or as an API key, `subscription` or `account`.

### Terminal interface

- Add the `system` theme, now the default, which derives colors from the terminal's reported foreground, background, and ANSI palette, keeps pastel palettes pastel, and rebuilds when the terminal regains focus after switching between light and dark.
- Accept `#rgb`, `oklch()`, and `okhsl()` colors and an optional `appearance` field in theme files.
- Add `terminal_hyperlinks`, `terminal_images`, and `terminal_true_color` to override detected terminal capabilities, and draw themes with the 256-color palette on terminals without 24-bit color.
- Search the fullscreen transcript with `ctrl+f`: matches are highlighted, `enter` and `shift+enter` step between them, and `escape` closes the search.
- Show a clickable jump-to-latest label while the fullscreen transcript is scrolled up, return with `end`, and add `half_page_scroll` for half-page Page Up and Page Down.
- Select a word with a double click and a paragraph with a triple click, copy the active selection with `ctrl+x`, and add `copy_on_select` to turn off automatic selection copy.
- Add `external_editor` to choose the `ctrl+g` editor ahead of `$VISUAL` and `$EDITOR`, and run editor commands that carry arguments.
- Add `output_pad` to set the transcript's horizontal padding apart from `content_padding`.
- Accept `quiet_startup: "header"` to keep the startup header with the version and key hints and hide the rest.
- Link file paths in built-in file tool titles with OSC 8 `file://` hyperlinks.
- Show the arguments of tool calls without a custom renderer as `key=value` pairs when collapsed and `key: value` lines when expanded.
- Complete slash commands when the prompt starts with whitespace.
- Add `/bug` to write a ZIP bug report with redacted settings, environment, extensions and recorded failures, optionally with the session transcript, for attaching to a GitHub issue.

### Sessions and runtime

- Write the session file when the first message is sent, so leaving before saying anything leaves no file.
- Add `--session-id <id>` to resume or start a workspace session under an exact id, and `--name`/`-n` to name the session at startup in every mode.
- Read `AGENTS.override.md` in place of a directory's `AGENTS.md` and `CLAUDE.md`, keeping instructions from other directories.
- Add `compaction` token budgets (`reserve_tokens`, `keep_recent_tokens`) with per-model overrides.
- Fit attached, `read`, and tool-result images to per-model `image_limits` once as they join the conversation, so history and prompt caches stay stable across model switches.
- Keep valuable prompt caches warm during long tool runs, and optionally between runs, with cost-aware one-token refreshes (`cache_warming`, default `streaming`; `prompt_cache_lifetimes`). Refreshes are recorded as `cache_warm` ledger events and billed.
- Show cache warming in `/session`: the mode, the next decision with its economics, or why nothing is being warmed. With `cache_miss_notices` on, the transcript shows each refresh with its cost, and extensions can override each decision with the `cache_warming_decision` event.
- Add a global `http_proxy` setting applied as `HTTP_PROXY` and `HTTPS_PROXY` to micro's HTTP clients.
- Honor `Retry-After` (seconds or HTTP date) and `retry-after-ms` when retrying provider requests, falling back to exponential backoff when absent or unreadable.

### Extensions and RPC

- Let an extension's `tool_call` handler return `terminate: true` with a block, ending the run without another model call when every call in the batch was blocked that way.
- Add the `provider_stream_event` extension event with each parsed provider stream event before micro normalizes it.
- Add RPC `clear_queue`, which removes and returns queued steering and follow-up messages, and report each `prompt`, `steer`, and `follow_up` disposition (`started` or `queued`) in its response.

## [0.1.13] - 2026-09-04

- Show `micro — <workspace>` in terminal tabs and name Bun extension host processes.

## [0.1.12] - 2026-09-01

- Allow manual compaction below the automatic threshold.
- Fork sessions from the persisted conversation branch.
- Exclude Git internals from file completion.
- Add public and authenticated installation bootstrap commands.
- Ignore local generated artifacts.
- Add the MIT license, security policy, and contribution guide.
- Reject filesystem symlink escapes and keep the Bun extension host read-only.
- Apply the session sandbox to RPC shell commands and rebuild the provider runtime on model switches.
- Pair phones through a secret-bearing QR code, require encrypted remote relays, and reject authenticated frame replays after reconnect.
- Store session files with owner-only permissions and make failed deletion retryable.
- Record request pricing for stable historical bills and correct compaction branch totals.
- Preserve run-only trust and refresh the skill command registry during `/reload`.
- Refresh live model listings at startup when `live_models` is enabled.
- Load all maintained extension examples and enforce the compatibility sweep in CI.
- Include MIT and Apache licensing material in release archives.
- Use the standalone sandbox helper when Linux tests launch extension hosts.
- Preserve dependency installation guidance when Bun reports an imported package.

## [0.1.11] - 2026-08-14

- Authenticate managed update checks and downloads for private GitHub releases.

## [0.1.10] - 2026-08-10

- Reject external entities and DTD declarations while parsing Typst XML.
- Authenticate the dependency-audit workflow for the private repository.

## [0.1.9] - 2026-07-31

- Publish release artifacts from the private repository.

## [0.1.8] - 2026-07-24

- Align Linux sandbox setup and tests with hosted CI runners.

## [0.1.7] - 2026-07-20

- Avoid root propagation remounts during Linux sandbox setup.

## [0.1.6] - 2026-07-13

- Isolate RPC interruption test workspaces.

## [0.1.5] - 2026-07-06

- Map the Linux sandbox child process from its parent namespace.

## [0.1.4] - 2026-07-01

- Map the sandbox identity before entering the Linux namespace.

## [0.1.3] - 2026-06-23

- Preserve permitted root writes under the Linux sandbox.

## [0.1.2] - 2026-06-18

- Pass the Linux sandbox lint gate.

## [0.1.1] - 2026-06-15

- Improve Linux handling for protected sandbox paths. Landlock cannot exclude protected descendants from a writable workspace; see the [known gaps](docs/sandbox.md#known-gaps).

## [0.1.0] - 2026-06-08

The first release includes the terminal agent, provider integrations, append-only sessions, billing, prompt-cache diagnostics, project configuration, command sandboxing, extensions, MCP tools, remote control, and managed updates. The core agent is a Rust binary; TypeScript extensions require Bun.

[Unreleased]: https://github.com/rmonvfer/micro/compare/v0.1.13...HEAD
[0.1.13]: https://github.com/rmonvfer/micro/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/rmonvfer/micro/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/rmonvfer/micro/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/rmonvfer/micro/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/rmonvfer/micro/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/rmonvfer/micro/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/rmonvfer/micro/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/rmonvfer/micro/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/rmonvfer/micro/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/rmonvfer/micro/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/rmonvfer/micro/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/rmonvfer/micro/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/rmonvfer/micro/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rmonvfer/micro/releases/tag/v0.1.0
