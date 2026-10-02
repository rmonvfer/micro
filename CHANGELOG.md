# Changelog

## [Unreleased]

- Configure MCP servers in `mcp.json`, globally and in trusted projects, instead of `mcp_servers` in settings. Entries use the `mcpServers` shape shared with other MCP clients and may carry a `description`.
- Connect to MCP servers over streamable HTTP, with `url` and `headers`, alongside stdio.
- Sign in to MCP servers with OAuth: discovery, dynamic client registration with `oauth.clientName`, PKCE with a loopback callback and a clickable sign-in link, `oauth.authServerMetadataUrl`, RFC 9207 issuer checks, refresh, and step-up sign-in that keeps granted scopes. Credentials are stored per server name and URL, readable only by their owner.
- Send a provider's micro credential to an HTTP MCP server with `"auth": { "provider": "<name>" }`, allowed only in the global `mcp.json`.
- Add `micro mcp add|remove|list|login|logout` and the `/mcp` command.
- Name MCP tools with `-` written as `_` (`mcp__my_server__x`), give colliding tool names a hash suffix, and reject server names that differ only in `-` and `_`.
- Connect MCP servers side by side; servers with `"exposure": "deferred"` connect in the background and `tool_search` waits for them. An `mcp_servers` system prompt section lists servers whose tools are not declared.
- Title MCP tool calls `server/tool`.
- Sign in to Anthropic with a Claude Pro/Max subscription, by browser or by pasting the code Anthropic shows when the browser runs elsewhere.
- Sign in with ChatGPT for the `openai` provider, and sign in to `openai-codex` by browser or device code.
- Sign in to OpenRouter (pasting the redirect URL over SSH), xAI with SuperGrok or X Premium, and Kimi Code.
- Refresh expiring OAuth tokens under the credential file lock.
- Open Anthropic subscription requests with Claude Code's identity.
- Use Anthropic workload identity federation from `ANTHROPIC_FEDERATION_RULE_ID`, `ANTHROPIC_ORGANIZATION_ID` and `ANTHROPIC_IDENTITY_TOKEN_FILE`.
- Add `micro auth check`, `micro auth print-api-key` and `micro auth print-bearer-token`.
- Label `/login` and `/logout` entries as `not configured`, or as an API key, `subscription` or `account`.
- Add the `system` theme, now the default, which derives colors from the terminal's reported foreground, background, and ANSI palette, keeps pastel palettes pastel, and rebuilds when the terminal regains focus after switching between light and dark.
- Accept `#rgb`, `oklch()`, and `okhsl()` colors and an optional `appearance` field in theme files.
- Keep `/model` and `/thinking` choices to the current session, and save one as the default with `ctrl+s` in their pickers or `--default` before the argument.
- Search the fullscreen transcript with `ctrl+f`: matches are highlighted, `enter` and `shift+enter` step between them, and `escape` closes the search.
- Show a clickable jump-to-latest label while the fullscreen transcript is scrolled up, return with `end`, and add `half_page_scroll` for half-page Page Up and Page Down.
- Select a word with a double click and a paragraph with a triple click, copy the active selection with `ctrl+x`, and add `copy_on_select` to turn off automatic selection copy.
- Add `external_editor` to choose the `ctrl+g` editor ahead of `$VISUAL` and `$EDITOR`, and run editor commands that carry arguments.
- Add `output_pad` to set the transcript's horizontal padding apart from `content_padding`.
- Accept `quiet_startup: "header"` to keep the startup header with the version and key hints and hide the rest.
- Link file paths in built-in file tool titles with OSC 8 `file://` hyperlinks.
- Add `terminal_hyperlinks`, `terminal_images`, and `terminal_true_color` to override detected terminal capabilities, and draw themes with the 256-color palette on terminals without 24-bit color.
- Show the arguments of tool calls without a custom renderer as `key=value` pairs when collapsed and `key: value` lines when expanded.
- Complete slash commands when the prompt starts with whitespace.
- Write the session file when the first message is sent, so leaving before saying anything leaves no file.
- Add `--session-id <id>` to resume or start a workspace session under an exact id, and `--name`/`-n` to name the session at startup in every mode.
- Save truncated `bash` output in full to a private temporary file and name its path in the result.
- Read `AGENTS.override.md` in place of a directory's `AGENTS.md` and `CLAUDE.md`, keeping instructions from other directories.
- Add a global `http_proxy` setting applied as `HTTP_PROXY` and `HTTPS_PROXY` to micro's HTTP clients.
- Honor `Retry-After` (seconds or HTTP date) and `retry-after-ms` when retrying provider requests, falling back to exponential backoff when absent or unreadable.
- Add RPC `clear_queue`, which removes and returns queued steering and follow-up messages, and report each `prompt`, `steer`, and `follow_up` disposition (`started` or `queued`) in its response.
- Add the `default_tools` setting (user and trusted project) with `+name`/`-name` entries; `/reload` turns on tools newly added to it, and `--tools` still overrides it.
- Add `compaction` token budgets (`reserve_tokens`, `keep_recent_tokens`) with per-model overrides.
- Fit attached, `read`, and tool-result images to per-model `image_limits` once as they join the conversation, so history and prompt caches stay stable across model switches.
- Keep valuable prompt caches warm during long tool runs, and optionally between runs, with cost-aware one-token refreshes (`cache_warming`, default `streaming`; `prompt_cache_lifetimes`). Refreshes are recorded as `cache_warm` ledger events and billed.

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
