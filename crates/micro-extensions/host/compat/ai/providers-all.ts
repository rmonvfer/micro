
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
	/**
 * Present once `install()` starts stamping it; read defensively since older writers of this file
 * might not have.
 */
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

/** Providers present in the embedded catalog. */
export type BuiltinProvider = string;

/** Typed read of the embedded built-in catalog. */
export function getBuiltinModel<TProvider extends BuiltinProvider>(provider: TProvider, modelId: string): Model<Api> | undefined {
	return modelsByProvider.get(provider)?.find((model) => model.id === modelId);
}

export function getBuiltinProviders(): BuiltinProvider[] {
	return [...catalog.providers];
}

/** Generation timestamp shared by every built-in provider catalog. */
export function getBuiltinModelDataGeneratedAt(): number | undefined {
	if (!catalog.generatedAt) return undefined;
	const generatedAt = Date.parse(catalog.generatedAt);
	return Number.isNaN(generatedAt) ? undefined : generatedAt;
}

export function getBuiltinModels<TProvider extends BuiltinProvider>(provider: TProvider): Model<Api>[] {
	return modelsByProvider.get(provider) ?? [];
}

/**
 * The wire-facaded `ProviderStreams` implementation for each api id this catalog's models actually
 * use.
 */
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

/** All built-in providers, freshly constructed from the embedded catalog. */
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


export function builtinModels(): { getProviders(): Provider[]; getProvider(id: string): Provider | undefined; getModel(provider: string, id: string): Model<Api> | undefined } {
	const providers = builtinProviders();
	const byId = new Map(providers.map((provider) => [provider.id, provider]));
	return {
		getProviders: () => providers,
		getProvider: (id) => byId.get(id),
		getModel: (provider, id) => getBuiltinModel(provider, id),
	};
}
