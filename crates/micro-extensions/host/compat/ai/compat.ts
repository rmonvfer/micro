// What `@earendil-works/pi-ai/compat` and `@mariozechner/pi-ai/compat` resolve to.
//
// pi-ai's own `compat.ts` re-exports everything `index.ts` exports (already in `./index.ts`
// here) plus the api-registry and env-key-aware `stream`/`complete`/`streamSimple`/
// `completeSimple` dispatch. What's real below is exactly that registry and dispatch — an
// extension that calls `registerApiProvider({ api: "my-proxy", stream, streamSimple }, ...)`
// and then `stream(model, context)` gets the real thing: the registry holds its entry,
// dispatch finds it, env API keys are injected the same way pi-ai's own `stream()` injects
// them. This is exactly the shape `custom-provider-gitlab-duo/index.ts` (pi's own example
// of registering a complete provider) uses: an extension that already holds its own
// credential — a `GITLAB_TOKEN` env var, a manually-run OAuth flow — and wants pi-ai's
// dispatch machinery around its own HTTP calls. Nothing here resolves *this host's*
// credentials to do that; the registry only ever runs an implementation the extension
// itself supplied.
//
// pi-ai/compat also pre-registers ten builtin API ids (`anthropic-messages`,
// `openai-completions`, ...), each backed by an HTTP+SSE client `./index.ts`'s header
// documents as too large to vendor (`@anthropic-ai/sdk`, `openai`, thousands of lines of
// wire-format logic). Eight of the ten are real anyway, further down this file
// (`anthropicMessagesApi`, `openAICompletionsApi`, `openAIResponsesApi`,
// `azureOpenAIResponsesApi`, `openAICodexResponsesApi`, `googleGenerativeAIApi`,
// `googleVertexApi`, `bedrockConverseStreamApi`) — not by vendoring pi-ai's own client, but
// by facading to `crates/micro-provider`, which already speaks these same wire protocols
// for micro's own agent loop. `mistralConversationsApi` and `piMessagesApi` have no
// `micro_models::WireApi` counterpart at all (Mistral's Conversations API and pi's own
// internal message format are protocols micro-provider never learned), so those two exist
// as named exports — an import of them still succeeds — whose `stream`/`streamSimple`
// throw a specific, named reason the moment either is actually called.
//
// `getModel`/`getModels`/`getProviders` (deprecated static catalog reads backed by pi-ai's
// generated model data) and `registerFauxProvider` (an in-process fake provider for pi's
// own test suite) are left out entirely rather than faked.

import {
	type Api,
	type AssistantMessage,
	type AssistantMessageEvent,
	type AssistantMessageEventStream,
	type Context,
	createAssistantMessageEventStream,
	type Model,
	ModelsError,
	type ProviderStreams,
	type SimpleStreamOptions,
	type StreamOptions,
} from "./index.ts";

export * from "./index.ts";

// ---------------------------------------------------------------------------------------
// env-api-keys.ts, inlined rather than a sibling file: `crates/micro-extensions/src/
// compat.rs`'s `PI_AI_FILES` list is what actually gets written to the node_modules
// tree an extension resolves against (see that file's own `install()`), and a second file
// here would need a change there to ever be read. Pure `process.env`/filesystem-existence
// logic running in this host's own Bun process — it never crosses into Rust and never
// touches a credential store, so it carries none of the concerns `../../context.ts` raises
// about keeping `ctx.modelRegistry` (a live credential-resolving object) off the wire.
// ---------------------------------------------------------------------------------------

let procEnvCache: Map<string, string> | null = null;

/** Fallback for a Bun issue where compiled binaries can expose an empty `process.env`
 *  inside Linux sandboxes even though `/proc/self/environ` has the real environment. */
function getBunSandboxEnvValue(name: string): string | undefined {
	if (typeof process === "undefined" || !process.versions?.bun || Object.keys(process.env).length > 0) {
		return undefined;
	}

	if (procEnvCache === null) {
		procEnvCache = new Map();
		try {
			const { readFileSync } = require("node:fs") as { readFileSync(path: string, encoding: BufferEncoding): string };
			const data = readFileSync("/proc/self/environ", "utf-8");
			for (const entry of data.split("\0")) {
				const idx = entry.indexOf("=");
				if (idx > 0) {
					procEnvCache.set(entry.slice(0, idx), entry.slice(idx + 1));
				}
			}
		} catch {
			// /proc/self/environ may not exist or may not be readable.
		}
	}

	return procEnvCache.get(name);
}

