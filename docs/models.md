# Image and classifier models

The catalog lists three types of model. Chat models hold conversations and are the only ones `/model` and `--model` offer. Image models generate images from a prompt and optional input images. Classifier models answer typed questions about a JSON state with probabilities. Extensions reach the last two through `ctx.modelRegistry`; their usage is billed to the session like any turn.

List each type from the command line:

```bash
micro models --type image
micro models --type classifier jev
```

## Image models

OpenRouter's image models, such as `google/gemini-2.5-flash-image` and `black-forest-labs/flux.2-pro`, are listed under the `openrouter` provider and use the same `OPENROUTER_API_KEY` or stored credential as its chat models. An upstream id that is both a chat and an image model has one entry of each type. `micro models --live` refreshes them with the rest of OpenRouter's listing.

```ts
export const capabilities = ["commands", "provider_stream"];

export default (micro) => {
  micro.registerCommand("paint", {
    async handler(prompt, ctx) {
      const painter = ctx.modelRegistry.findOfType("image", "openrouter", "google/gemini-2.5-flash-image");
      const result = await ctx.modelRegistry.generateImages(painter, {
        input: [{ type: "text", text: prompt }],
      });
      if (result.stopReason !== "stop") return result.errorMessage;
      return `${result.output.filter((block) => block.type === "image").length} images`;
    },
  });
};
```

`input` may also hold `{ type: "image", data, mimeType }` blocks to edit or follow. The result's `output` holds base64 image blocks and any text the model wrote; generated images are not saved to disk.

## Classifier models

micro carries TypeSafe's Jev from every service that serves it:

| Provider | Model ids | Authentication |
|---|---|---|
| `typesafe` | `jev-latest` | `TYPESAFE_API_KEY` |
| `openrouter` | `typesafe/jev-1.13`, `~typesafe/jev-latest` | `OPENROUTER_API_KEY` |
| `cloudflare-workers-ai` | `typesafe/jev` | `CLOUDFLARE_API_KEY` and `CLOUDFLARE_ACCOUNT_ID` |
| `vercel-ai-gateway` | `typesafe-ai/jev` | `AI_GATEWAY_API_KEY` |
| `opencode` | `jev-1.13`, `jev-1.13-free` | `OPENCODE_API_KEY` |

OpenRouter's other decision models are listed beside it, and every model on a [llama.cpp router](llama-cpp.md#classification) is also a classifier.

A request carries a JSON object `state` and named questions. A `choice` question picks one of its `criteria` keys, a `score` question rates on levels listed lowest first, and a `bool` question answers yes or no:

```ts
const jev = ctx.modelRegistry.findOfType("classifier", "typesafe", "jev-latest");
const result = await ctx.modelRegistry.classify(jev, {
  state: { message: "The change works, thanks." },
  questions: {
    approved: {
      type: "bool",
      instructions: "Does the user approve of the result?",
      criteria: { true: "Approval", false: "No approval" },
    },
  },
});
result.answers.approved.probability; // 0.97
```

A choice answer carries `choice`, every option's `probabilities` and a `confidence`; a score answer carries the expected `score` and a `confidence`; a bool answer carries the `probability` of yes. The optional third argument, `{ temperature }`, softens or sharpens label probabilities on classifiers that can apply it.

## The model registry

`ctx.modelRegistry` answers lookups from the catalog micro last reported: `find(provider, id)` for a chat model, `findOfType(type, provider, id)`, `getAll()`, `getAvailable()`, `getModelsOfType(type)`, `getAvailableOfType(type)`, `getAllModels()` and `getAllAvailable()`. A model is available when its provider has a credential. `refresh()` reads the catalog again after a sign-in.

`generateImages()` and `classify()` need the `provider_stream` capability. Each resolves the provider's credential when it is called, refreshing an expired token, and never hands it to the extension. A failed request resolves with `stopReason: "error"` and an `errorMessage`; a malformed request or an unknown model rejects.

## Usage and cost

When the service reports token counts, `result.usage` carries them with their cost at the model's catalog price. micro records each call in the session ledger as a `model_call` event naming the extension that made it, so it counts toward the footer's session cost, `/session` and `micro bill`. Models without a catalog price, such as TypeSafe's direct `jev-latest`, report tokens at no cost.

## From Rust

`micro_provider::ModelRuntime` is the same registry for code inside micro: `models_of_type`, `available_of_type`, `all_models`, `find_of_type`, `generate_images` and `classify`, each resolving credentials at call time. `micro_provider::model_call_event` turns a result's usage into the ledger event that bills it.
