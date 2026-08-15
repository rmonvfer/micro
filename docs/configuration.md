# Configuration

Where micro keeps things is settled by one rule, applied by every crate that stores
anything. `MICRO_DIR` names a single directory and everything goes in it, which is how a
test run or a second profile stays clear of your real setup. Otherwise an existing
`~/.micro` keeps holding everything, so an installation made before this rule existed never
moves. Otherwise micro is new to the machine, and what you wrote is kept apart from what
micro produced, where the XDG base directory specification says each belongs.

```
~/.config/micro/          what you wrote          ($XDG_CONFIG_HOME/micro)
├── auth.json             credentials, one entry per provider
├── models.json           additions and overrides for the model catalog
├── trust.json            which projects may run the code they ship
├── capabilities.json     what each extension was allowed to do
├── config.json           remembered settings
├── SYSTEM.md             a system prompt replacing micro's own
├── APPEND_SYSTEM.md      text added to the end of it
├── themes/               your own colour schemes
├── prompts/              prompt files, each becoming a slash command
└── skills/               a SKILL.md per directory

~/.local/share/micro/     what micro produced     ($XDG_DATA_HOME/micro)
├── sessions/             one JSONL log, metadata sidecar and blob directory per conversation
├── npm/, git/            extension packages `micro install` fetched
├── extensions/           extensions of your own that load everywhere
└── remote-control.json   the phone this machine is paired with
```

The split is about authorship, not importance: what you wrote is worth carrying to another
machine, and what micro produced could be produced again. Under `MICRO_DIR` or an existing
`~/.micro` both columns land in the one directory, exactly as before.

Nothing here has to exist. A missing file means the defaults, not an error.

## auth.json

Written by `micro auth login` and read whenever a provider is resolved, with permissions
that let only its owner read it. One entry per provider, keyed by canonical id —
`anthropic`, `openrouter`, `github-copilot`, `gemini`, `openai` — recording either the key
you pasted or the tokens from a device-code sign-in, which are exchanged for a fresh one
when they expire.

Edit it through the CLI rather than by hand: `micro auth login <provider>` to add,
`micro auth logout <provider>` to remove, `micro auth status` to see what is in force, and
`micro auth import` to adopt the credentials agent47 already holds. A provider with no entry
falls back to its environment variable; see [providers.md](providers.md) for the names.

## models.json

Layered over the catalog compiled into the binary. Provider-level settings apply to every
model already under that provider, and a model entry either patches one that exists or
registers a new one. Only the fields you name are touched.

```json
{
  "providers": {
    "ollama": {
      "base_url": "http://localhost:11434/v1",
      "api": "openai-completions",
      "models": [
        { "id": "qwen3-coder:30b", "name": "Qwen3 Coder 30B", "aliases": ["local"] }
      ]
    },
    "anthropic": {
      "models": [{ "id": "claude-opus-5", "max_output_tokens": 64000 }]
    }
  }
}
```

The first block registers a provider the bundled catalog has never heard of, reachable
afterwards as `local` or `ollama/qwen3-coder:30b`. The second changes one number on a model
that already exists and leaves its name, context window, and prices alone.

A model entry takes `id`, `name`, `api`, `base_url`, `context_window`, `max_output_tokens`,
`reasoning`, `input`, `headers`, `aliases`, and `cost` — the last an object of `input`,
`output`, `cache_read`, and `cache_write` prices in dollars per million tokens. `api` is one
of `anthropic-messages`, `openai-completions`, `openai-responses`, or
`google-generative-ai`. Anything a model omits is inherited from its provider block, so a
provider that declares `base_url` and `api` once need not repeat them. A new model that
resolves neither from itself nor its provider is rejected rather than half-registered.

Adding a provider block for a name the catalog already knows re-points every model under it,
which is the way to send a provider through a proxy without listing its models.

## trust.json

What was decided about each project, keyed by its canonical path.