/** Resolve a provider env value from scoped overrides, then normal `process.env`, then
 *  the Bun sandbox fallback above. */
function getProviderEnvValue(name: string, env?: Record<string, string>): string | undefined {
	return env?.[name] || (typeof process !== "undefined" ? process.env[name] : undefined) || getBunSandboxEnvValue(name) || undefined;
}

const ANTHROPIC_AUTH_TOKEN_ENV = "ANTHROPIC_AUTH_TOKEN";
const ANTHROPIC_OAUTH_TOKEN_ENV = "ANTHROPIC_OAUTH_TOKEN";
const ANTHROPIC_API_KEY_ENV = "ANTHROPIC_API_KEY";

let cachedVertexAdcCredentialsExists: boolean | null = null;

function hasVertexAdcCredentials(env?: Record<string, string>): boolean {
	const explicitCredentialsPath = env?.GOOGLE_APPLICATION_CREDENTIALS;
	const fs = require("node:fs") as typeof import("node:fs");
	if (explicitCredentialsPath) {
		return fs.existsSync(explicitCredentialsPath);
	}

	if (cachedVertexAdcCredentialsExists === null) {
		const gacPath = getProviderEnvValue("GOOGLE_APPLICATION_CREDENTIALS", env);
		if (gacPath) {
			cachedVertexAdcCredentialsExists = fs.existsSync(gacPath);
		} else {
			const path = require("node:path") as typeof import("node:path");
			const os = require("node:os") as typeof import("node:os");
			cachedVertexAdcCredentialsExists = fs.existsSync(path.join(os.homedir(), ".config", "gcloud", "application_default_credentials.json"));
		}
	}
	return cachedVertexAdcCredentialsExists;
}

function getApiKeyEnvVars(provider: string): readonly string[] | undefined {
	if (provider === "github-copilot") {
		return ["COPILOT_GITHUB_TOKEN"];
	}

	// ANTHROPIC_AUTH_TOKEN participates in env discovery/status, but getEnvApiKey() skips
	// it because requests must pass it as Authorization: Bearer.
	if (provider === "anthropic") {
		return [ANTHROPIC_AUTH_TOKEN_ENV, ANTHROPIC_OAUTH_TOKEN_ENV, ANTHROPIC_API_KEY_ENV];
	}

	const envMap: Record<string, string> = {
		"ant-ling": "ANT_LING_API_KEY",
		"qwen-token-plan": "QWEN_TOKEN_PLAN_API_KEY",
		"qwen-token-plan-cn": "QWEN_TOKEN_PLAN_CN_API_KEY",
		"qwen-token-plan-individual": "QWEN_TOKEN_PLAN_API_KEY",
		openai: "OPENAI_API_KEY",
		"azure-openai-responses": "AZURE_OPENAI_API_KEY",
		nvidia: "NVIDIA_API_KEY",
		deepseek: "DEEPSEEK_API_KEY",
		google: "GEMINI_API_KEY",
		"google-vertex": "GOOGLE_CLOUD_API_KEY",
		groq: "GROQ_API_KEY",
		cerebras: "CEREBRAS_API_KEY",
		xai: "XAI_API_KEY",
		radius: "RADIUS_API_KEY",
		openrouter: "OPENROUTER_API_KEY",
		"vercel-ai-gateway": "AI_GATEWAY_API_KEY",
		zai: "ZAI_API_KEY",
		"zai-coding-cn": "ZAI_CODING_CN_API_KEY",
		mistral: "MISTRAL_API_KEY",
		minimax: "MINIMAX_API_KEY",
		"minimax-cn": "MINIMAX_CN_API_KEY",
		moonshotai: "MOONSHOT_API_KEY",
		"moonshotai-cn": "MOONSHOT_API_KEY",
		huggingface: "HF_TOKEN",
		fireworks: "FIREWORKS_API_KEY",
		together: "TOGETHER_API_KEY",
		baseten: "BASETEN_API_KEY",
		opencode: "OPENCODE_API_KEY",
		"opencode-go": "OPENCODE_API_KEY",
		"kimi-coding": "KIMI_API_KEY",
		"cloudflare-workers-ai": "CLOUDFLARE_API_KEY",
		"cloudflare-ai-gateway": "CLOUDFLARE_API_KEY",
		xiaomi: "XIAOMI_API_KEY",
		"xiaomi-token-plan-cn": "XIAOMI_TOKEN_PLAN_CN_API_KEY",
		"xiaomi-token-plan-ams": "XIAOMI_TOKEN_PLAN_AMS_API_KEY",
		"xiaomi-token-plan-sgp": "XIAOMI_TOKEN_PLAN_SGP_API_KEY",
	};

	const envVar = envMap[provider];
	return envVar ? [envVar] : undefined;
}

