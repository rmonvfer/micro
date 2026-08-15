# micro

A coding agent that runs in a terminal and keeps the receipts. Every session is an itemized,
auditable record of what the model saw, what it did, what it touched, and what it cost —
down to the cache miss. One append-only ledger holds all of it: the exact request each turn
sent and where every stretch of that prompt came from, what the provider billed for it, what
the operating system stopped a command from touching, and what an extension asked for and
was told. Everything else is a reading of that one file, which sits on your own disk in a
documented format and goes nowhere. One native binary, no runtime to install.

```bash
cargo install --path crates/micro-cli
micro auth login anthropic
micro "what does this project do?"
```

From a checkout, `cargo run --bin micro -- "…"` does the same without installing.

## The receipts

Ask what a session cost, and it answers turn by turn, broken down by where the money went:

```
$ micro bill
Bill for session 1786754321

turn 1                          anthropic/claude-opus-5  $0.004212
  system_prompt                    2,847 B  $0.000317
  project_instructions               392 B  $0.000044
  tool:bash                        1,204 B  $0.000134
  model (output)                            $0.003717

turn 2                          anthropic/claude-opus-5  $0.008628
  system_prompt                    2,847 B  $0.000032
  project_instructions               392 B  $0.000004
  tool:bash                        1,204 B  $0.000013
  user                               118 B  $0.000001
  model (output)                            $0.008578

Total                                       $0.012840

2 turns  118 in  1,061 out  1,109 read from cache  1,109 written to it

What each turn cost is exact: it is what the provider billed, at the
rates the model charges. How a turn is shared out between its sources
is an estimate, worked out from the bytes each one put into the prompt
— but the shares always add up to the turn.
```

`/bill` reads the same thing inside a session, and `micro bill <id> --diff 4` answers what
one turn added and why. A running total sits in the footer while you work.

The prompt in the second turn cost almost nothing because the provider still had it. When a
turn pays for a prompt twice, the ledger can say what broke it:

```
$ micro why-miss 1786754321 4
session 1786754321  turn 4

The prefix changed between turn 3 and turn 4:
  from  9f2a1c4e7b03
  to    41bd77a0e2f5

The project_instructions span of the prompt changed.

  - Run the tests with `cargo test`.
  + Run the tests with `cargo nextest run`.

The cache broke because the project's instructions and skills were read again (reload),
recorded at seq 42.
```

Commands run confined by the operating system, and a refusal is a fact rather than a
mystery. The model is told what happened in terms it can act on, and the session records it:

```
$ micro sessions export 1786754321 | grep sandbox_decision
{"v":1,"seq":18,"ts":1786754322511,"event":{"type":"sandbox_decision","policy":"workspace-write","operation":"write","path_or_host":"/etc/hosts","allowed":false,"tool_call_id":"call_1"}}
```

And when you want to know what the model was actually shown, rather than what it said:

```
$ micro sessions show 1786754321 --turn 4 --raw
```

That rebuilds the request from what was recorded, checks it against the hash of the body
that went out, and prints it. If the rebuild does not match, it says so — a record you
cannot check against anything is just a story.
[docs/ledger.md](docs/ledger.md) documents the format, which is versioned and public.

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

Pick a model with `-m`, which accepts an exact id, a provider-qualified id, a unique prefix,
or a short alias, and reports the candidates rather than guessing when a query matches
several. `micro models` prints the catalog with prices and context windows, and
`micro models --live` merges each provider's current listing first.

```bash
micro -m opus "…"                          # an alias
micro -m anthropic/claude-sonnet-5 "…"     # provider-qualified
micro models sonnet
```

Every run is written to disk as it happens. `micro sessions list` shows those from the
current workspace, `--all` shows every one, and `--resume <ID>` or `--continue` picks a
conversation back up where it stopped.

`--budget 5` stops a session at the first turn boundary past five dollars, with a line in
the ledger saying so; a `budget` setting does the same for every run. The ceiling covers
the whole conversation: what earlier runs of the same session spent counts too.

## Authenticating

micro talks to every service in its bundled catalog — Anthropic, OpenAI, Google, OpenRouter
and GitHub Copilot among them, along with the platform endpoints, the inference hosts and
the gateways. Sign in once per provider and the credential is stored in `auth.json` in
micro's configuration directory, readable only by you:

