# micro

A coding agent that runs in a terminal. It reads and edits files in a workspace you choose,
runs shell commands when you let it, and asks before doing anything it cannot take back.
One native binary, no runtime to install.

```bash
cargo install --path crates/micro-cli
micro auth login anthropic
micro "what does this project do?"
```

From a checkout, `cargo run --bin micro -- "…"` does the same without installing.

## Authenticating

micro talks to Anthropic, OpenRouter, GitHub Copilot, Google Gemini, and OpenAI. Sign in
once per provider and the credential is stored in `~/.micro/auth.json`, readable only by
you:

```bash
micro auth login openrouter    # pastes an API key
micro auth login github-copilot # opens a browser, device-code flow
micro auth status               # which providers are ready
```

A provider you have not signed into still works if its conventional environment variable is
set — `ANTHROPIC_API_KEY`, `OPENROUTER_API_KEY`, `GEMINI_API_KEY`, `OPENAI_API_KEY`, or one
of `COPILOT_GITHUB_TOKEN` / `GH_TOKEN` / `GITHUB_TOKEN`. The stored credential wins when
both exist. If you already use agent47, `micro auth import` adopts its credentials rather
than making you authenticate a second time.

## Interactive and one-shot

With no `--print`, micro opens a full-screen interface: the conversation scrolls, responses
stream in as they arrive, diffs are rendered inline, and a prompt on the command line seeds
the first turn. It returns you to your shell when you leave.

`--print` runs a single prompt to completion and exits, writing the response to stdout and
tool progress to stderr. This is the form to pipe, to put in a script, or to run in CI:

```bash
micro --print "summarize the error handling in src/"
micro -p -q "list the public functions in micro-agent" > api.txt
```

`-q` suppresses the tool progress so only the model's answer reaches the terminal.

Pick a model with `-m`, which accepts an exact id, a provider-qualified id, a unique
prefix, or a short alias, and reports the candidates rather than guessing when a query
matches several. `micro models` prints the catalog with prices and context windows, and
`micro models --live` merges each provider's current listing first.

```bash
micro -m opus "…"                          # an alias
micro -m anthropic/claude-sonnet-5 "…"     # provider-qualified
micro models sonnet
```

Every run is written to `~/.micro/sessions/` as it happens. `micro sessions list` shows
those from the current workspace, `--all` shows every one, and `--resume <ID>` or
`--continue` picks a conversation back up where it stopped.

## Approval

The agent asks before it acts, and how much it asks is the `--approve` mode.

**cautious**, the default, lets it read freely — `read`, `ls`, `grep`, `find` — and asks
about everything that changes a file or runs a command. It is the only mode that is safe
without knowing anything about the workspace. **workspace** additionally lets it write and
edit inside the workspace root, which the file tools already confine it to, while still
asking about every shell command, because a command can reach anywhere. **unrestricted**
allows everything except the handful of commands that cannot be undone.

A shell command gets more than a string comparison. The command line is split into the
programs it actually runs and each is judged on its own, so a rule permitting `git status`
says nothing about `git status; rm -rf ~`. Anything the parser will not vouch for —
substitution, subshells, expansion — is escalated rather than assumed harmless.

Only `--print` can ask you a question: the interface holds the terminal in raw mode and
cannot prompt, so there it refuses the call and explains why instead of running it
unapproved. Standing rules live in `~/.micro/policy.json`, and answering "allow for the
session" remembers exactly that invocation and nothing broader. See
[docs/configuration.md](docs/configuration.md).

## How a request flows

`micro-cli` resolves a model from the catalog, a credential from the store, and a workspace
root, then hands `micro-agent` a provider, a set of tools, and the conversation so far.
`Agent::run` builds a `Context` — system prompt, messages, tool definitions — and asks the
provider to stream it.

`micro-provider` turns that `Context` into an HTTP request in the shape the endpoint wants,
and the response body back into `StreamEvent`s on a channel. The agent forwards each one as
an `AgentEvent`, so the interface can paint tokens as they arrive. When the response is
complete, the agent runs whatever tools it asked for, appends each result to the
conversation, and goes around again until the model stops asking for tools.

The crates either side of that path do one thing each. `micro-types` holds the conversation
model every layer speaks. `micro-tools` holds the capabilities the model can invoke;
`micro-policy` wraps each of them in the approval gate. `micro-models` is the model catalog,
`micro-auth` the credentials, `micro-context` the project instructions and the compaction
that keeps a long conversation inside the context window, and `micro-session` the durable
log. `micro-tui` draws the interface and `micro-testkit` provides the fakes the agent loop
is tested against. Nothing depends on a layer above it, so the provider crate has no idea
tools exist and the tools have no idea a model is calling them.

[docs/architecture.md](docs/architecture.md) covers why the seams fall where they do.
[docs/providers.md](docs/providers.md) covers the providers and the model catalog.