/**
 * Find configured environment variables that can provide an API key for a provider. This
 * only reports actual API key variables — it intentionally excludes ambient credential
 * sources such as AWS profiles, AWS IAM credentials, and Google Application Default
 * Credentials.
 */
function findEnvKeys(provider: string, env?: Record<string, string>): string[] | undefined {
	const envVars = getApiKeyEnvVars(provider);
	if (!envVars) return undefined;

	const found = envVars.filter((envVar) => !!getProviderEnvValue(envVar, env));
	return found.length > 0 ? found : undefined;
}

/** Get the API key for a provider from known environment variables, e.g. `OPENAI_API_KEY`.
 *  Never returns a key for a provider that requires an OAuth token instead. */
/** Exported for `./providers-all.ts`'s `builtinProviders()`, which needs the same
 *  provider-to-env-var mapping to build real auth for the builtin catalog rather than a
 *  second, independently-maintained copy of it. */
export function getEnvApiKey(provider: string, env?: Record<string, string>): string | undefined {
	const envKeys = findEnvKeys(provider, env);
	if (envKeys?.[0]) {
		const apiKeyEnv = provider === "anthropic" ? envKeys.find((key) => key !== ANTHROPIC_AUTH_TOKEN_ENV) : envKeys[0];
		if (apiKeyEnv) return getProviderEnvValue(apiKeyEnv, env);
	}

	// Vertex AI supports either an explicit API key or Application Default Credentials,
	// configured via `gcloud auth application-default login`.
	if (provider === "google-vertex") {
		const hasCredentials = hasVertexAdcCredentials(env);
		const hasProject = !!(getProviderEnvValue("GOOGLE_CLOUD_PROJECT", env) || getProviderEnvValue("GCLOUD_PROJECT", env));
		const hasLocation = !!getProviderEnvValue("GOOGLE_CLOUD_LOCATION", env);

		if (hasCredentials && hasProject && hasLocation) {
			return "<authenticated>";
		}
	}

	if (provider === "amazon-bedrock") {
		// Amazon Bedrock supports multiple credential sources: AWS_PROFILE, IAM keys,
		// a Bedrock bearer token, ECS task roles, or IRSA — all ambient, none read here.
		if (
			getProviderEnvValue("AWS_PROFILE", env) ||
			(getProviderEnvValue("AWS_ACCESS_KEY_ID", env) && getProviderEnvValue("AWS_SECRET_ACCESS_KEY", env)) ||
			getProviderEnvValue("AWS_BEARER_TOKEN_BEDROCK", env) ||
			getProviderEnvValue("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", env) ||
			getProviderEnvValue("AWS_CONTAINER_CREDENTIALS_FULL_URI", env) ||
			getProviderEnvValue("AWS_WEB_IDENTITY_TOKEN_FILE", env)
		) {
			return "<authenticated>";
		}
	}

	return undefined;
}

export type ApiStreamFunction = (model: Model<Api>, context: Context, options?: StreamOptions) => AssistantMessageEventStream;

export type ApiStreamSimpleFunction = (
	model: Model<Api>,
	context: Context,
	options?: SimpleStreamOptions,
) => AssistantMessageEventStream;

