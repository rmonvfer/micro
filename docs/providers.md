# Providers and models

micro ships with a model catalog and clients for the wire protocols used by its supported providers. You can authenticate several providers and switch models without starting a new conversation.

## Common providers

| Provider ID      | Service         | Authentication                                          |
| ---------------- | --------------- | ------------------------------------------------------- |
| `anthropic`      | Anthropic       | Claude Pro/Max sign-in, API key, or identity federation |
| `openai`         | OpenAI          | Sign in with ChatGPT or API key                         |
| `openai-codex`   | OpenAI Codex    | ChatGPT Plus/Pro sign-in                                |
| `google`         | Google Gemini   | API key                                                 |
| `openrouter`     | OpenRouter      | OpenRouter sign-in or API key                           |
| `xai`            | xAI             | SuperGrok or X Premium sign-in, or API key              |
| `kimi-coding`    | Kimi For Coding | Kimi Code sign-in or API key                            |
| `github-copilot` | GitHub Copilot  | Device-code login or token                              |

The bundled catalog also includes cloud platform endpoints, inference hosts, model vendors, and gateways.

List providers and models known to your build with:

```bash
micro models
```

## Authenticate

```bash
micro auth login anthropic
micro auth login anthropic --method copy_code
micro auth login github-copilot
micro auth status
```

A provider that takes both an account sign-in and an API key asks which to use; `--method` answers in advance with `oauth`, `api_key`, `browser`, `copy_code` or `device_code`. Inside the interface, `/login` offers the same choices and labels each provider as `not configured`, or configured with an API key, a `subscription`, or an `account` (OpenRouter's sign-in is an account, not a subscription). `micro auth logout <provider>` and `/logout` remove a stored credential.

Browser sign-ins open the provider's page and wait for it to redirect to a listener on `127.0.0.1` (set `MICRO_OAUTH_CALLBACK_HOST` to listen elsewhere). When the browser runs on another machine, as over SSH, paste the final redirect URL or the code at the prompt instead. Anthropic's `copy_code` method is made for that case: Anthropic shows a `code#state` value to paste back. OpenAI Codex's `device_code` method and the xAI, Kimi Code and GitHub Copilot sign-ins show a code to enter on a page opened anywhere.

Sign in with ChatGPT registers this installation with OpenAI under a stable `device_id`, a UUID micro creates in the global config the first time it is needed. OpenRouter's sign-in yields a permanent API key that it stores like any other credential.

Expiring tokens are refreshed before a request needs them, while `auth.json` is locked, so two micro processes never spend the same rotating refresh token. Anthropic subscription requests are sent as Claude Code sends them; Anthropic bills that usage as extra usage rather than against plan limits.

Stored credentials are checked first. If none exists, micro checks environment variables:

| Provider         | Variables, in order                                                  |
| ---------------- | -------------------------------------------------------------------- |
| `anthropic`      | `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_TOKEN`, `ANTHROPIC_API_KEY` |
| `openai`         | `OPENAI_API_KEY`                                                     |
| `google`         | `GEMINI_API_KEY`                                                     |
| `openrouter`     | `OPENROUTER_API_KEY`                                                 |
| `github-copilot` | `COPILOT_GITHUB_TOKEN`                                               |

Other providers use the conventional `<PROVIDER>_API_KEY` name unless their catalog entry specifies another variable.

With no Anthropic key or token set, micro uses workload identity federation when `ANTHROPIC_FEDERATION_RULE_ID`, `ANTHROPIC_ORGANIZATION_ID` and `ANTHROPIC_IDENTITY_TOKEN_FILE` are all set. It exchanges the identity token for a short-lived access token and exchanges again shortly before that token expires, reading the identity token file each time, so keep the file fresh for long sessions. `ANTHROPIC_SERVICE_ACCOUNT_ID` and `ANTHROPIC_WORKSPACE_ID` are sent with the exchange when set.

## Use credentials from other programs

```bash
micro auth check anthropic
micro auth check --model sonnet --json
micro auth print-api-key openrouter
micro auth print-bearer-token openai-codex --min-expiry 1h
```

`micro auth check` takes a provider or a model and prints `ready`, `not_ready` or `invalid`, exiting with `0`, `1` or `2`. It refreshes an expired OAuth credential unless given `--no-refresh`; `--credentials` prints the resolved credential instead of the status, and `--json` writes the whole result. `print-api-key` prints a provider's API key, and `print-bearer-token` prints its OAuth access token after refreshing any token with less than `--min-expiry` (30 minutes by default) left. Both write secrets to standard output.

## Select a model

Use `-m` for one run or `/model` inside the interface:

```bash
micro -m opus "review this patch"
micro -m anthropic/claude-sonnet-5 "review this patch"
micro models sonnet
```

Resolution checks, in order:

1. provider-qualified ID;
2. exact model ID;
3. alias;
4. unique prefix;
5. unique substring of an ID or display name.

An ambiguous query prints the candidates. It is not resolved by ranking or guessing.

Provider qualification is useful when several services expose the same model:

```text
anthropic/claude-sonnet-5
openrouter/anthropic/claude-sonnet-5
```

## Live model listings

The bundled catalog works offline. Fetch current OpenRouter listings and, when authenticated, GitHub Copilot listings with:

```bash
micro models --live
```

Live data is merged over the bundled catalog. Fields omitted by the provider, such as aliases or prices, retain their catalog values. A listing that cannot be reached is reported without removing bundled models. Other providers do not implement live listing refresh.

If `live_models` or `MICRO_LIVE_MODELS` is enabled, micro refreshes provider listings before selecting a model at startup. If startup cannot resolve a model query from the local catalog, micro also tries the live listings before accepting an unknown model with unknown limits and pricing. Use `micro models --live` for an explicit refresh.

## Add a local or compatible endpoint

Add it to `models.json`:

```json
{
  "providers": {
    "ollama": {
      "base_url": "http://localhost:11434/v1",
      "api": "openai-completions",
      "models": [
        {
          "id": "qwen3-coder:30b",
          "name": "Qwen3 Coder 30B",
          "aliases": ["local"],
          "context_window": 32768
        }
      ]
    }
  }
}
```

Then run:

```bash
micro -m local "explain this crate"
```

Model entries can also provide prices for input, output, cache reads, and cache writes. `micro bill` uses those values with provider-reported usage.

## Supported protocols

The provider layer handles Anthropic Messages, OpenAI-compatible chat completions, OpenAI Responses, Google Generative AI, Vertex, and Amazon Bedrock Converse Stream.

The selected model determines the protocol. This matters for providers such as GitHub Copilot that expose different model families through different APIs.

Adding a service that already speaks a supported protocol normally requires catalog and authentication entries, not a new agent loop. See [Architecture](architecture.md) for the provider boundary.