```json
{
  "projects": {
    "/Users/you/code/thing": { "trusted": true, "decided_at": 1786000000000 }
  }
}
```

A project is only asked about when it carries something micro would run or be steered by:
a `.micro/` directory holding `settings.json`, `extensions`, `skills`, `prompts`, `themes`,
`SYSTEM.md` or `APPEND_SYSTEM.md`. Everything else is used without a question.

An undecided project takes the `default_project_trust` setting — `ask`, `always` or
`never`. `ask` puts the question at the terminal before the interface starts; with nobody
there to answer, as in `--print` and `--rpc`, the project is left alone. `/trust` settles it
from inside a session, and takes effect on the next run because what loads was decided
before the first one started.

## config.json

Settings remembered between runs. Every field is optional, and a key written by a version
that knew more is preserved rather than dropped when the file is saved.

```json
{
  "model": "opus",
  "provider": "anthropic",
  "thinking": "medium",
  "theme": "dark",
  "live_models": true
}
```

`interface_padding` is the columns and rows kept clear between the terminal's edges and the
interface; `model` is a query the catalog resolves — an id, a qualified id, a prefix, or an
alias — rather than an assertion that a particular model exists. `thinking` is `off`, `low`,
`medium`, or `high`; `live_models` merges live provider listings into the catalog at
startup.

Three layers decide what is in force, each beating the one below: a command-line argument,
then an environment variable, then this file. The variables are `MICRO_MODEL`,
`MICRO_PROVIDER`, `MICRO_THINKING`, `MICRO_THEME`, and `MICRO_LIVE_MODELS`.

The `micro` binary reads `MICRO_MODEL` and `MICRO_PROVIDER`, and takes the rest from
`--model`, `--provider`, and `--thinking` on the command line.

Any setting at all can be named for one run with `-c key=value`, which writes into this
file's contents as they are read rather than into the file itself: `-c mermaid=off`,
`-c image_width_cells=40`. The value is read as JSON, so `false` is a boolean and `40` is
a number; anything that is not JSON is taken as a string, which is what makes
`-c theme=dracula` work unquoted. The key is a dotted path, which today reaches a key a
newer version wrote, since the settings above are all flat. Repeat the flag to set more
than one, and a later assignment beats an earlier one. A mistyped assignment stops the run
rather than falling back to the stored settings.

## mcp_servers

Programs that provide tools over the Model Context Protocol. Each entry names a server,
and its tools reach the model as `mcp__<server>__<tool>` alongside micro's own.

```json
{
  "mcp_servers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_TOKEN": "..." }
    },
    "notes": { "command": "/usr/local/bin/notes-mcp", "enabled": false }
  }
}
```

`command` is the only required key. `args`, `env`, and `cwd` say how to run it; `env` adds
to the environment the server inherits rather than replacing it. `enabled` is true unless
it says otherwise, so a server can be turned off without being deleted. `startup_timeout`
and `tool_timeout` are in seconds, and default to 20 and 120: unlike a shell command there
is no output to watch while an MCP call runs, so a wedged server costs one call rather than
the session.

A server that will not start is named and skipped. The run goes ahead with the tools that
did load, since a broken entry should cost its own tools and nothing else.

## tool_search_threshold

Every tool on offer is described to the model on every request, so a few MCP servers
between them can cost more of the context window than the conversation does. Past this
many tools beyond the built-in ones, the extra ones stop being described and a
`tool_search` tool is offered in their place: the model searches for what it needs by name
or by what it does, gets back the same descriptions, and calls them as usual. One exchange
in place of a standing charge.

The default is 15. Zero describes every tool however many there are. The built-in tools are
always described, since deferring those would cost a search before the model could read a
file.

## sandbox

What the commands a session runs are allowed to touch: `read-only`, `workspace-write`, or
`full`. The default is `workspace-write` — the workspace is writable, nothing else is, and
the network is off.

