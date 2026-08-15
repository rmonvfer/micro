# Providers and models

micro speaks six wire protocols across the services in its catalog, resolves a credential
for each, and decides which models exist from a catalog assembled in three layers.

## The providers

The bundled catalog carries thirty-five services. These are the ones most people reach for:

| Id | Name | Endpoint | Sign-in |
| --- | --- | --- | --- |
| `anthropic` | Anthropic | `api.anthropic.com` | API key |
| `openai` | OpenAI | `api.openai.com/v1` | API key |
| `google` | Google | `generativelanguage.googleapis.com/v1beta` | API key |
| `openrouter` | OpenRouter | `openrouter.ai/api/v1` | API key |
| `github-copilot` | GitHub Copilot | `api.individual.githubcopilot.com` | Device code |

The rest are of a piece with those: the platform endpoints (`amazon-bedrock`,
`google-vertex`, `azure-openai-responses`, `openai-codex`), the inference hosts (`groq`,
`cerebras`, `fireworks`, `together`, `nvidia`, `huggingface`), the gateways
(`vercel-ai-gateway`, `cloudflare-ai-gateway`), and the model makers who serve their own
(`deepseek`, `mistral`, `xai`, `moonshotai`, `zai`, `minimax`, and others). Every one of them
is signed into with a pasted key except GitHub Copilot, which uses the device-code flow.

Five clients serve the six protocols. `Anthropic` speaks the Messages API. `Gemini` speaks
Google's generative API, with a Vertex variant addressed under a project and a location.
`Bedrock` speaks Converse Stream, which is signed rather than keyed and answers in a binary
event stream rather than in server-sent events. `Codex` speaks the Responses protocol, with
variants for a subscription token and for Azure's deployment addressing. `OpenAi` speaks
chat completions, which is also what a model from a live listing is taken to speak when the
listing says nothing about it.

A client is chosen from the model rather than from its provider, because one service often
serves several protocols: GitHub Copilot answers Claude models over the Messages shape and
GPT models over the Responses shape. Sending a Responses model a completion instead would
work by accident and lose what the protocol is for — reasoning replayed between turns, which
is how a model keeps its thread across tool calls.

Common alternative spellings are folded onto the canonical id, so `claude` reaches
`anthropic`, `gemini` reaches `google`, `copilot` or `github` reach `github-copilot`, and
`codex` or `chatgpt` reach `openai-codex`. The canonical id is what the credential is filed
under and what a model's `provider` field holds.

## How credentials resolve

`AuthStore::resolve` prefers what is stored and falls back to the environment.

A credential stored by `micro auth login` lives in `auth.json` in micro's configuration
directory, one entry per provider, in a file only its owner can read. If the provider issues
short-lived tokens, the stored credential is exchanged for a fresh one at resolve time
rather than at login, so a long-idle installation still works without signing in again.

With nothing stored, the conventional environment variable is used instead:

| Provider | Variables, in order |
| --- | --- |
| `anthropic` | `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_TOKEN`, `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `google` | `GEMINI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |
| `github-copilot` | `COPILOT_GITHUB_TOKEN` |

A provider the table does not name falls back to the conventional `<PROVIDER>_API_KEY` —
`GROQ_API_KEY`, `DEEPSEEK_API_KEY`, and so on — which is also what an extension declaring a
provider of its own relies on.

`micro auth status` reports each provider's state and which of the two it came from, so a key
that is being shadowed by a stored credential is visible rather than mysterious.

## The model catalog

A model is more than an id. `micro-models` records the display name, the provider, the wire
API it speaks, its endpoint, its context window and output cap, whether it reasons, what
input it accepts, any headers it needs, and its price per million tokens for input, output,
cache reads, and cache writes. The prices are what the interface bases a running cost on.

The catalog is assembled in three layers, each overlaying the last.

**The bundled catalog** is compiled into the binary, so micro works offline with no setup.
It carries every service above along with the models each of them serves, prices included.

**A user catalog** at `models.json` in micro's configuration directory is applied over it.
An entry naming a model that already exists patches only the fields it mentions; an entry
naming a new one registers it. Provider-level settings re-point every model under that
provider at once, which is how a whole provider moves behind a proxy. See
[configuration.md](configuration.md) for the file's shape.

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

A service speaking a protocol micro already has needs no new code, which is most of them.
Give it an entry in the bundled catalog with its endpoint, the protocol it declares, and its
models, and give `micro-auth` its id, display name and environment variables so a credential
can be stored and resolved. A provider block naming no environment variable still resolves
one, from its own id.

A protocol micro does not have means implementing `Provider`: turn a `Model`, a `Context`,
and a key into a receiver of `StreamEvent`s, and answer separately with the body that
request would carry. The work is the translation at both edges — the request body the
endpoint expects, and its response framing back into events — and the two must agree, since
a session records a request by the hash of that body and rebuilds it from the same
assembly. `Anthropic`, `OpenAi`, `Codex`, `Gemini` and `Bedrock` are the worked examples,
and the registry picks between them from the model's declared protocol and its provider.

Nothing above needs to change. The agent loop and the tools never learn
that a provider was added, and a user reaches the new models through the same resolution
that finds every other one.