export interface ApiProvider<TApi extends Api = Api, TOptions extends StreamOptions = StreamOptions> {
	api: TApi;
	stream: (model: Model<TApi>, context: Context, options?: TOptions) => AssistantMessageEventStream;
	streamSimple: (model: Model<TApi>, context: Context, options?: SimpleStreamOptions) => AssistantMessageEventStream;
}

interface ApiProviderInternal {
	api: Api;
	stream: ApiStreamFunction;
	streamSimple: ApiStreamSimpleFunction;
}

type RegisteredApiProvider = { provider: ApiProviderInternal; sourceId?: string };

const apiProviderRegistry = new Map<string, RegisteredApiProvider>();

function wrapStream<TApi extends Api, TOptions extends StreamOptions>(
	api: TApi,
	stream: ApiProvider<TApi, TOptions>["stream"],
): ApiStreamFunction {
	return (model, context, options) => {
		if (model.api !== api) {
			throw new Error(`Mismatched api: ${model.api} expected ${api}`);
		}
		return stream(model as Model<TApi>, context, options as TOptions);
	};
}

function wrapStreamSimple<TApi extends Api>(api: TApi, streamSimple: ApiProvider<TApi>["streamSimple"]): ApiStreamSimpleFunction {
	return (model, context, options) => {
		if (model.api !== api) {
			throw new Error(`Mismatched api: ${model.api} expected ${api}`);
		}
		return streamSimple(model as Model<TApi>, context, options);
	};
}

/** Register an API implementation under `provider.api`. `sourceId`, when given, lets a
 *  batch of registrations from the same source be torn down together later with
 *  `unregisterApiProviders`. */
export function registerApiProvider<TApi extends Api, TOptions extends StreamOptions>(
	provider: ApiProvider<TApi, TOptions>,
	sourceId?: string,
): void {
	apiProviderRegistry.set(provider.api, {
		provider: {
			api: provider.api,
			stream: wrapStream(provider.api, provider.stream),
			streamSimple: wrapStreamSimple(provider.api, provider.streamSimple),
		},
		sourceId,
	});
}

export function getApiProvider(api: Api): ApiProviderInternal | undefined {
	return apiProviderRegistry.get(api)?.provider;
}

export function getApiProviders(): ApiProviderInternal[] {
	return Array.from(apiProviderRegistry.values(), (entry) => entry.provider);
}

export function unregisterApiProviders(sourceId: string): void {
	for (const [api, entry] of apiProviderRegistry.entries()) {
		if (entry.sourceId === sourceId) {
			apiProviderRegistry.delete(api);
		}
	}
}

function resolveApiProvider(api: Api): ApiProviderInternal {
	const provider = getApiProvider(api);
	if (!provider) {
		throw new ModelsError("stream", `No API provider registered for api: ${api}`);
	}
	return provider;
}

const AMBIENT_AUTH_MARKER = "<authenticated>";

function hasExplicitApiKey(apiKey: string | undefined): apiKey is string {
	return typeof apiKey === "string" && apiKey.trim().length > 0;
}

function withEnvApiKey<TOptions extends StreamOptions>(model: Model<Api>, options: TOptions | undefined): TOptions | undefined {
	if (hasExplicitApiKey(options?.apiKey)) return options;
	const apiKey = getEnvApiKey(model.provider, options?.env);
	if (!apiKey || apiKey === AMBIENT_AUTH_MARKER) return options;
	return { ...options, apiKey } as TOptions;
}

/** Stream a model through whatever `ApiProvider` is registered for `model.api`. Env API
 *  keys are injected the same way pi-ai's own `stream()` injects them; there is no
 *  builtin-provider fast path here since no builtin API implementation is registered — see
 *  the file header. */
export function stream<TApi extends Api>(model: Model<TApi>, context: Context, options?: StreamOptions): AssistantMessageEventStream {
	const provider = resolveApiProvider(model.api);
	return provider.stream(model, context, withEnvApiKey(model, options));
}

export async function complete<TApi extends Api>(model: Model<TApi>, context: Context, options?: StreamOptions): Promise<AssistantMessage> {
	const s = stream(model, context, options);
	return s.result();
}

export function streamSimple<TApi extends Api>(
	model: Model<TApi>,
	context: Context,
	options?: SimpleStreamOptions,
): AssistantMessageEventStream {
	const provider = resolveApiProvider(model.api);
	return provider.streamSimple(model, context, withEnvApiKey(model, options));
}