```json
{ "sandbox": "workspace-write" }
```

A project can settle it for its own sessions in `.micro/settings.json`, once the project has
been trusted, and `--sandbox` on the command line beats both. [sandbox.md](sandbox.md)
describes what each policy enforces, on which platforms, and how to check it.

## budget

What one session may spend before it stops, in US dollars. Zero, which is also what leaving
it out means, is no ceiling.

```json
{ "budget": 5.0 }
```

`--budget` says the same for one run. The ceiling is on the session rather than the run, so
what earlier runs of the same conversation spent counts against it, and a run that reaches
it stops at the next turn boundary and says so in the ledger. See [ledger.md](ledger.md).

## cache_miss_notices

Off unless it says otherwise. With it on, a turn that wrote a prompt into the provider's
cache without reading any of it back says so on screen, which is the cheap version of the
question [`micro why-miss`](ledger.md) answers properly after the fact.

## sessions/

One conversation per session, as a JSONL log with one fact per line, a metadata sidecar
carrying the id, workspace, model, and title so that listing sessions does not mean
replaying every log, and a directory of the content the log names by hash.

Lines are appended as they are produced rather than written when a run ends, so an
interrupted run keeps everything said before the interruption. The log is never rewritten,
which means a crash costs at most the line being written, and loading skips any line it
cannot parse instead of refusing the session.

The conversation is only part of what is in there. The same log carries what each turn asked
a provider for, what the provider said it cost, what the sandbox refused, and what an
extension was told — see [ledger.md](ledger.md) for the format and for the readings built on
it.

`micro sessions list` shows the sessions belonging to the current workspace, `--all` shows
every one, `micro sessions show <id>` reads one back, `micro sessions export <id>` prints it
whole, and `micro sessions delete <id>` removes a log with its sidecar and its blobs.
`--resume <ID>` and `--continue` reopen a conversation, seeding the agent with the messages
already on disk.

## Project instructions

Separate from the settings above, and read mostly from the workspace rather than from
micro's home. A `CLAUDE.md` or `AGENTS.md` is collected from micro's home directory first,
then from the filesystem root down to the workspace, and all of it is appended to the system
prompt. Reading outward-in means a nearer file is read last and so has the final word: the
home directory holds what applies to every project, a file above the workspace holds what
applies to a group of them, and the workspace's own file wins where they disagree.

An `@import` directive inside one of these pulls in another file, resolved relative to the
file that imported it and followed up to five levels deep before a directive is left as
written.

## The system prompt

A `SYSTEM.md` replaces what the model is told about what it is; an `APPEND_SYSTEM.md` is
added to it. Both are looked for in the project's `.micro/` first and in micro's home
directory second, so a project can speak for itself where it needs to and the user's own
applies everywhere else. The project's copies are read only once the project is trusted:
replacing what the model is told is exactly the kind of thing that decision is about.

Project instructions and the list of available skills are appended after both, so a
`SYSTEM.md` changes the opening of the prompt rather than everything in it.

## prompts/

A markdown file in `prompts/` becomes a slash command named after the file. Running
`/review 42` reads `review.md`, substitutes the arguments into its body, and sends the
result as the prompt.

```markdown
---
description: Review a pull request
argument-hint: <number> [branch]
---
Review PR $1 against ${2:-main} and list anything that would block a merge.
```

Arguments are substituted the way a shell substitutes them: `$1` for the first, `$@` or
`$ARGUMENTS` for all of them, `${1:-default}` for one that may be missing, and `${@:2}` or
`${@:2:3}` for a run of them. What a user typed is text — a `$1` inside an argument stays a
`$1` rather than being substituted again.

A name micro already answers to keeps its meaning: built-in commands are matched first, so
a file called `quit.md` does not take over `/quit`. Prompts are read from micro's home
directory and from a trusted project's `.micro/prompts`, with the project's winning where
both offer the same name.
