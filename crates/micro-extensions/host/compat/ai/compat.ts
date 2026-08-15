

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



let procEnvCache: Map<string, string> | null = null;


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
			
		}
	}

	return procEnvCache.get(name);
}

/**
 * Resolve a provider env value from scoped overrides, then normal `process.env`, then the Bun
 * sandbox fallback above.
 */
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

/** Find configured environment variables that can provide an API key for a provider. */
function findEnvKeys(provider: string, env?: Record<string, string>): string[] | undefined {
	const envVars = getApiKeyEnvVars(provider);
	if (!envVars) return undefined;

	const found = envVars.filter((envVar) => !!getProviderEnvValue(envVar, env));
	return found.length > 0 ? found : undefined;
}

/** Get the API key for a provider from known environment variables, e.g. */
/** Exported for `./providers-all.ts`'s `builtinProviders()`. */
export function getEnvApiKey(provider: string, env?: Record<string, string>): string | undefined {
	const envKeys = findEnvKeys(provider, env);
	if (envKeys?.[0]) {
		const apiKeyEnv = provider === "anthropic" ? envKeys.find((key) => key !== ANTHROPIC_AUTH_TOKEN_ENV) : envKeys[0];
		if (apiKeyEnv) return getProviderEnvValue(apiKeyEnv, env);
	}

	
	if (provider === "google-vertex") {
		const hasCredentials = hasVertexAdcCredentials(env);
		const hasProject = !!(getProviderEnvValue("GOOGLE_CLOUD_PROJECT", env) || getProviderEnvValue("GCLOUD_PROJECT", env));
		const hasLocation = !!getProviderEnvValue("GOOGLE_CLOUD_LOCATION", env);

		if (hasCredentials && hasProject && hasLocation) {
			return "<authenticated>";
		}
	}

	if (provider === "amazon-bedrock") {
		
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

/** Register an API implementation under `provider.api`. */
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

fu