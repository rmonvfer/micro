# Providers and models

micro speaks three wire formats across five providers, resolves a credential for each, and
decides which models exist from a catalog assembled in three layers.

## The providers

| Id | Name | Endpoint | Sign-in |
| --- | --- | --- | --- |
| `anthropic` | Anthropic | `api.anthropic.com` | API key |
| `openrouter` | OpenRouter | `openrouter.ai/api/v1` | API key |
| `github-copilot` | GitHub Copilot | `api.individual.githubcopilot.com` | Device code |
| `gemini` | Google Gemini | `generativelanguage.googleapis.com` | API key |
| `openai` | OpenAI | `api.openai.com/v1` | API key |

Three client implementations serve them. `Anthropic` speaks the Messages API, `Gemini`
speaks Google's generative API, and `OpenAi` speaks chat completions with per-host
adjustments for OpenRouter and Copilot. A provider micro does not recognize falls back to
the OpenAI shape, which most gateways implement.

Common alternative spellings are folded onto the canonical id, so `claude` reaches
`anthropic`, `google` reaches `gemini`, and `copilot` or `github` reach `github-copilot`.
The canonical id is what the credential is filed under and what a model's `provider` field
holds.

## How credentials resolve

`AuthStore::resolve` prefers what is stored and falls back to the environment.

A credential stored by `micro auth login` lives in `~/.micro/auth.json`, one entry per
provider, in a file only its owner can read. If the provider issues short-lived tokens, the
stored credential is exchanged for a fresh one at resolve time rather than at login, so a
long-idle installation still works without signing in again.

With nothing stored, the conventional environment variable is used instead:

| Provider | Variables, in order |
| --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |
| `github-copilot` | `COPILOT_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN` |
| `gemini` | `GEMINI_API_KEY`, `GOOGLE_API_KEY` |
| `openai` | `OPENAI_API_KEY` |

`micro auth status` reports each provider's state and which of the two it came from, so a
key that is being shadowed by a stored credential is visible rather than mysterious.

## The model catalog

A model is more than an id. `micro-models` records the display name, the provider, the wire
API it speaks, its endpoint, its context window and output cap, whether it reasons, what
input it accepts, any headers it needs, and its price per million tokens for input, output,
cache reads, and cache writes. The prices are what the interface bases a running cost on.

The catalog is assembled in three layers, each overlaying the last.

**The bundled catalog** is compiled into the binary, so micro works offline with no setup.
It covers the current models on all four of the providers that publish them.

**A user catalog** at `~/.micro/models.json` is applied over it. An entry naming a model
that already exists patches only the fields it mentions; an entry naming a new one registers
it. Provider-level settings re-point every model under that provider at once, which is how a
whole provider moves behind a proxy. See [configuration.md](configuration.md) for the file's
shape.

**Live listings** are merged last, so a model released since the build appears without one.
`micro models --live` fetches OpenRouter's public model list and, when a Copilot credential
is present, the models that account is entitled to. Providers are independent: one that is
unreachable leaves its bundled entries in place and reports the failure rather than emptying
the catalog. A listing is authoritative about what it states and silent about the rest, so
anything it omits — headers, aliases, limits, prices a subscription provider does not quote
— is carried over rather than blanked.

Resolution takes what a user typed and finds one model, in tiers: a provider-qualified id,
then an exact id, then an alias, then a unique prefix, then a substring of an id or display
name. The first tier that matches decides, and matching several is reported as an ambiguity
with the candidates rather than resolved by guessing. The qualified form is tried first and
alone, because an OpenRouter id contains a slash of its own — `anthropic/claude-sonnet-5`
means the model Anthropic serves, and `openrouter/anthropic/claude-sonnet-5` means the one
OpenRouter serves, and both stay reachable.

## Adding a provider

Implement `Provider`, which is one method: turn a `Model`, a `Context`, and an API key into
a receiver of `StreamEvent`s. The work is the translation at both edges — the request body
the endpoint expects, and its response framing back into events. `Anthropic`, `OpenAi`, and
`Gemini` are the three worked examples; an endpoint that already speaks one of those shapes
needs no new client at all.

Register it in `micro-provider`'s registry with its canonical id, display name, default
endpoint, sign-in method, and a conservative output cap for models whose real one is
unknown, then add the id and its environment variables to `micro-auth` so a credential can
be stored and resolved. Add its models to the bundled catalog, or leave that to a live
listing if the provider publishes one.

Nothing above needs to change. The agent loop and the tools never learn
that a provider was added, and a user reaches the new models through the same resolution
that finds every other one.
