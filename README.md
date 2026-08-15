# micro

A coding agent that runs in a terminal. It reads and edits files in a workspace you choose
and runs shell commands there, once you have said the project may run its own code.
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

## Trusting a project

A project can carry things it asks micro to run: its own settings, the extensions it ships,
the skills and prompts it offers. Reading a project is one thing; running what it contains
is another, and that is the one thing micro asks about.

A project carrying none of it — no `.micro/` directory — is used without a question, which
is most of them. One that does is answered by whatever was decided about it before, then by
the `default_project_trust` setting (`ask`, `always` or `never`), and only then by asking.
With nobody at a terminal there is nobody to ask, so `--print` and `--rpc` leave an
undecided project alone rather than running its code unasked. `/trust` settles it either
way, and the decision is remembered in `~/.micro/trust.json`. `--approve` trusts the
project for one run and `--no-approve` refuses it for one run; neither is written down,
which is what a scripted run wants.

Tool calls themselves are not gated: once micro is running, it acts. `--tools` narrows what
the model is offered to a named list, and `--exclude-tools` withholds particular ones.

## Reaching a session from your phone

A session can be watched and driven from a phone while the terminal stays fully usable.
Pair a phone with the machine once:

```
/remote pair          # prints a link to open in the app
/remote pair qr       # draws it as a code to scan instead
```

From then on, `/remote` puts a session on that phone — no link, no code. It appears in the
app's session list beside every other session this machine has offered, with the ones that
are still live marked as such. Open one and you can read the conversation as it streams,
send a prompt, steer the turn that is running, queue a follow-up, stop it, and change the
model or the thinking level. Everything the phone submits goes in the way a typed line
does, so a command it sends runs as that command and shows up in the terminal's own
transcript.

Nothing in between can read any of it. The phone and the machine derive keys from a secret
they share and the relay never holds; what crosses the relay is ciphertext it routes
without being able to open. `micro remote pair` writes that secret to
`~/.micro/remote-control.json`, readable only by you. `MICRO_REMOTE_RELAY_URL` points the
pairing at a relay of your own.

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
model every layer speaks. `micro-tools` holds the capabilities the model can invoke. `micro-models` is the model catalog,
`micro-auth` the credentials, `micro-context` the project instructions and the compaction
that keeps a long conversation inside the context window, and `micro-session` the durable
log. `micro-tui` draws the interface, `micro-remote` carries a session to a phone, and
`micro-testkit` provides the fakes the agent loop is tested against. Nothing depends on a layer above it, so the provider crate has no idea
tools exist and the tools have no idea a model is calling them.

[docs/architecture.md](docs/architecture.md) covers why the seams fall where they do.
[docs/providers.md](docs/providers.md) covers the providers and the model catalog.
[docs/extensions.md](docs/extensions.md) covers writing and installing an extension, and
what an extension written for pi can expect here.
[docs/extension-testing.md](docs/extension-testing.md) covers the two harnesses that check
extensions, and why one of them drives a real terminal.
