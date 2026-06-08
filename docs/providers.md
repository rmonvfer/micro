# Providers and models

micro ships with a model catalog and clients for the wire protocols used by its supported providers. You can authenticate several providers and switch models without starting a new conversation.

## Common providers

| Provider ID      | Service        | Authentication                         |
| ---------------- | -------------- | -------------------------------------- |
| `anthropic`      | Anthropic      | API key or supported stored credential |
| `openai`         | OpenAI         | API key                                |
| `google`         | Google Gemini  | API key                                |
| `openrouter`     | OpenRouter     | API key                                |
| `github-copilot` | GitHub Copilot | Device-code login or token             |

The bundled catalog also includes cloud platform endpoints, inference hosts, model vendors, and gateways.

List providers and models known to your build with:

```bash
micro models
```

## Authenticate

```bash
micro auth login anthropic
micro auth login github-copilot
micro auth status
```

API-key logins read the key from the terminal. Device-code logins open the provider's browser flow. `micro auth logout <provider>` removes a stored credential.

Stored credentials are checked first. If none exists, micro checks environment variables:

| Provider         | Variables, in order                                                  |
| ---------------- | -------------------------------------------------------------------- |
| `anthropic`      | `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_TOKEN`, `ANTHROPIC_API_KEY` |
| `openai`         | `OPENAI_API_KEY`                                                     |
| `google`         | `GEMINI_API_KEY`                                                     |
| `openrouter`     | `OPENROUTER_API_KEY`                                                 |
| `github-copilot` | `COPILOT_GITHUB_TOKEN`                                               |

Other providers use the conventional `<PROVIDER>_API_KEY` name unless their catalog entry specifies another variable.

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

The bundled catalog works offline. Fetch current listings from configured providers with:

```bash
micro models --live
```

Live data is merged over the bundled catalog. Fields omitted by the provider, such as aliases or prices, retain their catalog values. A provider that cannot be reached is reported without removing its bundled models.

Set `live_models` in `config.json` to perform this merge at startup.

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

The provider layer currently handles Anthropic Messages, OpenAI-compatible chat completions, OpenAI Responses, Google Generative AI, Vertex, and Amazon Bedrock Converse Stream.

The selected model determines the protocol. This matters for providers such as GitHub Copilot that expose different model families through different APIs.

Adding a service that already speaks a supported protocol normally requires catalog and authentication entries, not a new agent loop. See [Architecture](architecture.md) for the provider boundary.
