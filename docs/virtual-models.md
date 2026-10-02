# Virtual models

A virtual model is a selectable model that sends each request to a physical model of its choice. A router can send quick questions to a small model and hard problems to a large one while the user selects a single model.

Extensions register virtual models. They appear in `/model` and `--model` like any chat model, under whichever provider the extension names: one with physical models of its own, such as `openai-codex/auto`, or one that exists only for the virtual model. Under a provider with physical models, a virtual model is offered when that provider has a credential, and it hides a physical model with the same id.

## Register a virtual model

```ts
export const capabilities = ["providers"];

export default (micro) => {
  micro.registerVirtualModel({
    provider: "router",
    id: "auto",
    name: "Auto",
    thinkingLevels: ["low", "high"],
    route(request, ctx) {
      // Tool follow-ups and retries stay on the model that handled the turn.
      const sticky = request.failed ?? request.previous;
      if (request.reason !== "user" && sticky) {
        return { model: sticky.model, thinkingLevel: sticky.thinkingLevel ?? "medium" };
      }
      const id = request.thinkingLevel === "high" ? "claude-sonnet-5" : "claude-haiku-4-5-20251001";
      return { model: ctx.modelRegistry.find("anthropic", id), thinkingLevel: "medium" };
    },
  });
};
```

`thinkingLevels` lists the levels offered for selection and defaults to `["off"]`; what a level means is up to the router. `contextWindow` and `maxTokens` describe the model before its first response and default to 128,000 and 16,384 tokens. `input` defaults to text and images. Registering the same provider and id again replaces the model, and `unregisterVirtualModel(provider, id)` removes it. Registration needs the `providers` capability and happens while the extension loads.

## Route requests

`route(request, ctx)` runs before every request made with the virtual model and returns `{ model, thinkingLevel }`, where `model` is any physical chat model whose provider has a credential. The thinking level is clamped to what that model supports. If `route()` throws, or returns a virtual model or one without a credential, the request ends with an error response.

| Field | Meaning |
|---|---|
| `model`, `thinkingLevel` | The selected virtual model and level |
| `reason` | `user` for the first request after something the user wrote, `continuation` for any other request in the run, `retry` for an automatic retry after a failed request |
| `previous` | The physical model, and thinking level when known, of the latest successful response in `messages` |
| `failed` | For `retry`: the physical model, thinking level and the failed assistant `message`, with its `errorMessage` |
| `state` | Router state last kept on this session branch |
| `messages` | The conversation for this request, system prompt first |

Returning `previous` for a continuation and `failed` for a retry keeps prompt caches and thinking signatures valid. Switching models between turns is allowed but loses the prompt cache.

## Keep routing state

`route()` may return `state` next to the model. micro keeps it as a custom entry on the session branch and passes it back as `request.state` on later requests, so forks and `/tree` navigation see their own branch's state. Returning nothing, or `request.state` itself, keeps the current state; any other value is kept as new state before the request is sent. State must be JSON.

Routers can call other models through `ctx.modelRegistry`, for example a classifier from `findOfType("classifier", provider, id)` passed to `classify()`. The call adds latency before the first token of the turn and is billed to the session. [`jev-router.ts`](https://github.com/rmonvfer/micro/blob/main/examples/extensions/jev-router.ts) plans on a strong OpenAI Codex model chosen by the Jev classifier, lets it make the first edit, then switches once to a cheaper model, keeping the phase as router state.

## What is recorded

Each assistant message names the physical model that answered, so the transcript replays across physical models as it would after a manual switch. The footer shows the routed model after the selection, as `router/auto • high → anthropic/claude-sonnet-5`. `/session` and `micro bill` list cost per physical model, priced at that model's rates.