export async function completeSimple<TApi extends Api>(
	model: Model<TApi>,
	context: Context,
	options?: SimpleStreamOptions,
): Promise<AssistantMessage> {
	const s = streamSimple(model, context, options);
	return s.result();
}

// ---------------------------------------------------------------------------------------
// The eight provider API factories that facade to `crates/micro-provider` — real network
// calls, run by Rust, over the same wire every other shim module reaches micro through.
//
// `globalThis.__MICRO_WIRE__` is published once by `host/wire.ts`, right after this
// process starts, before any extension is imported — see that file's own comment on why
// `globalThis` rather than a shared package: a compat shim resolves through its own
// `NODE_PATH` node_modules tree, a separate module instance from whatever `host.ts`
// resolves internally, so a "shared" package would risk two independent copies of the
// request/response bookkeeping rather than one. `ask`/`send` here are the exact functions
// `host-wire.ts` uses for every other request this host makes — not reimplemented, not
// wrapped a second time.
//
// This file declares the ambient global itself rather than importing `host/wire.ts`'s own
// declaration: this shim resolves through its own physical `node_modules` tree (see
// `crates/micro-extensions/src/compat.rs`'s `install()`), so nothing on its module
// resolution path leads back to that file — `globalThis` is the one thing both sides
// share at runtime, not at the type-checker's compilation-unit level.
// ---------------------------------------------------------------------------------------

declare global {
	// eslint-disable-next-line no-var
	var __MICRO_WIRE__: { ask(request: Record<string, unknown>): Promise<Record<string, unknown>>; send(message: Record<string, unknown>): void } | undefined;
}

function wire(): { ask(request: Record<string, unknown>): Promise<Record<string, unknown>> } {
	const published = globalThis.__MICRO_WIRE__;
	if (!published) {
		throw new Error(
			"no wire to ask a provider through — this code is running outside micro's extension host, where globalThis.__MICRO_WIRE__ is never published",
		);
	}
	return published;
}

function emptyUsage() {
	return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
}

/** The `AssistantMessage` a stream ends with when nothing was ever heard back at all —
 *  the wire itself refused, rather than the request reaching micro-provider and failing
 *  there (which already comes back as its own `error` event, translated on the Rust side). */
function setupErrorMessage(model: Model<Api>, error: unknown): AssistantMessage {
	return {
		role: "assistant",
		content: [],
		api: model.api,
		provider: model.provider,
		model: model.id,
		usage: emptyUsage(),
		stopReason: "error",
		errorMessage: error instanceof Error ? error.message : String(error),
		timestamp: Date.now(),
	};
}

/** Run one request against whichever provider client micro-provider has for `apiId`, and
 *  replay the ordered `AssistantMessageEvent` sequence Rust collected into a fresh
 *  `AssistantMessageEventStream` — see `crates/micro-cli/src/extensions.rs`'s
 *  `provider_stream`/`drain_provider_stream` for the Rust side of this same contract.
 *  `stream` and `streamSimple` are the same function here: the distinction between them in
 *  pi-ai's own client implementations is an internal optimization (prompt-cache shaping,
 *  mainly) that has no counterpart to preserve on this side of a facade — both send the
 *  same request and read back the same events. */
function wireProviderStream(
	apiId: Api,
	model: Model<Api>,
	context: Context,
	options: StreamOptions | SimpleStreamOptions | undefined,
): AssistantMessageEventStream {
	const stream = createAssistantMessageEventStream();

	if (options?.signal?.aborted) {
		const message = setupErrorMessage(model, new Error("Request was aborted"));
		message.stopReason = "aborted";
		stream.push({ type: "error", reason: "aborted", error: message });
		stream.end();
		return stream;
	}

	(async () => {
		const response = await wire().ask({
			type: "request",
			request: "provider_stream",
			api: apiId,
			model: {
				id: model.id,
				provider: model.provider,
				baseUrl: model.baseUrl,
				maxTokens: (options as { maxTokens?: number } | undefined)?.maxTokens ?? model.maxTokens,
				reasoning: model.reasoning,
				thinkingLevel: (options as { reasoning?: string } | undefined)?.reasoning ?? "off",
				headers: { ...model.headers, ...options?.headers },
			},
			context: {
				systemPrompt: context.systemPrompt,
				messages: context.messages,
				tools: context.tools ?? [],
			},
			apiKey: options?.apiKey ?? "",
		});

		if (typeof response.error === "string") {
			throw new Error(response.error);
		}
		const events = (response.events as AssistantMessageEvent[] | undefined) ?? [];
		for (const event of events) {
			stream.push(event);
		}
	})()
		.catch((error) => {
			stream.push({ type: "error", reason: "error", error: setupErrorMessage(model, error) });
		})
		.finally(() => stream.end());

	return stream;
}

