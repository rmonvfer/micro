# Getting started

The release installer downloads the native binary for macOS on Apple Silicon and Linux on x86_64 or ARM64. Linux binaries require glibc 2.35 or later; musl builds are not provided. Bun is optional and only required for TypeScript extensions.

## Install

Install the latest public release with:

```bash
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  https://raw.githubusercontent.com/rmonvfer/micro/main/scripts/install.sh | bash
```

If GitHub requires authentication for the repository, authenticate the GitHub CLI and fetch the same script through the API:

```bash
gh auth login
gh api -H "Accept: application/vnd.github.raw+json" repos/rmonvfer/micro/contents/scripts/install.sh | bash
```

The installer verifies the release checksum, keeps versioned copies under `~/.local/share/micro/dist`, and links `micro` from `~/.local/bin`. Packaged interactive installations check for updates automatically once every 24 hours. Set `auto_update` to `false`, set `MICRO_NO_AUTO_UPDATE=1`, or run `micro update` when you want explicit control.

Public release checks do not require a token. For a private repository or authenticated API access, micro reads `MICRO_GITHUB_TOKEN`, then `GITHUB_TOKEN`, then `GH_TOKEN`, and otherwise reuses the token from `gh auth login`. The token must be able to read the repository's releases.

To build from a source checkout instead:

```bash
cargo install --path crates/micro-cli
```

To run the checkout without installing it:

```bash
cargo run --bin micro -- "explain this repository"
```

## Authenticate a provider

Sign in to at least one provider:

```bash
micro auth login anthropic
micro auth status
```

`micro auth login` signs in with a subscription or account where the provider offers one, such as a Claude Pro/Max or ChatGPT plan, or reads an API key from the terminal. Credentials are stored in `auth.json` with user-only permissions. Over SSH, `micro auth login anthropic --method copy_code` lets you finish the sign-in in a browser on another machine.

Environment variables also work:

```bash
export ANTHROPIC_API_KEY=...
export OPENAI_API_KEY=...
export GEMINI_API_KEY=...
export OPENROUTER_API_KEY=...
```

Stored credentials take precedence over environment variables. See [Providers and models](providers.md) for the full resolution rules.

## Run a first session

Open the terminal interface:

```bash
cd /path/to/project
micro
```

You can supply the first prompt on the command line:

```bash
micro "find the entry point and explain how requests are handled"
```

micro treats the current directory as the workspace. Use `-C` to select another directory:

```bash
micro -C /path/to/project "run the tests and summarize the failures"
```

Before the first message, the interface opens on micro's mark with the version and the most useful keys beside it, followed by the instruction files, skills, prompts and extensions that loaded. Press `ctrl+o` to list every key and the path each resource was read from; press it again to fold the list back. The screen gives way to the conversation once there is one, and `quiet_startup` in [Configuration](configuration.md) shortens or hides it.

The default sandbox policy allows writes inside the workspace and blocks network access. Built-in file tools keep `.git` and `.micro` read-only; command-level protected-path enforcement is platform-specific. See [Security model](security.md) before changing the policy.

### Read the transcript

In fullscreen mode, Page Up and Page Down scroll the transcript, and a "Jump to latest message" label appears while it is scrolled up; click it or press `end` to follow new output again. `ctrl+f` opens a search box at the top right: type to find text in the rendered transcript, press `enter` or `ctrl+g` for the next match and `shift+enter` or `ctrl+shift+g` for the previous one, and `escape` to close it. In regular mode `ctrl+f` keeps moving the cursor right.

Dragging the mouse selects text, a double click selects a word, and a triple click selects a paragraph. Selections are copied as soon as they are made unless `copy_on_select` is off; `ctrl+x` copies the active selection, or the last answer when nothing is selected.

## Run without the interface

`--print` runs the prompt to completion and exits:

```bash
micro --print "summarize src/"
```

The final response goes to standard output. Tool progress goes to standard error. Add `--quiet` to suppress progress:

```bash
micro --print --quiet "list the public API" > api.txt
```

Print mode is suitable for scripts and CI. If a project contains executable `.micro/` configuration and has no saved trust decision, print mode ignores that project configuration because it cannot ask for approval.

## Select a model

List the bundled catalog:

```bash
micro models
```

Filter it or fetch live provider listings:

```bash
micro models sonnet
micro models --live
```

Use `-m` for one run:

```bash
micro -m opus "review this patch"
micro -m anthropic/claude-sonnet-5 "review this patch"
```

Model queries accept an exact ID, a provider-qualified ID, an alias, or a unique partial match. micro prints the candidates when a query is ambiguous.

Inside the interface, `/model` and `/thinking` change the model and reasoning effort for the current session. Press `ctrl+s` on a model or level in their pickers, or put `--default` before the argument, to also save it as the default for new sessions:

```text
/model --default anthropic/claude-sonnet-5
/thinking --default high
```

## Resume work

Sessions are saved while they run.

```bash
micro sessions list
micro --continue
micro --resume <SESSION_ID>
```

`--continue` selects the most recent session for the current workspace. `--resume` selects a specific session.

Inside the interface, `/sessions`, `/resume`, `/tree`, and `/fork` provide the same workflow without leaving the terminal UI.

## Next steps

- Read [Sessions](sessions.md) to inspect requests, costs, and cache misses.
- Read [Configuration](configuration.md) to set defaults and add project instructions.
- Read [MCP servers](mcp.md) to connect external tools.
- Read [Extensions](extensions.md) to add tools, commands, and UI components.
- Use `micro --help` and [CLI reference](cli-reference.md) for the full command list.
