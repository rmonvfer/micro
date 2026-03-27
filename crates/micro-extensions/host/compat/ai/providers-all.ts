// What `@earendil-works/pi-ai/providers/all` and `@mariozechner/pi-ai/providers/all`
// resolve to.
//
// pi-ai's real `providers/all.ts` builds every built-in provider (`anthropicProvider()`,
// `openaiProvider()`, and 37 more) against a generated model catalog and pi-ai's own
// HTTP+SSE API clients. `getBuiltinModel`/`getBuiltinModels`/`getBuiltinProviders` are
// genuinely synchronous there — they read an already-loaded in-memory catalog and return
// plain objects, not promises — so this file cannot route them through the wire the way
// `../compat.ts`'s streaming facade does: an extension that calls `getBuiltinModel()` at
// import time and gets a `Promise` back instead of a `Model` is broken in exactly the way
// this whole compatibility layer exists to avoid.
//
// Instead: `crates/micro-extensions/src/compat.rs`'s `install()` writes `../catalog.json`
// once per host start, serialized from `micro_models::Catalog::bundled()` — the same
// catalog `crates/micro-cli/src/extensions.rs`'s `model_catalog` wire arm reads for
// anything genuinely dynamic later. Imported here as static JSON, so `getBuiltinModel` and
// friends stay real, synchronous functions over real data, with no cache to warm and no
// window where they answer wrong.
//
// `builtinProviders()`/`builtinModels()` go one step further: each provider's `stream`/
// `streamSimple` dispatch to `../compat.ts`'s wire-facaded API factories (real network
// calls through `crates/micro-provider`), and `auth.apiKey` resolves through the same
// environment-variable mapping `../compat.ts`'s `getEnvApiKey` already carries — not a
// second, guessed-at copy of which env var each provider reads.
//
// Every relative import here goes up one directory: this file's source lives at
// `host/compat/ai/providers-all.ts`, but `crates/micro-extensions/src/compat.rs`'s
// `PI_AI_FILES` deploys it to `providers/all.ts` inside the package — one level below
// `index.ts`/`compat.ts`/`oauth.ts` and wherever `install()` writes `catalog.json`, the
// same nesting pi-ai's own real `providers/all.ts` has relative to its own package root.
import catalogData from "../catalog.json" with { type: "json" };
import {
	anthropicMessagesApi,
	azureOpenAIResponsesApi,
	bedrockConverseStreamApi,
	getEnvApiKey,
	googleGenerativeAIApi,
	googleVertexApi,
	openAICodexResponsesApi,
	openAICompletionsApi,
	openAIResponsesApi,
} from "../compat.ts";
import { type Api, type AuthContext, type AuthInteraction, type Model, type ProviderAuth, type ProviderStreams } from "../index.ts";
import type { CreateProviderOptions, Provider } from "../index.ts";
import { createProvider } from "../index.ts";

interface CatalogModelJson {
	id: string;
	name: string;
	provider: string;
	api: string;
	baseUrl: string;
	contextWindow: number;
	maxTokens: number;
	reasoning: boolean;
	input: string[];
	cost: { input: number; output: number; cacheRead: number; cacheWrite: number };
}

interface CatalogJson {
	providers: string[];
	models: CatalogModelJson[];
	/** Present once `install()` starts stamping it; read defensively since older writers
	 *  of this file might not have. */
	generatedAt?: string;
}

const catalog = catalogData as CatalogJson;

function toModel(entry: CatalogModelJson): Model<Api> {
	return {
		id: entry.id,
		name: entry.name,
		api: entry.api,
		provider: entry.provider,
		baseUrl: entry.baseUrl,
		reasoning: entry.reasoning,
		input: entry.input.filter((kind): kind is "text" | "image" => kind === "text" || kind === "image"),
		cost: entry.cost,
		contextWindow: entry.contextWindow,
		maxTokens: entry.maxTokens,
	};
}

const modelsByProvider = new Map<string, Model<Api>[]>();
for (const entry of catalog.models) {
	const list = modelsByProvider.get(entry.provider) ?? [];
	list.push(toModel(entry));
	modelsByProvider.set(entry.provider, list);
}

/** Providers present in the embedded catalog. pi-ai's own type additionally includes
 *  purely dynamic providers (like `"radius"`) with no static catalog entry; this shim has
 *  no equivalent to add, since everything it knows about a provider comes from this same
 *  static file. */
export type BuiltinProvider = string;

/** Typed read of the embedded built-in catalog. Matches pi-ai's real `getBuiltinModel`
 *  member for member: synchronous, `undefined` for an id the catalog doesn't carry. */
export function getBuiltinModel<TProvider extends BuiltinProvider>(provider: TProvider, modelId: string): Model<Api> | undefined {
	return modelsByProvider.get(provider)?.find((model) => model.id === modelId);
}