function wireProviderApi(apiId: Api): ProviderStreams {
	return {
		api: apiId,
		stream: (model, context, options) => wireProviderStream(apiId, model, context, options),
		streamSimple: (model, context, options) => wireProviderStream(apiId, model, context, options),
	};
}

function unsupportedProviderApi(apiId: Api, why: string): ProviderStreams {
	const refuse = (): never => {
		throw new Error(`pi-ai's ${apiId} has no micro-provider client behind it — ${why}`);
	};
	return { api: apiId, stream: refuse, streamSimple: refuse };
}

/** Anthropic's Messages API, facaded to `crates/micro-provider`'s real Anthropic client.
 *  The `custom-provider-gitlab-duo` example calls this directly. */
export function anthropicMessagesApi(): ProviderStreams {
	return wireProviderApi("anthropic-messages");
}

/** OpenAI's Chat Completions API, facaded to `crates/micro-provider`'s real OpenAI client. */
export function openAICompletionsApi(): ProviderStreams {
	return wireProviderApi("openai-completions");
}

/** OpenAI's Responses API, facaded to `crates/micro-provider`'s real client for it. The
 *  `custom-provider-gitlab-duo` example calls this directly. */
export function openAIResponsesApi(): ProviderStreams {
	return wireProviderApi("openai-responses");
}

/** Azure's hosting of the Responses protocol — the same wire shape as `openAIResponsesApi`,
 *  reached with Azure's own headers; `crates/micro-provider::client_for` tells the two
 *  apart by `model.provider`, so nothing here needs to. */
export function azureOpenAIResponsesApi(): ProviderStreams {
	return wireProviderApi("azure-openai-responses");
}

/** OpenAI Codex's Responses protocol, reached with a subscription token rather than a
 *  platform key — again told apart from the plain Responses case by `model.provider` on
 *  the Rust side. */
export function openAICodexResponsesApi(): ProviderStreams {
	return wireProviderApi("openai-codex-responses");
}

/** Google's Generative AI API, facaded to `crates/micro-provider`'s real Gemini client. */
export function googleGenerativeAIApi(): ProviderStreams {
	return wireProviderApi("google-generative-ai");
}

/** Google Vertex AI's hosting of the Gemini shape, facaded the same way. */
export function googleVertexApi(): ProviderStreams {
	return wireProviderApi("google-vertex");
}

/** Amazon Bedrock's Converse Stream API, facaded to `crates/micro-provider`'s real,
 *  SigV4-signed Bedrock client. */
export function bedrockConverseStreamApi(): ProviderStreams {
	return wireProviderApi("bedrock-converse-stream");
}

/** Mistral's Conversations API. `crates/micro-models::WireApi` has no variant for it —
 *  micro-provider has never spoken this protocol — so this exists as a real named export
 *  (an extension's `import { mistralConversationsApi } from "@earendil-works/pi-ai/compat"`
 *  succeeds) whose `stream`/`streamSimple` refuse, by name, the moment either is called. */
export function mistralConversationsApi(): ProviderStreams {
	return unsupportedProviderApi("mistral-conversations", "micro-provider has never implemented Mistral's Conversations API");
}

/** pi's own internal message-relay format, used between pi's own processes rather than
 *  spoken by any model provider directly — nothing on micro's side, or pi-ai's own
 *  `providers/all.ts`, has ever had a real client for this either. */
export function piMessagesApi(): ProviderStreams {
	return unsupportedProviderApi("pi-messages", "this is pi's own internal relay format, not a protocol any model provider speaks");
}
