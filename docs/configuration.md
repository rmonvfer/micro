# Configuration

Everything micro keeps lives in one directory. `MICRO_DIR` names it; when that variable is
unset the directory is `~/.micro`. Every crate that stores anything resolves the location
the same way, so pointing `MICRO_DIR` at a scratch directory moves credentials, models,
policy, and sessions together — which is how a test run or a second profile stays clear of
your real setup.

```
~/.micro/
├── auth.json      credentials, one entry per provider
├── models.json    additions and overrides for the model catalog
├── policy.json    standing rules about what may run unasked
├── config.json    remembered settings
└── sessions/      one JSONL log and metadata sidecar per conversation
```

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

## policy.json

The standing answer to what may run without being asked. Absent, it means the cautious
default.

```json
{
  "mode": "workspace",
  "rules": {
    "bash:cargo": "allow",
    "bash:git push": "ask",
    "write:src/generated.rs": "deny"
  }
}
```

`mode` is `cautious`, `workspace`, or `unrestricted`, and `--approve` overrides it for a
single run. A rule key is either a tool name — `bash`, `read`, `write`, `edit`,
`multi_edit`, `ls`, `grep`, `find` — or a tool and a subject after a colon. The value is
`allow`, `ask`, or `deny`.

The subject is a command for `bash` and a path for a file tool. A `bash` subject matches as
a prefix, token by token, so `bash:cargo` covers every cargo invocation while leaving the
rest of a chained command to be judged separately. Rules are consulted from most specific to
least — an exact rule, then a prefix rule, then the tool's own rule, then the mode — and a
command that cannot be undone is refused ahead of all of it unless an exact rule names that
precise command.

Grants made by answering "allow for the session" are remembered in memory for that run only
and never written here. They match the exact invocation that was approved and nothing
broader.

## config.json

Settings remembered between runs. Every field is optional, and a key written by a version
that knew more is preserved rather than dropped when the file is saved.

```json
{
  "model": "opus",
  "provider": "anthropic",
  "thinking": "medium",
  "theme": "dark",
  "approval": "cautious",
  "live_models": true
}
```

`model` is a query the catalog resolves — an id, a qualified id, a prefix, or an alias —
rather than an assertion that a particular model exists. `thinking` is `off`, `low`,
`medium`, or `high`; `approval` takes the same three modes as `policy.json`; `live_models`
merges live provider listings into the catalog at startup.

Three layers decide what is in force, each beating the one below: a command-line argument,
then an environment variable, then this file. The variables are `MICRO_MODEL`,
`MICRO_PROVIDER`, `MICRO_THINKING`, `MICRO_THEME`, `MICRO_APPROVAL`, and
`MICRO_LIVE_MODELS`.

The `micro` binary reads `MICRO_MODEL` and `MICRO_PROVIDER`, and takes the rest from
`--model`, `--provider`, `--thinking`, and `--approve` on the command line.

## sessions/

One conversation per session, as two files: a JSONL log with one serialized message per
line, and a metadata sidecar carrying the id, workspace, model, and title so that listing
sessions does not mean replaying every log.

Messages are appended as they are produced rather than written when a run ends, so an
interrupted run keeps everything said before the interruption. The log is never rewritten,
which means a crash costs at most the line being written, and loading skips any line it
cannot parse instead of refusing the session.

`micro sessions list` shows the sessions belonging to the current workspace, `--all` shows
every one, and `micro sessions delete <id>` removes a log and its sidecar. `--resume <ID>`
and `--continue` reopen a conversation, seeding the agent with the messages already on disk.

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
