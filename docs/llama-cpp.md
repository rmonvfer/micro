# Local models with llama.cpp

micro works with the [llama.cpp](https://github.com/ggml-org/llama.cpp) router: a `llama-server` started without a model, which discovers GGUF files and loads or unloads them on request. Its loaded models are chat models under the `llama.cpp` provider, and each is also a [classifier](#classification).

## Start the router

Start `llama-server` without `--model`, `-m` or `-hf`; any of them starts single-model mode instead.

```bash
llama-server --models-dir ~/models --no-models-autoload --jinja \
  --host 127.0.0.1 --port 8080 -ngl 999 -c 32768
```

`--models-dir` is where GGUF files are found: a single-file model can sit directly in it, while multimodal and multi-shard models go in subdirectories of their own. `--no-models-autoload` keeps loading explicit, `--jinja` enables chat templates and tool calling, and `-c` sets each loaded model's context window. Restart the router after adding files by hand.

## Connect micro

```bash
micro llama connect http://127.0.0.1:8080
```

`connect` checks that the address answers as a router and remembers it in `llama-cpp.json` in micro's configuration directory. Pass `--api-key` when the router was started with `--api-key`; the key is stored as the `llama.cpp` credential. `LLAMA_BASE_URL` and `LLAMA_API_KEY` configure the same without storing anything and take precedence.

## Manage models

```bash
micro llama status
micro llama search qwen3
micro llama search unsloth/Qwen3-4B-GGUF
micro llama download unsloth/Qwen3-4B-GGUF:Q4_K_M
micro llama load Qwen3-4B-GGUF --unload-others
micro llama unload Qwen3-4B-GGUF
```

`status` lists the router's models and their state. `search` lists GGUF repositories on Hugging Face, or the quantizations of one repository, `Q4_K_M` first and the rest smallest first. `download` has the router fetch a repository and shows the bytes as they arrive; it warns about gated repositories, which need the router's own process to run with an `HF_TOKEN` that has access. Searching uses `HF_TOKEN`, then `$HF_TOKEN_PATH`, `$HF_HOME/token`, `$XDG_CACHE_HOME/huggingface/token` and `~/.cache/huggingface/token`, and works without one at lower rate limits.

`load` waits until the model is serving. Other loaded models stay loaded unless `--unload-others` is passed; micro never unloads a model unasked and never deletes model files. The router may be shared with other clients, so every command reads its current state.

Loaded and sleeping models appear in `/model` and `micro models`; a sleeping model wakes when a request reaches it. With router autoload on, unloaded preset models appear too and load on first use. micro asks the router for its models each time it starts.

## Context windows

A loaded model reports the context window it runs with. micro remembers it in `llama-cpp-context.json` in its data directory, so the same model is described by that window rather than its training context while it is unloaded or asleep. A model started with `-c` is described by that value; otherwise the training context applies, and 128,000 tokens when the router says nothing.

## Classification

Every model offered for chat is also a classifier model with the same id. The model never generates an answer: each question becomes one prompt holding the state, every question of the request, the state again, and then the question with its answers under single-token labels, letters for a choice (up to 62 options), `Yes` and `No` for a bool, and digits for a score (up to 10 levels). micro reads the probabilities of the labels as the next token from `/completion` and normalizes them. A choice returns every option's probability and a confidence of `(n * peak - 1) / (n - 1)`; a score returns the expected level.

Questions run one after another, and everything before the final question is the same for all of them, so the router's prompt cache evaluates it once. The `temperature` option divides label logits before normalizing; values above 1 soften overconfident small models without changing any answer. Small models may follow instructions written inside the state despite the prompt telling them not to. Hybrid models such as Qwen3.5 need `--ctx-checkpoints 32 --checkpoint-min-step 0` to avoid reprocessing the whole state for every question.

## Troubleshooting

`curl http://127.0.0.1:8080/models` shows what the router knows. No models usually means `--models-dir` or the directory layout is wrong; a model missing from `/model` under `--no-models-autoload` needs `micro llama load` first; a load that fails or exhausts memory needs a smaller `-c` or another model unloaded. "Server is not running in llama.cpp router mode" means it was started with a model.