export function getBuiltinProviders(): BuiltinProvider[] {
	return [...catalog.providers];
}

/** Generation timestamp shared by every built-in provider catalog — when `install()`
 *  wrote `../catalog.json`, not when pi's own upstream catalog was generated (this shim
 *  carries no such thing; see `../index.ts`'s header for why). `undefined` until
 *  `install()` starts stamping it. */
export function getBuiltinModelDataGeneratedAt(): number | undefined {
	if (!catalog.generatedAt) return undefined;
	const generatedAt = Date.parse(catalog.generatedAt);
	return Number.isNaN(generatedAt) ? undefined : generatedAt;
}

export function getBuiltinModels<TProvider extends BuiltinProvider>(provider: TProvider): Model<Api>[] {
	return modelsByProvider.get(provider) ?? [];
}

/** The wire-facaded `ProviderStreams` implementation for each api id this catalog's
 *  models actually use — the same eight factories `../compat.ts` exports, keyed the way
 *  `createProvider`'s own api-map form expects. A model whose `api` isn't one of these
 *  (Mistral, pi-messages — see `../compat.ts`'s header) still gets a real, honest refusal
 *  through those factories' own `stream`/`streamSimple`, not a missing map entry here. */
const WIRE_APIS: Partial<Record<Api, ProviderStreams>> = {
	"anthropic-messages": anthropicMessagesApi(),
	"openai-completions": openAICompletionsApi(),
	"openai-responses": openAIResponsesApi(),
	"azure-openai-responses": azureOpenAIResponsesApi(),
	"openai-codex-responses": openAICodexResponsesApi(),
	"google-generative-ai": googleGenerativeAIApi(),
	"google-vertex": googleVertexApi(),
	"bedrock-converse-stream": bedrockConverseStreamApi(),
};

/** Real env-based auth for a builtin provider, through the exact provider-to-env-var
 *  mapping `../compat.ts`'s `getEnvApiKey` already carries (`ANTHROPIC_API_KEY`,
 *  `OPENAI_API_KEY`, Bedrock's ambient AWS credential chain, and the rest). No OAuth: pi-ai's
 *  own builtin providers each hardcode a real per-service OAuth dance (device codes,
 *  provider-specific token exchanges) this shim has no client library to run — a provider
 *  built from this file is reachable by env-configured API key, the same as
 *  `custom-provider-anthropic`'s own `CUSTOM_ANTHROPIC_API_KEY` path, not by `/login`. */
function envBackedAuth(providerId: string): ProviderAuth {
	return {
		apiKey: {
			name: providerId,
			async resolve({ signal }: { ctx: AuthContext; signal: AbortSignal }) {
				signal.throwIfAborted();
				const apiKey = getEnvApiKey(providerId);
				return apiKey ? { auth: { apiKey }, source: `${providerId} environment` } : undefined;
			},
			async login(interaction: AuthInteraction) {
				const key = await interaction.prompt({ type: "secret", message: `Enter an API key for ${providerId}` });
				return { type: "api_key" as const, key };
			},
		},
	};
}

/** All built-in providers, freshly constructed from the embedded catalog — real
 *  `getModels()`, real env-based auth, real streaming through `../compat.ts`'s wire
 *  facade for the eight api ids `crates/micro-provider` speaks. */
export function builtinProviders(): Provider[] {
	return getBuiltinProviders().map((providerId) => {
		const models = getBuiltinModels(providerId);
		const apiIds = [...new Set(models.map((model) => model.api))];
		const api: CreateProviderOptions["api"] =
			apiIds.length === 1 && WIRE_APIS[apiIds[0]] ? WIRE_APIS[apiIds[0]]! : (Object.fromEntries(apiIds.map((id) => [id, WIRE_APIS[id]])) as Partial<Record<Api, ProviderStreams>>);
		return createProvider({
			id: providerId,
			name: providerId,
			auth: envBackedAuth(providerId),
			models,
			api,
		});
	});
}

/** A read-only snapshot of every built-in provider's catalog and auth, without pi-ai's
 *  full `Models`/`MutableModels` registry (refresh scheduling, credential storage,
 *  login/logout orchestration) — see `../index.ts`'s header for why that registry itself
 *  isn't reproduced here. This is the part of it `builtinModels()` actually promises:
 *  "a Models collection with every built-in provider registered", read once rather than
 *  live. */
export function builtinModels(): { getProviders(): Provider[]; getProvider(id: string): Provider | undefined; getModel(provider: string, id: string): Model<Api> | undefined } {
	const providers = builtinProviders();
	const byId = new Map(providers.map((provider) => [provider.id, provider]));
	return {
		getProviders: () => providers,
		getProvider: (id) => byId.get(id),
		getModel: (provider, id) => getBuiltinModel(provider, id),
	};
}