```bash
micro auth login openrouter     # pastes an API key
micro auth login github-copilot # opens a browser, device-code flow
micro auth status               # which providers are ready
```

A provider you have not signed into still works if its environment variable is set:
`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY`,
`COPILOT_GITHUB_TOKEN`, and otherwise the conventional `<PROVIDER>_API_KEY`. The stored
credential wins when both exist. If you already use agent47, `micro auth import` adopts its
credentials rather than making you authenticate a second time.

A model micro has never heard of is a `models.json` entry away, which is how a local
endpoint joins the catalog:

```json
{ "providers": { "ollama": {
    "base_url": "http://localhost:11434/v1", "api": "openai-completions",
    "models": [{ "id": "qwen3-coder:30b", "name": "Qwen3 Coder 30B", "aliases": ["local"] }]
} } }
```

`micro -m local "…"` reaches it from then on, and everything above — the ledger, the bill,
the sandbox — works the same. A bill that comes to zero says which kind of zero it is: a
model that charges nothing per token, or one the catalog has no price for at all.

## Trusting a project

The stance is that the agent should act freely, because what it does is confined and
recorded. Three things make that true, and they are decided before any of the project's own
code is read.

**Trust** is about whether a project may run what it ships: its own settings, the extensions
it carries, the skills and prompts it offers. Reading a project is one thing; running what
it contains is another, and that is the one thing micro asks about. A project carrying none
of it — no `.micro/` directory — is used without a question, which is most of them. One that
does is answered by whatever was decided about it before, then by the `default_project_trust`
setting (`ask`, `always` or `never`), and only then by asking. With nobody at a terminal
there is nobody to ask, so `--print` and `--rpc` leave an undecided project alone rather than
running its code unasked. `/trust` settles it either way and the decision is remembered.
`--approve` trusts the project for one run and `--no-approve` refuses it for one run; neither
is written down, which is what a scripted run wants.

**The sandbox** is about what any command may touch, trusted or not. By default a session
writes inside its workspace and nowhere else, and reaches no network; `.git` and `.micro`
stay read-only inside it — whoever writes your git hooks owns your next commit.
`--sandbox read-only` narrows that for a run and `--sandbox full` lifts it
loudly. The operating system enforces it — a Seatbelt profile on macOS, Landlock and seccomp
on Linux — and on a platform with no sandbox behind it the file tools still enforce the
policy and commands run unconfined, which micro says rather than implies. Every refusal is
told to the model and written to the ledger. See [docs/sandbox.md](docs/sandbox.md).

**Capabilities** are about what an extension may ask micro for. An extension declares the
set in its own package, and anything outside it is refused where the ask arrives — by a name
the extension can catch, with the session carrying on and the attempt recorded. That is the
path to running someone's extension without vouching for the whole checkout it came in.

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
model or the thinking level. Everything the phone submits goes in the way a typed line does,
so a command it sends runs as that command and shows up in the terminal's own transcript.

Nothing in between can read any of it. The phone and the machine derive keys from a secret
they share and the relay never holds; what crosses the relay is ciphertext it routes without
being able to open. The secret is written to `remote-control.json` in micro's data
directory, readable only by you. `MICRO_REMOTE_RELAY_URL` points the pairing at a relay of
your own.

## Extensions

An extension is a TypeScript file that adds tools, commands, shortcuts, event handlers, and
pieces of the interface. A single file needs no build step and no install step:

```ts
export const capabilities = ["commands", "ui"];

export default (micro) => {
    micro.registerCommand("hello", {
        description: "say hello",
        handler: async (args, ctx) => ctx.ui.notify(`hello ${args || "world"}`),
    });
};
```

The API is the one pi extensions are written against, and micro is compatible with it as far
as the vendored examples test — every extension in `examples/extensions` came from pi's own
examples, and the compatibility suite runs each one against the real binary: single files
and installed packages alike, including the ones that only do anything with a person at a
terminal. Some of it has no counterpart here: pi's own agent loop, its session runtime, its
interactive mode, and its terminal image protocols throw a named error when called rather
than at import. An extension that reaches for none of those runs unmodified, imports from
pi's own packages included. That is a migration path rather than a claim of completeness.

What is different is the manifest. An extension says what it needs — `tools`, `exec`,
`context`, `ui`, and the rest — and micro holds it to that at the broker rather than
trusting it because the file was installed. An extension that declares nothing still runs:
micro works out the set it would need, asks once, and remembers the answer.

[docs/extensions.md](docs/extensions.md) covers the API, the capability names and the
install paths; [docs/extension-testing.md](docs/extension-testing.md) covers the harnesses.

## Where things live

`MICRO_DIR` names a single directory and everything goes in it. Otherwise an existing
`~/.micro` keeps holding everything, so an installation made before this rule existed never
moves. Otherwise what you wrote and what micro produced are kept apart, where the XDG base
directory specification says each belongs: credentials, settings, themes, prompts and skills
under `~/.config/micro`, and sessions, installed packages and the pairing secret under
`~/.local/share/micro`. The line between them is authorship. Your files are worth carrying
to the next machine; micro can always regenerate its own.
[docs/configuration.md](docs/configuration.md) has the full layout.

## What micro will not do

There is no telemetry. Nothing is reported, counted, or phoned home, in any build, under any
setting. There are no accounts and there is no hosted anything.

The ledger is yours. It is a JSONL file under your own directory that you can read with
`grep`, hand to a script, or delete, and `micro sessions export` prints it whole in a
documented, versioned format. Nothing about a session leaves the machine except the request
you asked to be sent, to the provider you chose.

There is no semantic index and no retrieval layer. The model gets read, grep, find, and a
shell, which is what it is good at, and micro spends its complexity on knowing exactly what
was sent instead.

## How a request flows

`micro-cli` resolves a model from the catalog, a credential from the store, a workspace root,
and a sandbox policy, then hands `micro-agent` a provider, a set of tools, and the
conversation so far. `Agent::run` builds a `Context` — system prompt, messages, tool
definitions — records what it is about to send, and asks the provider to stream it.

`micro-provider` turns that `Context` into an HTTP request in the shape the endpoint wants,
and the response body back into `StreamEvent`s on a channel. The agent forwards each one as
an `AgentEvent`, so the interface can paint tokens as they arrive. When the response is
complete, the agent runs whatever tools it asked for, appends each result to the
conversation, and goes around again until the model stops asking for tools. Everything that
happens on the way — the request, the usage, the refusals, the extension crossings — goes to
the session on one channel, in the order it happened.

The crates either side of that path do one thing each. `micro-types` holds the conversation
model and the ledger's vocabulary. `micro-tools` holds the capabilities the model can
invoke, and `micro-sandbox` decides what any of them may touch. `micro-models` is the model
catalog, `micro-auth` the credentials, `micro-context` the project instructions and the
compaction that keeps a long conversation inside the context window, and `micro-session` the
durable log. `micro-tui` draws the interface, `micro-extensions` runs someone else's code
beside micro rather than inside it, `micro-remote` carries a session to a phone, and
`micro-testkit` provides the fakes the agent loop is tested against. Nothing depends on a
layer above it, so the provider crate has no idea tools exist and the tools have no idea a
model is calling them.

## About Bun

Extensions run in a Bun process of their own. The core binary embeds no JavaScript runtime:
an extension reaches micro by asking over a pipe, which is what makes the capability
manifest enforceable and what keeps a third-party file out of the agent's own address space.
Bun is what a pi-compatible extension expects to run under, and it is owned by Anthropic.
Without Bun installed, micro runs unchanged except that no extension loads — nothing else in
the binary depends on it.

## Documentation

[docs/ledger.md](docs/ledger.md) is the ledger format, the bill, budgets, and `why-miss`.
[docs/sandbox.md](docs/sandbox.md) covers the policies, what enforces them on which
platform, and how to check.
[docs/architecture.md](docs/architecture.md) covers why the seams fall where they do.
[docs/configuration.md](docs/configuration.md) covers where things live and every setting.
[docs/providers.md](docs/providers.md) covers the providers and the model catalog.
[docs/extensions.md](docs/extensions.md) covers writing, installing and confining an
extension, and what an extension written for pi can expect here.
[docs/extension-testing.md](docs/extension-testing.md) covers the harnesses that check
extensions, and why one of them drives a real terminal.
