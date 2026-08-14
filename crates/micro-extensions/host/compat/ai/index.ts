// What `@earendil-works/pi-ai` and `@mariozechner/pi-ai` resolve to for a pi extension
// running under micro.
//
// pi-ai's real index.ts is a hub of `export *` across a dozen submodules of its own —
// provider auth, model catalogs, telemetry, event-stream parsing. Most of an extension's
// use of it is types, erased by Bun before this file is ever reached. What's below is
// everything else: real, working code for every piece that is pure logic — touches no
// network, needs no credential — ported faithfully from pi-ai's own source rather than
// redesigned. None of it is a stub: every export does what pi-ai's own version does, for
// the same input.
//
// What's still a real gap, not silently papered over: the provider API clients
// (`anthropicMessagesApi`, `openAIResponsesApi`, `openAICompletionsApi`, and the rest under
// pi-ai's `api/`) are real HTTP+SSE clients against `@anthropic-ai/sdk`, `openai`,
// `@google/genai`, and `@aws-sdk/client-bedrock-runtime` — thousands of lines of wire-format
// logic, not glue, and not vendored into this shim. `createProvider()` below still lets an
// extension register a provider backed by its *own* `stream`/`streamSimple` (exactly what
// `custom-provider-gitlab-duo/index.ts` does — see `./compat.ts`), so the gap is narrower
// than "provider registration doesn't work"; it's specifically "pi-ai's own built-in API
// clients aren't here to register." `Type`/`Static`/`TSchema` are typebox's own schema
// builder, re-exported from the real `typebox` package rather than reimplemented — micro's
// NODE_PATH wiring (`crates/micro-extensions/src/compat.rs`) already makes the genuine
// one reachable, the same package extensions build tool parameters against.
//
// `@earendil-works/pi-ai/oauth` needs no runtime exports at all — see `./oauth.ts`: pi-ai's
// real `oauth.ts` is `export type` only, so every import from it is already free.

import type { TSchema, TUnsafe } from "typebox";
import { Type } from "typebox";
import { Compile } from "typebox/compile";
import type { TLocalizedValidationError } from "typebox/error";
import { Value } from "typebox/value";

export type { Static, TSchema } from "typebox";
export { Type };

// Shapes carried across this file, typed locally rather than imported from pi-ai's own
// `types.ts` — a value-level import of that file would need everything else it pulls in,
// while every consumer of these as *types* already reaches the real ones through
// `import type` at zero cost (see the file header). Kept narrow: only what the functions
// below actually construct or inspect.

export type KnownApi =
	| "openai-completions"
	| "mistral-conversations"
	| "openai-responses"
	| "azure-openai-responses"
	| "openai-codex-responses"
	| "anthropic-messages"
	| "bedrock-converse-stream"
	| "google-generative-ai"
	| "google-vertex"
	| "pi-messages";
export type Api = KnownApi | (string & {});
export type ProviderId = string;
export type ThinkingLevel = "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type ModelThinkingLevel = "off" | ThinkingLevel;
export type ThinkingLevelMap = Partial<Record<ModelThinkingLevel, string | null>>;
export type ProviderEnv = Record<string, string>;
export type ProviderHeaders = Record<string, string | null>;
export type StopReason = "pending" | "stop" | "length" | "toolUse" | "error" | "aborted" | "deferred";

export interface TextContent {
	type: "text";
	text: string;
	textSignature?: string;
}

export interface ThinkingContent {
	type: "thinking";
	thinking: string;
	thinkingSignature?: string;
	redacted?: boolean;
}

export interface ImageContent {
	type: "image";
	data: string;
	mimeType: string;
}

export interface ToolCall {
	type: "toolCall";
	id: string;
	name: string;
	arguments: Record<string, any>;
	thoughtSignature?: string;
	namespace?: string;
}

export interface Usage {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cacheWrite1h?: number;
	reasoning?: number;
	totalTokens: number;
	cost: { input: number; output: number; cacheRead: number; cacheWrite: number; total: number };
}

export interface DeferredHandle {
	provider: string;
	modelId: string;
	api: string;
	id: string;
	expiresAt?: number;
	pollAfterMs?: number;
	data?: unknown;
}

export interface UserMessage {
	role: "user";
	content: string | (TextContent | ImageContent)[];
	timestamp: number;
}

export interface AssistantMessageDiagnostic {
	type: string;
	timestamp: number;
	error?: DiagnosticErrorInfo;
	details?: Record<string, unknown>;
}

export interface AssistantMessage {
	role: "assistant";
	content: (TextContent | ThinkingContent | ToolCall)[];
	api: Api;
	provider: ProviderId;
	model: string;
	responseModel?: string;
	responseId?: string;
	diagnostics?: AssistantMessageDiagnostic[];
	usage: Usage;
	stopReason: StopReason;
	deferred?: DeferredHandle;
	errorMessage?: string;
	rawStopReason?: string;
	endTurn?: boolean;
	timestamp: number;
}

export interface ToolResultMessage<TDetails = any> {
	role: "toolResult";
	toolCallId: string;
	toolName: string;
	content: (TextContent | ImageContent)[];
	details?: TDetails;
	usage?: Usage;
	addedToolNames?: string[];
	isError: boolean;
	timestamp: number;
}

export type Message = UserMessage | AssistantMessage | ToolResultMessage;

export interface Tool<TParameters extends TSchema = TSchema> {
	name: string;
	description: string;
	parameters: TParameters;
	constrainedSampling?: false | { type: "json_schema"; strict: "prefer" | "require" } | { type: "grammar"; variants: Record<string, string> };
}

export interface Context {
	systemPrompt?: string;
	messages: Message[];
	tools?: Tool[];
}

export type AssistantMessageEvent =
	| { type: "start"; partial: AssistantMessage }
	| { type: "text_start"; contentIndex: number; partial: AssistantMessage }
	| { type: "text_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
	| { type: "text_end"; contentIndex: number; content: string; partial: AssistantMessage }
	| { type: "thinking_start"; contentIndex: number; partial: AssistantMessage }
	| { type: "thinking_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
	| { type: "thinking_end"; contentIndex: number; content: string; partial: AssistantMessage }
	| { type: "tool_call"; contentIndex: number; toolCall: ToolCall; partial: AssistantMessage }
	| { type: "done"; message: AssistantMessage }
	| { type: "error"; reason: string; error: AssistantMessage };

export interface ModelCostRates {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
}

export interface ModelCostTier extends ModelCostRates {
	inputTokensAbove: number;
}

export interface ModelCost extends ModelCostRates {
	tiers?: ModelCostTier[];
}

export interface Model<TApi extends Api = Api> {
	id: string;
	name: string;
	api: TApi;
	provider: ProviderId;
	baseUrl: string;
	reasoning: boolean;
	thinkingLevelMap?: ThinkingLevelMap;
	input: ("text" | "image")[];
	cost: ModelCost;
	contextWindow: number;
	maxTokens: number;
	samplingParams?: Record<string, unknown>;
	headers?: Record<string, string>;
	compat?: unknown;
}

/** Passthrough request shape. Every field here rides opaquely through this file's
 *  functions (`lazyStream`, `createProvider`, ...) without being read, so it is typed
 *  loosely rather than reproducing pi-ai's full per-field documentation. */
export interface StreamOptions {
	signal?: AbortSignal;
	telemetryContext?: unknown;
	apiKey?: string;
	fetch?: typeof globalThis.fetch;
	env?: ProviderEnv;
	onPayload?: (payload: unknown, model: Model) => unknown;
	onResponse?: (response: unknown, model: Model) => void | Promise<void>;
	headers?: ProviderHeaders;
	timeoutMs?: number;
	maxRetries?: number;
	maxRetryDelayMs?: number;
	[key: string]: unknown;
}

export interface SimpleStreamOptions extends StreamOptions {
	retryPolicy?: RetryPolicy;
}

export type DeferredFetchOptions = StreamOptions;
export type DeferredCancelOptions = StreamOptions;

export interface ProviderStreams {
	stream(model: Model<Api>, context: Context, options?: StreamOptions): AssistantMessageEventStream;
	streamSimple(model: Model<Api>, context: Context, options?: SimpleStreamOptions): AssistantMessageEventStream;
	fetchDeferred?(model: Model<Api>, handle: DeferredHandle, options?: DeferredFetchOptions): AssistantMessageEventStream;
	cancelDeferred?(model: Model<Api>, handle: DeferredHandle, options?: DeferredCancelOptions): Promise<void>;
}

// ---------------------------------------------------------------------------------------
// utils/typebox-helpers.ts
// ---------------------------------------------------------------------------------------

/**
 * Creates a string enum schema compatible with Google's API and other providers that
 * don't support anyOf/const patterns. Identical to pi-ai's own implementation — it is a
 * thin wrapper over `Type.Unsafe`, nothing about it is provider- or process-specific.
 */
export function StringEnum<T extends readonly string[]>(
	values: T,
	options?: { description?: string; default?: T[number] },
): TUnsafe<T[number]> {
	return Type.Unsafe<T[number]>({
		type: "string",
		enum: values as any,
		...(options?.description && { description: options.description }),
		...(options?.default && { default: options.default }),
	});
}

// ---------------------------------------------------------------------------------------
// utils/uuid.ts
// ---------------------------------------------------------------------------------------

let lastTimestamp = -Infinity;
let sequence = 0;

function fillRandomBytes(bytes: Uint8Array): void {
	if (globalThis.crypto?.getRandomValues) {
		globalThis.crypto.getRandomValues(bytes);
		return;
	}
	for (let i = 0; i < bytes.length; i++) {
		bytes[i] = Math.floor(Math.random() * 256);
	}
}

/** Generate a time-ordered UUIDv7. */
export function uuidv7(): string {
	const random = new Uint8Array(16);
	fillRandomBytes(random);
	const timestamp = Date.now();

	if (timestamp > lastTimestamp) {
		sequence = random[6] * 0x1000000 + random[7] * 0x10000 + random[8] * 0x100 + random[9];
		lastTimestamp = timestamp;
	} else {
		sequence = (sequence + 1) >>> 0;
		if (sequence === 0) lastTimestamp++;
	}

	const bytes = new Uint8Array(16);
	bytes[0] = (lastTimestamp / 0x10000000000) & 0xff;
	bytes[1] = (lastTimestamp / 0x100000000) & 0xff;
	bytes[2] = (lastTimestamp / 0x1000000) & 0xff;
	bytes[3] = (lastTimestamp / 0x10000) & 0xff;
	bytes[4] = (lastTimestamp / 0x100) & 0xff;
	bytes[5] = lastTimestamp & 0xff;
	bytes[6] = 0x70 | ((sequence >>> 28) & 0x0f);
	bytes[7] = (sequence >>> 20) & 0xff;
	bytes[8] = 0x80 | ((sequence >>> 14) & 0x3f);
	bytes[9] = (sequence >>> 6) & 0xff;
	bytes[10] = ((sequence & 0x3f) << 2) | (random[10] & 0x03);
	bytes[11] = random[11];
	bytes[12] = random[12];
	bytes[13] = random[13];
	bytes[14] = random[14];
	bytes[15] = random[15];

	const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
	return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10, 16).join("")}`;
}

// ---------------------------------------------------------------------------------------
// utils/text.ts
// ---------------------------------------------------------------------------------------

type ContentBlock = TextContent | ImageContent | ThinkingContent | ToolCall;

/** Extract and join text from message content. */
export function contentText(content: string | readonly ContentBlock[], separator = "\n"): string {
	if (typeof content === "string") return content;
	return content
		.filter((block) => block.type === "text")
		.map((block) => block.text)
		.join(separator);
}

// ---------------------------------------------------------------------------------------
// utils/overflow.ts
// ---------------------------------------------------------------------------------------

const OVERFLOW_PATTERNS = [
	/prompt is too long/i,
	/request_too_large/i,
	/input is too long for requested model/i,
	/exceeds the context window/i,
	/exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\d,]+ tokens?|\s*\([\d,]+\))/i,
	/input token count.*exceeds the maximum/i,
	/maximum prompt length is \d+/i,
	/reduce the length of the messages/i,
	/maximum context length is \d+ tokens/i,
	/exceeds (?:the )?maximum allowed input length of [\d,]+ tokens?/i,
	/input \(\d+ tokens\) is longer than the model'?s context length \(\d+ tokens\)/i,
	/exceeds the limit of \d+/i,
	/exceeds the available context size/i,
	/greater than the context length/i,
	/context window exceeds limit/i,
	/exceeded model token limit/i,
	/too large for model with \d+ maximum context length/i,
	/prompt has [\d,]+ tokens?, but the configured context size is [\d,]+ tokens?/i,
	/model_context_window_exceeded/i,
	/prompt too long; exceeded (?:max )?context length/i,
	/range of input length should be/i,
	/context[_ ]length[_ ]exceeded/i,
	/too many tokens/i,
	/token limit exceeded/i,
	/^4(?:00|13)\s*(?:status code)?\s*\(no body\)/i,
];

const NON_OVERFLOW_PATTERNS = [/^(Throttling error|Service unavailable):/i, /rate limit/i, /too many requests/i];

/** Check if an assistant message represents a context overflow error. See pi-ai's
 *  `utils/overflow.ts` for the full per-provider reliability notes this mirrors. */
export function isContextOverflow(message: AssistantMessage, contextWindow?: number): boolean {
	if (message.stopReason === "error" && message.errorMessage) {
		const isNonOverflow = NON_OVERFLOW_PATTERNS.some((p) => p.test(message.errorMessage!));
		if (!isNonOverflow && OVERFLOW_PATTERNS.some((p) => p.test(message.errorMessage!))) {
			return true;
		}
	}

	if (contextWindow && message.stopReason === "stop") {
		const inputTokens = message.usage.input + message.usage.cacheRead;
		if (inputTokens > contextWindow) {
			return true;
		}
	}

	if (contextWindow && message.stopReason === "length" && message.usage.output === 0) {
		const inputTokens = message.usage.input + message.usage.cacheRead;
		if (inputTokens >= contextWindow * 0.99) {
			return true;
		}
	}

	return false;
}

/** Check whether a length stop ended below the caller or model's intended output limit. */
export function isRecoverableLength(message: AssistantMessage, desiredMaxOutput: number): boolean {
	return message.stopReason === "length" && desiredMaxOutput > 0 && message.usage.output < desiredMaxOutput;
}

/** Get the overflow patterns for testing purposes. */
export function getOverflowPatterns(): RegExp[] {
	return [...OVERFLOW_PATTERNS];
}

// ---------------------------------------------------------------------------------------
// utils/retry.ts
// ---------------------------------------------------------------------------------------

function buildProviderErrorPattern(patterns: readonly string[]): RegExp {
	return new RegExp(patterns.join("|"), "i");
}

const NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN = buildProviderErrorPattern([
	"GoUsageLimitError",
	"FreeUsageLimitError",
	"Monthly usage limit reached",
	"available balance",
	"insufficient_quota",
	"out of budget",
	"quota exceeded",
	"billing",
]);

const RETRYABLE_PROVIDER_ERROR_PATTERN = buildProviderErrorPattern([
	"overloaded",
	"rate.?limit",
	"too many requests",
	"429",
	"500",
	"502",
	"503",
	"504",
	"524",
	"service.?unavailable",
	"server.?error",
	"internal.?error",
	"provider.?returned.?error",
	"exceeded request buffer limit while retrying upstream",
	"network.?error",
	"connection.?error",
	"connection.?refused",
	"connection.?lost",
	"other side closed",
	"fetch failed",
	"getaddrinfo",
	"ENOTFOUND",
	"EAI_AGAIN",
	"upstream.?connect",
	"reset before headers",
	"socket hang up",
	"socket connection was closed",
	"timed? out",
	"timeout",
	"terminated",
	"websocket.?closed",
	"websocket.?error",
	"ended without",
	"stream ended before message_stop",
	"stream ended before a terminal response event",
	"http2 request did not get a response",
	"retry delay",
	"you can retry your request",
	"try your request again",
	"please retry your request",
	"ResourceExhausted",
]);

/** Retry policy: bounded attempts with exponential backoff (`baseDelayMs * 2^(attempt-1)`). */
export interface RetryPolicy {
	enabled: boolean;
	maxRetries: number;
	baseDelayMs: number;
}

/** Optional callbacks emitted by {@link retryAssistantCall} around each retry. */
export interface RetryCallbacks {
	onRetryScheduled?: (attempt: number, maxAttempts: number, delayMs: number, errorMessage: string) => void | Promise<void>;
	onRetryAttemptStart?: () => void | Promise<void>;
	onRetryFinished?: (success: boolean, attempt: number, finalError?: string) => void | Promise<void>;
}

class RetrySleepAbortError extends Error {
	constructor() {
		super("Aborted");
	}
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted) {
			reject(new RetrySleepAbortError());
			return;
		}
		const timeout = setTimeout(resolve, ms);
		signal?.addEventListener(
			"abort",
			() => {
				clearTimeout(timeout);
				reject(new RetrySleepAbortError());
			},
			{ once: true },
		);
	});
}

/** Run a single assistant-producing call with bounded retry on transient errors. See
 *  pi-ai's `utils/retry.ts` for the full behavior contract this mirrors exactly. */
export async function retryAssistantCall(
	produce: () => Promise<AssistantMessage>,
	policy: RetryPolicy | undefined,
	signal: AbortSignal | undefined,
	callbacks?: RetryCallbacks,
): Promise<AssistantMessage> {
	const maxAttempts = policy?.enabled ? policy.maxRetries : 0;

	let attempt = 0;
	let lastRetry: { attempt: number; errorMessage: string } | undefined;
	for (;;) {
		const response = await produce();

		if (response.stopReason === "aborted") {
			if (lastRetry) await callbacks?.onRetryFinished?.(false, lastRetry.attempt);
			return response;
		}

		if (response.stopReason !== "error") {
			if (lastRetry) await callbacks?.onRetryFinished?.(true, lastRetry.attempt);
			return response;
		}

		if (attempt >= maxAttempts || !isRetryableAssistantError(response)) {
			if (lastRetry) await callbacks?.onRetryFinished?.(false, lastRetry.attempt, response.errorMessage);
			return response;
		}

		attempt++;
		lastRetry = { attempt, errorMessage: response.errorMessage || "Unknown error" };
		const delayMs = policy!.baseDelayMs * 2 ** (attempt - 1);
		await callbacks?.onRetryScheduled?.(attempt, maxAttempts, delayMs, lastRetry.errorMessage);

		try {
			await sleep(delayMs, signal);
		} catch (error) {
			await callbacks?.onRetryFinished?.(false, attempt, lastRetry.errorMessage);
			if (error instanceof RetrySleepAbortError) {
				return { ...response, stopReason: "aborted", errorMessage: undefined };
			}
			throw error;
		}
		await callbacks?.onRetryAttemptStart?.();
	}
}

/** Classifies whether a failed assistant message looks like a transient provider or
 *  transport error, so callers can decide if the last assistant turn should be restarted. */
export function isRetryableAssistantError(message: AssistantMessage): boolean {
	if (message.stopReason !== "error" || !message.errorMessage) return false;
	const errorMessage = message.errorMessage;
	if (NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN.test(errorMessage)) return false;
	return RETRYABLE_PROVIDER_ERROR_PATTERN.test(errorMessage);
}

// ---------------------------------------------------------------------------------------
// utils/json-parse.ts
//
// `parseStreamingJson`'s fallback in pi-ai goes through the `partial-json` npm package
// once `repairJson` isn't enough. That package isn't vendored into this shim (a small,
// standalone dependency — see the report to the team for the concrete ask), so the
// fallback here is a real tolerant parser written against the same contract instead:
// never throw, return as much of the streamed object as is syntactically complete, drop
// only the trailing fragment that streaming hasn't finished emitting yet. It works by
// closing whatever string/array/object is still open at the end of the text and, if that
// still doesn't parse, trimming one character at a time from the tail and re-closing
// until something does. A well-formed prefix (a tool call's first three finished
// arguments, say) survives; only the incomplete tail is ever dropped.
// ---------------------------------------------------------------------------------------

const VALID_JSON_ESCAPES = new Set(['"', "\\", "/", "b", "f", "n", "r", "t", "u"]);

function isControlCharacter(char: string): boolean {
	const codePoint = char.codePointAt(0);
	return codePoint !== undefined && codePoint >= 0x00 && codePoint <= 0x1f;
}

function escapeControlCharacter(char: string): string {
	switch (char) {
		case "\b":
			return "\\b";
		case "\f":
			return "\\f";
		case "\n":
			return "\\n";
		case "\r":
			return "\\r";
		case "\t":
			return "\\t";
		default:
			return `\\u${char.codePointAt(0)?.toString(16).padStart(4, "0") ?? "0000"}`;
	}
}

/** Repairs malformed JSON string literals by escaping raw control characters inside
 *  strings and doubling backslashes before invalid escape characters. */
export function repairJson(json: string): string {
	let repaired = "";
	let inString = false;

	for (let index = 0; index < json.length; index++) {
		const char = json[index];

		if (!inString) {
			repaired += char;
			if (char === '"') {
				inString = true;
			}
			continue;
		}

		if (char === '"') {
			repaired += char;
			inString = false;
			continue;
		}

		if (char === "\\") {
			const nextChar = json[index + 1];
			if (nextChar === undefined) {
				repaired += "\\\\";
				continue;
			}

			if (nextChar === "u") {
				const unicodeDigits = json.slice(index + 2, index + 6);
				if (/^[0-9a-fA-F]{4}$/.test(unicodeDigits)) {
					repaired += `\\u${unicodeDigits}`;
					index += 5;
					continue;
				}
			}

			if (VALID_JSON_ESCAPES.has(nextChar)) {
				repaired += `\\${nextChar}`;
				index += 1;
				continue;
			}

			repaired += "\\\\";
			continue;
		}

		repaired += isControlCharacter(char) ? escapeControlCharacter(char) : char;
	}

	return repaired;
}

export function parseJsonWithRepair<T>(json: string): T {
	try {
		return JSON.parse(json) as T;
	} catch (error) {
		const repairedJson = repairJson(json);
		if (repairedJson !== json) {
			return JSON.parse(repairedJson) as T;
		}
		throw error;
	}
}

/** Scan text for what's still open at the end: a stack of unclosed `{`/`[`, and whether
 *  the text ends inside an unterminated string (with a dangling escape, if so). */
function scanOpenStructures(text: string): { stack: ("{" | "[")[]; inString: boolean; trailingEscape: boolean } {
	const stack: ("{" | "[")[] = [];
	let inString = false;
	let escapeNext = false;
	for (const char of text) {
		if (inString) {
			if (escapeNext) {
				escapeNext = false;
			} else if (char === "\\") {
				escapeNext = true;
			} else if (char === '"') {
				inString = false;
			}
			continue;
		}
		if (char === '"') {
			inString = true;
		} else if (char === "{" || char === "[") {
			stack.push(char);
		} else if (char === "}" || char === "]") {
			stack.pop();
		}
	}
	return { stack, inString, trailingEscape: escapeNext };
}

/** Close whatever string/array/object `text` leaves open, so it has a chance of parsing
 *  as valid (if still incomplete) JSON. */
function closePartialJson(text: string): string {
	const { stack, inString, trailingEscape } = scanOpenStructures(text);
	let closed = trailingEscape ? text.slice(0, -1) : text;
	if (inString) closed += '"';
	for (let i = stack.length - 1; i >= 0; i--) {
		closed += stack[i] === "{" ? "}" : "]";
	}
	return closed;
}

/** Best-effort parse of JSON that may still be streaming in. Trims a trailing fragment
 *  that hasn't finished arriving rather than failing outright — see the section header
 *  for why this exists in place of pi-ai's `partial-json` dependency. */
function tolerantParsePartialJson<T>(text: string): T | undefined {
	const trimmed = text.trim();
	if (!trimmed) return undefined;
	for (let cut = 0; cut <= trimmed.length; cut++) {
		const candidate = cut === 0 ? trimmed : trimmed.slice(0, trimmed.length - cut).trimEnd();
		if (cut > 0 && candidate.length === 0) break;
		try {
			return JSON.parse(closePartialJson(candidate)) as T;
		} catch {
			// The tail up to this cut isn't a complete value yet; trim further.
		}
	}
	return undefined;
}

/** Attempts to parse potentially incomplete JSON during streaming. Always returns a valid
 *  object, even if the JSON is incomplete. */
export function parseStreamingJson<T = Record<string, unknown>>(partialJson: string | undefined): T {
	if (!partialJson || partialJson.trim() === "") {
		return {} as T;
	}

	try {
		return parseJsonWithRepair<T>(partialJson);
	} catch {
		const tolerant = tolerantParsePartialJson<T>(partialJson);
		if (tolerant !== undefined) return tolerant;
		const repairedTolerant = tolerantParsePartialJson<T>(repairJson(partialJson));
		return repairedTolerant !== undefined ? repairedTolerant : ({} as T);
	}
}

// ---------------------------------------------------------------------------------------
// utils/diagnostics.ts
// ---------------------------------------------------------------------------------------

export interface DiagnosticErrorInfo {
	name?: string;
	message: string;
	stack?: string;
	code?: string | number;
}

export function formatThrownValue(value: unknown): string {
	if (value instanceof Error) return value.message || value.name;
	if (typeof value === "string") return value;
	return String(value);
}

export function extractDiagnosticError(error: unknown): DiagnosticErrorInfo {
	if (!(error instanceof Error)) return { name: "ThrownValue", message: formatThrownValue(error) };
	const code = (error as Error & { code?: unknown }).code;
	return {
		name: error.name || undefined,
		message: error.message || error.name,
		stack: error.stack,
		code: typeof code === "string" || typeof code === "number" ? code : undefined,
	};
}

export function createAssistantMessageDiagnostic(
	type: string,
	error: unknown,
	details?: Record<string, unknown>,
): AssistantMessageDiagnostic {
	return { type, timestamp: Date.now(), error: extractDiagnosticError(error), details };
}

export function appendAssistantMessageDiagnostic<T extends { diagnostics?: AssistantMessageDiagnostic[] }>(
	message: T,
	diagnostic: AssistantMessageDiagnostic,
): void {
	message.diagnostics = [...(message.diagnostics ?? []), diagnostic];
}

// ---------------------------------------------------------------------------------------
// utils/event-stream.ts
// ---------------------------------------------------------------------------------------

/** Generic event stream class for async iteration. */
export class EventStream<T, R = T> implements AsyncIterable<T> {
	private queue: T[] = [];
	private waiting: ((value: IteratorResult<T>) => void)[] = [];
	private done = false;
	private finalResultPromise: Promise<R>;
	private resolveFinalResult!: (result: R) => void;
	private isComplete: (event: T) => boolean;
	private extractResult: (event: T) => R;

	constructor(isComplete: (event: T) => boolean, extractResult: (event: T) => R) {
		this.isComplete = isComplete;
		this.extractResult = extractResult;
		this.finalResultPromise = new Promise((resolve) => {
			this.resolveFinalResult = resolve;
		});
	}

	push(event: T): void {
		if (this.done) return;

		if (this.isComplete(event)) {
			this.done = true;
			this.resolveFinalResult(this.extractResult(event));
		}

		const waiter = this.waiting.shift();
		if (waiter) {
			waiter({ value: event, done: false });
		} else {
			this.queue.push(event);
		}
	}

	end(result?: R): void {
		this.done = true;
		if (result !== undefined) {
			this.resolveFinalResult(result);
		}
		while (this.waiting.length > 0) {
			const waiter = this.waiting.shift()!;
			waiter({ value: undefined as any, done: true });
		}
	}

	async *[Symbol.asyncIterator](): AsyncIterator<T> {
		while (true) {
			if (this.queue.length > 0) {
				yield this.queue.shift()!;
			} else if (this.done) {
				return;
			} else {
				const result = await new Promise<IteratorResult<T>>((resolve) => this.waiting.push(resolve));
				if (result.done) return;
				yield result.value;
			}
		}
	}

	result(): Promise<R> {
		return this.finalResultPromise;
	}
}

export class AssistantMessageEventStream extends EventStream<AssistantMessageEvent, AssistantMessage> {
	constructor() {
		super(
			(event) => event.type === "done" || event.type === "error",
			(event) => {
				if (event.type === "done") {
					return event.message;
				} else if (event.type === "error") {
					return event.error;
				}
				throw new Error("Unexpected event type for final result");
			},
		);
	}
}

/** Factory function for AssistantMessageEventStream (for use in extensions) */
export function createAssistantMessageEventStream(): AssistantMessageEventStream {
	return new AssistantMessageEventStream();
}

// ---------------------------------------------------------------------------------------
// utils/validation.ts
// ---------------------------------------------------------------------------------------

const validatorCache = new WeakMap<object, ReturnType<typeof Compile>>();
const TYPEBOX_KIND = Symbol.for("TypeBox.Kind");

interface JsonSchemaObject {
	type?: string | string[];
	properties?: Record<string, JsonSchemaObject>;
	required?: string[];
	items?: JsonSchemaObject | JsonSchemaObject[];
	additionalProperties?: boolean | JsonSchemaObject;
	allOf?: JsonSchemaObject[];
	anyOf?: JsonSchemaObject[];
	oneOf?: JsonSchemaObject[];
}

function getSchemaTypes(schema: JsonSchemaObject): string[] {
	if (typeof schema.type === "string") {
		return [schema.type];
	}
	if (Array.isArray(schema.type)) {
		return schema.type.filter((type): type is string => typeof type === "string");
	}
	return [];
}

function matchesJsonType(value: unknown, type: string): boolean {
	switch (type) {
		case "number":
			return typeof value === "number";
		case "integer":
			return typeof value === "number" && Number.isInteger(value);
		case "boolean":
			return typeof value === "boolean";
		case "string":
			return typeof value === "string";
		case "null":
			return value === null;
		case "array":
			return Array.isArray(value);
		case "object":
			return typeof value === "object" && value !== null && !Array.isArray(value);
		default:
			return false;
	}
}

function getSubSchemaValidator(schema: JsonSchemaObject): ReturnType<typeof Compile> | undefined {
	try {
		return getValidator(schema as Tool["parameters"]);
	} catch {
		return undefined;
	}
}

function coercePrimitiveByType(value: unknown, type: string): unknown {
	switch (type) {
		case "number": {
			if (value === null) return 0;
			if (typeof value === "string" && value.trim() !== "") {
				const parsed = Number(value);
				if (Number.isFinite(parsed)) return parsed;
			}
			if (typeof value === "boolean") return value ? 1 : 0;
			return value;
		}
		case "integer": {
			if (value === null) return 0;
			if (typeof value === "string" && value.trim() !== "") {
				const parsed = Number(value);
				if (Number.isInteger(parsed)) return parsed;
			}
			if (typeof value === "boolean") return value ? 1 : 0;
			return value;
		}
		case "boolean": {
			if (value === null) return false;
			if (typeof value === "string") {
				if (value === "true") return true;
				if (value === "false") return false;
			}
			if (typeof value === "number") {
				if (value === 1) return true;
				if (value === 0) return false;
			}
			return value;
		}
		case "string": {
			if (value === null) return "";
			if (typeof value === "number" || typeof value === "boolean") return String(value);
			return value;
		}
		case "null": {
			if (value === "" || value === 0 || value === false) return null;
			return value;
		}
		default:
			return value;
	}
}

function applySchemaObjectCoercion(value: Record<string, unknown>, schema: JsonSchemaObject): void {
	const properties = schema.properties;
	const definedKeys = new Set<string>(properties ? Object.keys(properties) : []);

	if (properties) {
		for (const [key, propertySchema] of Object.entries(properties)) {
			if (!(key in value)) continue;
			value[key] = coerceWithJsonSchema(value[key], propertySchema);
		}
	}

	if (schema.additionalProperties && typeof schema.additionalProperties === "object") {
		for (const [key, propertyValue] of Object.entries(value)) {
			if (definedKeys.has(key)) continue;
			value[key] = coerceWithJsonSchema(propertyValue, schema.additionalProperties);
		}
	}
}

function applySchemaArrayCoercion(value: unknown[], schema: JsonSchemaObject): void {
	if (Array.isArray(schema.items)) {
		for (let index = 0; index < value.length; index++) {
			const itemSchema = schema.items[index];
			if (!itemSchema) continue;
			value[index] = coerceWithJsonSchema(value[index], itemSchema);
		}
		return;
	}

	if (schema.items && typeof schema.items === "object") {
		for (let index = 0; index < value.length; index++) {
			value[index] = coerceWithJsonSchema(value[index], schema.items);
		}
	}
}

function coerceWithUnionSchema(value: unknown, schemas: JsonSchemaObject[]): unknown {
	for (const schema of schemas) {
		const validator = getSubSchemaValidator(schema);
		if (validator?.Check(value)) return value;
	}

	for (const schema of schemas) {
		const candidate = structuredClone(value);
		const coerced = coerceWithJsonSchema(candidate, schema);
		const validator = getSubSchemaValidator(schema);
		if (validator?.Check(coerced)) return coerced;
	}
	return value;
}

function coerceWithJsonSchema(value: unknown, schema: JsonSchemaObject): unknown {
	let nextValue = value;

	if (Array.isArray(schema.allOf)) {
		for (const nested of schema.allOf) {
			nextValue = coerceWithJsonSchema(nextValue, nested);
		}
	}

	if (Array.isArray(schema.anyOf)) {
		nextValue = coerceWithUnionSchema(nextValue, schema.anyOf);
	}

	if (Array.isArray(schema.oneOf)) {
		nextValue = coerceWithUnionSchema(nextValue, schema.oneOf);
	}

	const schemaTypes = getSchemaTypes(schema);
	const matchesUnionMember =
		schemaTypes.length > 1 && schemaTypes.some((schemaType) => matchesJsonType(nextValue, schemaType));
	if (schemaTypes.length > 0 && !matchesUnionMember) {
		for (const schemaType of schemaTypes) {
			const candidate = coercePrimitiveByType(nextValue, schemaType);
			if (candidate !== nextValue) {
				nextValue = candidate;
				break;
			}
		}
	}

	if (
		schemaTypes.includes("object") &&
		typeof nextValue === "object" &&
		nextValue !== null &&
		!Array.isArray(nextValue)
	) {
		applySchemaObjectCoercion(nextValue as Record<string, unknown>, schema);
	}

	if (schemaTypes.includes("array") && Array.isArray(nextValue)) {
		applySchemaArrayCoercion(nextValue, schema);
	}

	return nextValue;
}

function normalizeOptionalNulls(value: unknown, schema: JsonSchemaObject): void {
	if (Array.isArray(value)) {
		if (Array.isArray(schema.items)) {
			for (let index = 0; index < value.length; index++) {
				const itemSchema = schema.items[index];
				if (itemSchema) normalizeOptionalNulls(value[index], itemSchema);
			}
		} else if (schema.items) {
			for (const item of value) normalizeOptionalNulls(item, schema.items);
		}
		return;
	}
	if (typeof value !== "object" || value === null || !schema.properties) return;

	const object = value as Record<string, unknown>;
	const required = new Set(schema.required ?? []);
	for (const [key, propertySchema] of Object.entries(schema.properties)) {
		if (!(key in object)) continue;
		if (
			object[key] === null &&
			!required.has(key) &&
			typeof (propertySchema as { $ref?: unknown }).$ref !== "string" &&
			getSubSchemaValidator(propertySchema)?.Check(null) === false
		) {
			delete object[key];
		} else {
			normalizeOptionalNulls(object[key], propertySchema);
		}
	}
}

function getValidator(schema: Tool["parameters"]): ReturnType<typeof Compile> {
	const key = schema as object;
	const cached = validatorCache.get(key);
	if (cached) return cached;
	const validator = Compile(schema as TSchema);
	validatorCache.set(key, validator);
	return validator;
}

function formatValidationPath(error: TLocalizedValidationError): string {
	if (error.keyword === "required") {
		const requiredProperties = (error.params as { requiredProperties?: string[] }).requiredProperties;
		const requiredProperty = requiredProperties?.[0];
		if (requiredProperty) {
			const basePath = error.instancePath.replace(/^\//, "").replace(/\//g, ".");
			return basePath ? `${basePath}.${requiredProperty}` : requiredProperty;
		}
	}
	const path = error.instancePath.replace(/^\//, "").replace(/\//g, ".");
	return path || "root";
}

/** Finds a tool by name and validates the tool call arguments against its TypeBox schema. */
export function validateToolCall(tools: Tool[], toolCall: ToolCall): any {
	const tool = tools.find((t) => t.name === toolCall.name);
	if (!tool) {
		throw new Error(`Tool "${toolCall.name}" not found`);
	}
	return validateToolArguments(tool, toolCall);
}

/** Validates tool call arguments against the tool's TypeBox schema. */
export function validateToolArguments(tool: Tool, toolCall: ToolCall): any {
	const args = structuredClone(toolCall.arguments);
	normalizeOptionalNulls(args, tool.parameters as JsonSchemaObject);
	Value.Convert(tool.parameters as TSchema, args);

	const validator = getValidator(tool.parameters);
	if (!Object.getOwnPropertySymbols(tool.parameters).includes(TYPEBOX_KIND)) {
		const coerced = coerceWithJsonSchema(args, tool.parameters as JsonSchemaObject);
		if (coerced !== args) {
			if (typeof args === "object" && args !== null && typeof coerced === "object" && coerced !== null) {
				for (const key of Object.keys(args)) {
					delete (args as Record<string, unknown>)[key];
				}
				Object.assign(args as object, coerced);
			} else {
				return validator.Check(coerced) ? coerced : args;
			}
		}
	}

	if (validator.Check(args)) {
		return args;
	}

	const errors =
		validator
			.Errors(args)
			.map((error) => `  - ${formatValidationPath(error)}: ${error.message}`)
			.join("\n") || "Unknown validation error";

	const errorMessage = `Validation failed for tool "${toolCall.name}":\n${errors}\n\nReceived arguments:\n${JSON.stringify(toolCall.arguments, null, 2)}`;

	throw new Error(errorMessage);
}

// ---------------------------------------------------------------------------------------
// session-resources.ts
// ---------------------------------------------------------------------------------------

export type SessionResourceCleanup = (sessionId?: string) => void;

const sessionResourceCleanups = new Set<SessionResourceCleanup>();

export function registerSessionResourceCleanup(cleanup: SessionResourceCleanup): () => void {
	sessionResourceCleanups.add(cleanup);
	return () => {
		sessionResourceCleanups.delete(cleanup);
	};
}

export function cleanupSessionResources(sessionId?: string): void {
	const errors: unknown[] = [];
	for (const cleanup of sessionResourceCleanups) {
		try {
			cleanup(sessionId);
		} catch (error) {
			errors.push(error);
		}
	}
	if (errors.length > 0) {
		throw new AggregateError(errors, "Failed to cleanup session resources");
	}
}

// ---------------------------------------------------------------------------------------
// utils/abort.ts
// ---------------------------------------------------------------------------------------

function abortReason(signal: AbortSignal): unknown {
	if (signal.reason !== undefined) return signal.reason;
	const error = new Error("The operation was aborted");
	error.name = "AbortError";
	return error;
}

/** Create an operation-local signal for public APIs whose signal is optional. */
export function operationSignal(signal?: AbortSignal): AbortSignal {
	return signal ?? new AbortController().signal;
}

/** Stop waiting for an operation when its signal aborts while continuing to observe the
 *  abandoned promise so a later rejection is always handled. */
export function raceWithAbortSignal<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
	if (signal.aborted) {
		void operation.catch(() => {});
		return Promise.reject(abortReason(signal));
	}

	return new Promise<T>((resolve, reject) => {
		let settled = false;
		const cleanup = () => signal.removeEventListener("abort", onAbort);
		const onAbort = () => {
			if (settled) return;
			settled = true;
			cleanup();
			reject(abortReason(signal));
		};

		signal.addEventListener("abort", onAbort, { once: true });
		void operation.then(
			(value) => {
				if (settled) return;
				settled = true;
				cleanup();
				resolve(value);
			},
			(error: unknown) => {
				if (settled) return;
				settled = true;
				cleanup();
				reject(error);
			},
		);
		if (signal.aborted) onAbort();
	});
}

// ---------------------------------------------------------------------------------------
// auth/types.ts (value-carrying pieces only — everything else is a type, free via elision)
// ---------------------------------------------------------------------------------------

export interface ApiKeyCredential {
	type: "api_key";
	key?: string;
	env?: ProviderEnv;
}

export interface OAuthCredentials {
	refresh: string;
	access: string;
	expires: number;
	[key: string]: unknown;
}

export interface OAuthCredential extends OAuthCredentials {
	type: "oauth";
}

export type Credential = ApiKeyCredential | OAuthCredential;

export interface CredentialInfo {
	providerId: string;
	type: Credential["type"];
}

export interface AuthOperationOptions {
	signal?: AbortSignal;
}

export interface AuthContext {
	env(name: string): Promise<string | undefined>;
	fileExists(path: string): Promise<boolean>;
}

export interface ModelAuth {
	apiKey?: string;
	headers?: ProviderHeaders;
	baseUrl?: string;
}

export interface AuthResult {
	auth: ModelAuth;
	env?: ProviderEnv;
	source?: string;
}

export interface AuthCheck {
	source?: string;
	type: "api_key" | "oauth";
}

export type AuthType = "api_key" | "oauth";

export type AuthPrompt = { signal?: AbortSignal } & (
	| { type: "text"; message: string; placeholder?: string }
	| { type: "secret"; message: string; placeholder?: string }
	| { type: "select"; message: string; options: readonly { id: string; label: string; description?: string }[] }
	| { type: "manual_code"; message: string; placeholder?: string }
);

export interface AuthInfoLink {
	url: string;
	label?: string;
}

export type AuthEvent =
	| { type: "info"; message: string; links?: readonly AuthInfoLink[] }
	| { type: "auth_url"; url: string; instructions?: string }
	| { type: "device_code"; userCode: string; verificationUri: string; intervalSeconds?: number; expiresInSeconds?: number }
	| { type: "progress"; message: string };

export interface AuthInteraction {
	signal?: AbortSignal;
	prompt(prompt: AuthPrompt): Promise<string>;
	notify(event: AuthEvent): void;
}

export type ProviderAuthInteraction = AuthInteraction & { signal: AbortSignal };

export interface ApiKeyAuth {
	name: string;
	login?(interaction: ProviderAuthInteraction): Promise<ApiKeyCredential>;
	check?(input: { ctx: AuthContext; credential?: ApiKeyCredential; signal: AbortSignal }): Promise<AuthCheck | undefined>;
	resolve(input: { ctx: AuthContext; credential?: ApiKeyCredential; signal: AbortSignal }): Promise<AuthResult | undefined>;
}

export interface OAuthAuth {
	name: string;
	isSubscription?: boolean;
	loginLabel?: string;
	login(interaction: ProviderAuthInteraction): Promise<OAuthCredential>;
	refresh(credential: OAuthCredential, signal: AbortSignal): Promise<OAuthCredential>;
	toAuth(credential: OAuthCredential): Promise<ModelAuth>;
}

export interface ProviderAuth {
	apiKey?: ApiKeyAuth;
	oauth?: OAuthAuth;
}

export interface CredentialStore {
	read(providerId: string, options?: AuthOperationOptions): Promise<Credential | undefined>;
	list(options?: AuthOperationOptions): Promise<readonly CredentialInfo[]>;
	modify(
		providerId: string,
		fn: (current: Credential | undefined) => Promise<Credential | undefined>,
		options?: AuthOperationOptions,
	): Promise<Credential | undefined>;
	delete(providerId: string, options?: AuthOperationOptions): Promise<void>;
}

// ---------------------------------------------------------------------------------------
// auth/context.ts
// ---------------------------------------------------------------------------------------

interface NodeFsModule {
	access(path: string): Promise<void>;
}

interface NodeOsModule {
	homedir(): string;
}

// Variable specifier so browser bundlers do not try to resolve node builtins.
const importNodeModule = (specifier: string): Promise<unknown> => import(specifier);

function getProcessEnv(): Record<string, string | undefined> | undefined {
	const proc = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process;
	return proc?.env;
}

/** Default auth context: env vars from `process.env`, file existence via node:fs. */
export function defaultProviderAuthContext(): AuthContext {
	return {
		async env(name: string): Promise<string | undefined> {
			const value = getProcessEnv()?.[name];
			return typeof value === "string" && value.trim().length > 0 ? value : undefined;
		},

		async fileExists(path: string): Promise<boolean> {
			try {
				const fs = (await importNodeModule("node:fs/promises")) as NodeFsModule;
				let resolved = path;
				if (resolved.startsWith("~")) {
					const os = (await importNodeModule("node:os")) as NodeOsModule;
					resolved = os.homedir() + resolved.slice(1);
				}
				await fs.access(resolved);
				return true;
			} catch {
				return false;
			}
		},
	};
}

// ---------------------------------------------------------------------------------------
// auth/credential-store.ts
// ---------------------------------------------------------------------------------------

/** Default in-memory credential store. Apps inject persistent stores. Keyed by
 *  `Provider.id`, one credential per provider. Writes are serialized per provider through
 *  a promise chain. */
export class InMemoryCredentialStore implements CredentialStore {
	private credentials = new Map<string, Credential>();
	private chains = new Map<string, Promise<unknown>>();

	private enqueue<T>(providerId: string, task: () => Promise<T>, options?: AuthOperationOptions): Promise<T> {
		const signal = operationSignal(options?.signal);
		const previous = this.chains.get(providerId) ?? Promise.resolve();
		const queued = (async () => {
			await previous.catch(() => {});
			signal.throwIfAborted();
			return task();
		})();
		const tail = queued.catch(() => {});
		this.chains.set(providerId, tail);
		void tail.then(() => {
			if (this.chains.get(providerId) === tail) this.chains.delete(providerId);
		});
		return raceWithAbortSignal(queued, signal);
	}

	async read(providerId: string, options?: AuthOperationOptions): Promise<Credential | undefined> {
		options?.signal?.throwIfAborted();
		return this.credentials.get(providerId);
	}

	async list(options?: AuthOperationOptions): Promise<readonly CredentialInfo[]> {
		options?.signal?.throwIfAborted();
		return [...this.credentials].map(([providerId, credential]) => ({ providerId, type: credential.type }));
	}

	modify(
		providerId: string,
		fn: (current: Credential | undefined) => Promise<Credential | undefined>,
		options?: AuthOperationOptions,
	): Promise<Credential | undefined> {
		return this.enqueue(
			providerId,
			async () => {
				const current = this.credentials.get(providerId);
				const next = await fn(current);
				options?.signal?.throwIfAborted();
				if (next !== undefined) this.credentials.set(providerId, next);
				return next ?? current;
			},
			options,
		);
	}

	delete(providerId: string, options?: AuthOperationOptions): Promise<void> {
		return this.enqueue(
			providerId,
			async () => {
				this.credentials.delete(providerId);
			},
			options,
		);
	}
}

// ---------------------------------------------------------------------------------------
// auth/helpers.ts
// ---------------------------------------------------------------------------------------

/** Standard api-key auth: a stored credential key wins, otherwise the first set env var
 *  resolves. Includes a `login` that prompts for the key. */
export function envApiKeyAuth(name: string, envVars: readonly string[]): ApiKeyAuth {
	return {
		name,
		login: async (interaction) => {
			interaction.signal.throwIfAborted();
			const key = await interaction.prompt({ type: "secret", message: `Enter ${name}` });
			interaction.signal.throwIfAborted();
			return { type: "api_key", key };
		},
		resolve: async ({ ctx, credential, signal }) => {
			signal.throwIfAborted();
			if (credential?.key) {
				return { auth: { apiKey: credential.key }, env: credential.env, source: "stored credential" };
			}
			for (const envVar of envVars) {
				const value = await ctx.env(envVar);
				signal.throwIfAborted();
				if (value) return { auth: { apiKey: value }, source: envVar };
			}
			return undefined;
		},
	};
}

/** Wraps a dynamically imported `OAuthAuth` so provider definitions can advertise OAuth
 *  without importing the implementation up front. */
export function lazyOAuth(input: {
	name: string;
	isSubscription?: boolean;
	loginLabel?: string;
	load: () => Promise<OAuthAuth>;
}): OAuthAuth {
	let promise: Promise<OAuthAuth> | undefined;
	const loaded = () => {
		promise ??= input.load();
		return promise;
	};
	return {
		name: input.name,
		isSubscription: input.isSubscription,
		loginLabel: input.loginLabel,
		login: async (interaction) => (await loaded()).login(interaction),
		refresh: async (credential, signal) => (await loaded()).refresh(credential, signal),
		toAuth: async (credential) => (await loaded()).toAuth(credential),
	};
}

// ---------------------------------------------------------------------------------------
// auth/resolve.ts
// ---------------------------------------------------------------------------------------

export type ModelsErrorCode = "model_source" | "model_validation" | "provider" | "stream" | "auth" | "oauth";

export interface AuthResolutionOverrides {
	credentials?: CredentialStore;
	authContext?: AuthContext;
}

export class ModelsError extends Error {
	readonly code: ModelsErrorCode;

	constructor(code: ModelsErrorCode, message: string, options?: { cause?: unknown }) {
		super(message, options);
		this.name = "ModelsError";
		this.code = code;
	}
}

/** Minimum remaining lifetime, in milliseconds, an OAuth credential must have after
 *  refresh before it's trusted for a request — mirrors pi-ai's own margin. */
const OAUTH_EXPIRY_MARGIN_MS = 60_000;

/**
 * Resolve provider auth from a stored credential plus a `ProviderAuth`'s api-key/OAuth
 * implementations. OAuth credentials are refreshed through `credentials.modify()` so
 * concurrent callers cannot double-refresh a rotated token; api-key resolution never
 * touches the store's write path. Faithful to pi-ai's `resolveProviderAuth` — this is
 * pure orchestration over caller-supplied `CredentialStore`/`AuthContext`/`ProviderAuth`
 * values, the same shape `envApiKeyAuth`/`InMemoryCredentialStore` above produce; it never
 * reaches for a credential this host wouldn't otherwise have handed the extension.
 */
export async function resolveProviderAuth(
	providerId: string,
	auth: ProviderAuth,
	credentials: CredentialStore,
	ctx: AuthContext,
	signal: AbortSignal,
	overrides?: AuthResolutionOverrides,
): Promise<AuthResult | undefined> {
	const store = overrides?.credentials ?? credentials;
	const authContext = overrides?.authContext ?? ctx;

	let stored: Credential | undefined;
	try {
		stored = await store.read(providerId, { signal });
	} catch (error) {
		throw new ModelsError("auth", `Credential store read failed for ${providerId}`, { cause: error });
	}
	signal.throwIfAborted();

	if (stored?.type === "oauth" && auth.oauth) {
		const oauth = auth.oauth;
		const needsRefresh = stored.expires - Date.now() <= OAUTH_EXPIRY_MARGIN_MS;
		let credential = stored;
		if (needsRefresh) {
			try {
				const refreshed = await store.modify(
					providerId,
					async (current) => {
						if (current?.type !== "oauth") return current;
						if (current.expires - Date.now() > OAUTH_EXPIRY_MARGIN_MS) return current;
						return oauth.refresh(current, signal);
					},
					{ signal },
				);
				if (refreshed?.type !== "oauth") {
					throw new Error("Credential store did not return an OAuth credential after refresh");
				}
				credential = refreshed;
			} catch (error) {
				if (error instanceof ModelsError) throw error;
				throw new ModelsError("oauth", `OAuth refresh failed for ${providerId}`, { cause: error });
			}
			if (credential.expires - Date.now() <= 0) {
				throw new ModelsError("oauth", `OAuth refresh returned a token that expires too soon for ${providerId}`);
			}
		}
		try {
			const modelAuth = await oauth.toAuth(credential);
			return { auth: modelAuth, source: oauth.name };
		} catch (error) {
			throw new ModelsError("oauth", `OAuth auth derivation failed for ${providerId}`, { cause: error });
		}
	}

	if (!auth.apiKey) return undefined;
	try {
		const apiKeyCredential = stored?.type === "api_key" ? stored : undefined;
		return await auth.apiKey.resolve({ ctx: authContext, credential: apiKeyCredential, signal });
	} catch (error) {
		throw new ModelsError("auth", `API key auth failed for provider ${providerId}`, { cause: error });
	}
}

// ---------------------------------------------------------------------------------------
// api/lazy.ts
// ---------------------------------------------------------------------------------------

function createSetupErrorMessage(model: Model<Api>, error: unknown): AssistantMessage {
	return {
		role: "assistant",
		content: [],
		api: model.api,
		provider: model.provider,
		model: model.id,
		usage: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		stopReason: "error",
		errorMessage: error instanceof Error ? error.message : String(error),
		timestamp: Date.now(),
	};
}

function hasResult(
	source: AsyncIterable<AssistantMessageEvent>,
): source is AsyncIterable<AssistantMessageEvent> & { result(): Promise<AssistantMessage> } {
	return typeof (source as { result?: unknown }).result === "function";
}

async function forwardStream(
	target: AssistantMessageEventStream,
	source: AsyncIterable<AssistantMessageEvent>,
): Promise<void> {
	for await (const event of source) {
		target.push(event);
	}
	target.end(hasResult(source) ? await source.result() : undefined);
}

/** Returns a stream synchronously while running async setup (auth resolution, lazy module
 *  loading) behind it. Setup failures terminate the stream with an error event. */
export function lazyStream(
	model: Model<Api>,
	setup: () => Promise<AsyncIterable<AssistantMessageEvent>>,
): AssistantMessageEventStream {
	const outer = new AssistantMessageEventStream();

	setup()
		.then((inner) => forwardStream(outer, inner))
		.catch((error) => {
			const message = createSetupErrorMessage(model, error);
			outer.push({ type: "error", reason: "error", error: message });
			outer.end(message);
		});

	return outer;
}

export interface LazyApiCapabilities {
	fetchDeferred?: boolean;
	cancelDeferred?: boolean;
}

/** Wraps a dynamically imported API implementation module as `ProviderStreams`. The
 *  module loads on first stream call; load failures terminate the returned stream with an
 *  error event. */
export function lazyApi(load: () => Promise<ProviderStreams>, capabilities?: LazyApiCapabilities): ProviderStreams {
	const api: ProviderStreams = {
		stream: (model, context, options) => lazyStream(model, async () => (await load()).stream(model, context, options)),
		streamSimple: (model, context, options) =>
			lazyStream(model, async () => (await load()).streamSimple(model, context, options)),
	};

	if (capabilities?.fetchDeferred) {
		api.fetchDeferred = (model, handle, options) =>
			lazyStream(model, async () => {
				const implementation = await load();
				if (!implementation.fetchDeferred) throw new Error("API does not support deferred responses");
				return implementation.fetchDeferred(model, handle, options);
			});
	}
	if (capabilities?.cancelDeferred) {
		api.cancelDeferred = async (model, handle, options) => {
			const implementation = await load();
			if (!implementation.cancelDeferred) throw new Error("API cannot cancel deferred responses");
			await implementation.cancelDeferred(model, handle, options);
		};
	}

	return api;
}

// ---------------------------------------------------------------------------------------
// models.ts — the pure composition/arithmetic pieces only. `createModels()`'s full
// `MutableModels` registry (refresh scheduling, catalog persistence, `getAvailable()`,
// login/logout orchestration — some 700 lines in pi-ai) is not reproduced here: no
// example extension constructs one directly, pi-coding-agent's own `ModelRuntime` is what
// extensions actually reach for that. `createProvider()` — what `pi.registerProvider()`'s
// own docs show extensions building directly — is reproduced in full.
// ---------------------------------------------------------------------------------------

export interface RefreshModelsContext {
	credential?: Credential;
	stored?: Readonly<{ models: readonly Model<Api>[]; checkedAt: number }>;
	publish(publication: { persist?: { models: readonly Model<Api>[]; checkedAt: number } | null; update?: () => void }): Promise<boolean>;
	allowNetwork: boolean;
	force?: boolean;
	signal: AbortSignal;
}

export interface Provider<TApi extends Api = Api> {
	readonly id: string;
	readonly name: string;
	readonly baseUrl?: string;
	readonly headers?: ProviderHeaders;
	readonly auth: ProviderAuth;
	getModels(): readonly Model<TApi>[];
	refreshModels?(context: RefreshModelsContext): Promise<void>;
	filterModels?(models: readonly Model<TApi>[], credential: Credential | undefined): readonly Model<TApi>[];
	stream<T extends TApi>(model: Model<T>, context: Context, options?: StreamOptions): AssistantMessageEventStream;
	streamSimple(model: Model<TApi>, context: Context, options?: SimpleStreamOptions): AssistantMessageEventStream;
	fetchDeferred?(model: Model<TApi>, handle: DeferredHandle, options?: DeferredFetchOptions): AssistantMessageEventStream;
	cancelDeferred?(model: Model<TApi>, handle: DeferredHandle, options?: DeferredCancelOptions): Promise<void>;
}

export interface CreateProviderOptions<TApi extends Api = Api> {
	id: string;
	name?: string;
	baseUrl?: string;
	headers?: ProviderHeaders;
	auth: ProviderAuth;
	models: readonly Model<TApi>[];
	fetchModels?: (context: RefreshModelsContext) => Promise<readonly Model<TApi>[]>;
	filterModels?: (models: readonly Model<TApi>[], credential: Credential | undefined) => readonly Model<TApi>[];
	api: ProviderStreams | Partial<Record<TApi, ProviderStreams>>;
}

/** Builds a provider from parts — id/name/auth/models plus one `ProviderStreams`
 *  implementation (or a map keyed by `model.api` for mixed-API providers). Identical
 *  composition logic to pi-ai's `createProvider`: no network I/O of its own, it only
 *  dispatches to whatever `api` the caller supplied — a hand-written `streamSimple` (as
 *  `custom-provider-gitlab-duo/index.ts` does) or a vendored API client. */
export function createProvider<TApi extends Api = Api>(input: CreateProviderOptions<TApi>): Provider<TApi> {
	const baselineModels = input.models;
	let dynamicModels: readonly Model<TApi>[] = [];
	const fetchModels = input.fetchModels;
	const currentModels = (): readonly Model<TApi>[] => {
		const merged = [...baselineModels];
		for (const model of dynamicModels) {
			const index = merged.findIndex((entry) => entry.id === model.id);
			if (index >= 0) merged[index] = model;
			else merged.push(model);
		}
		return merged;
	};
	const single = typeof (input.api as ProviderStreams).stream === "function" ? (input.api as ProviderStreams) : undefined;
	const byApi = single ? undefined : (input.api as Partial<Record<string, ProviderStreams>>);

	const apiFor = (model: Model<Api>): ProviderStreams | undefined => single ?? byApi?.[model.api];

	const dispatch = (
		model: Model<Api>,
		run: (streams: ProviderStreams) => AssistantMessageEventStream,
	): AssistantMessageEventStream => {
		const streams = apiFor(model);
		if (!streams) {
			return lazyStream(model, async () => {
				throw new ModelsError("stream", `Provider ${input.id} has no API implementation for "${model.api}"`);
			});
		}
		return run(streams);
	};

	const provider: Provider<TApi> = {
		id: input.id,
		name: input.name ?? input.id,
		baseUrl: input.baseUrl,
		headers: input.headers,
		auth: input.auth,
		getModels: currentModels,
		refreshModels: fetchModels
			? async (context) => {
					if (context.stored) {
						const restored = context.stored.models as Model<TApi>[];
						if (
							!(await context.publish({
								update: () => {
									dynamicModels = restored;
								},
							}))
						) {
							return;
						}
					}
					if (!context.allowNetwork || context.signal.aborted) return;
					const refreshed = await fetchModels(context);
					if (context.signal.aborted) return;
					await context.publish({
						persist: { models: refreshed, checkedAt: Date.now() },
						update: () => {
							dynamicModels = refreshed;
						},
					});
				}
			: undefined,
		filterModels: input.filterModels,
		stream: (model, context, options) => dispatch(model, (streams) => streams.stream(model, context, options)),
		streamSimple: (model, context, options) => dispatch(model, (streams) => streams.streamSimple(model, context, options)),
	};

	const streams = single ? [single] : Object.values(byApi ?? {}).filter((entry): entry is ProviderStreams => entry !== undefined);
	if (streams.some((entry) => entry.fetchDeferred !== undefined)) {
		provider.fetchDeferred = (model, handle, options) =>
			lazyStream(model, async () => {
				const implementation = apiFor(model);
				if (!implementation?.fetchDeferred) {
					throw new ModelsError("provider", `Provider ${input.id} does not support deferred responses for "${model.api}"`);
				}
				return implementation.fetchDeferred(model, handle, options);
			});
	}
	if (streams.some((entry) => entry.cancelDeferred !== undefined)) {
		provider.cancelDeferred = async (model, handle, options) => {
			const implementation = apiFor(model);
			if (!implementation?.cancelDeferred) {
				throw new ModelsError("provider", `Provider ${input.id} cannot cancel deferred responses for "${model.api}"`);
			}
			await implementation.cancelDeferred(model, handle, options);
		};
	}

	return provider;
}

/** Runtime-checked narrowing for dynamically looked-up models. */
export function hasApi<TApi extends Api>(model: Model<Api>, api: TApi): model is Model<TApi> {
	return model.api === api;
}

export function calculateCost<TApi extends Api>(model: Model<TApi>, usage: Usage): Usage["cost"] {
	const inputTokens = usage.input + usage.cacheRead + usage.cacheWrite;
	let rates: ModelCostRates = model.cost;
	let matchedThreshold = -1;
	for (const tier of model.cost.tiers ?? []) {
		if (inputTokens > tier.inputTokensAbove && tier.inputTokensAbove > matchedThreshold) {
			rates = tier;
			matchedThreshold = tier.inputTokensAbove;
		}
	}

	const longWrite = usage.cacheWrite1h ?? 0;
	const shortWrite = usage.cacheWrite - longWrite;
	usage.cost.input = (rates.input / 1000000) * usage.input;
	usage.cost.output = (rates.output / 1000000) * usage.output;
	usage.cost.cacheRead = (rates.cacheRead / 1000000) * usage.cacheRead;
	usage.cost.cacheWrite = (rates.cacheWrite * shortWrite + rates.input * 2 * longWrite) / 1000000;
	usage.cost.total = usage.cost.input + usage.cost.output + usage.cost.cacheRead + usage.cost.cacheWrite;
	return usage.cost;
}

const EXTENDED_THINKING_LEVELS: ModelThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

export function getSupportedThinkingLevels<TApi extends Api>(model: Model<TApi>): ModelThinkingLevel[] {
	if (!model.reasoning) return ["off"];

	return EXTENDED_THINKING_LEVELS.filter((level) => {
		const mapped = model.thinkingLevelMap?.[level];
		if (mapped === null) return false;
		if (level === "xhigh" || level === "max") return mapped !== undefined;
		return true;
	});
}

export function clampThinkingLevel<TApi extends Api>(model: Model<TApi>, level: ModelThinkingLevel): ModelThinkingLevel {
	const availableLevels = getSupportedThinkingLevels(model);
	if (availableLevels.includes(level)) return level;

	const requestedIndex = EXTENDED_THINKING_LEVELS.indexOf(level);
	if (requestedIndex === -1) return availableLevels[0] ?? "off";

	for (let i = requestedIndex; i < EXTENDED_THINKING_LEVELS.length; i++) {
		const candidate = EXTENDED_THINKING_LEVELS[i];
		if (availableLevels.includes(candidate)) return candidate;
	}
	for (let i = requestedIndex - 1; i >= 0; i--) {
		const candidate = EXTENDED_THINKING_LEVELS[i];
		if (availableLevels.includes(candidate)) return candidate;
	}
	return availableLevels[0] ?? "off";
}

/** Check if two models are equal by comparing both their id and provider. */
export function modelsAreEqual<TApi extends Api>(a: Model<TApi> | null | undefined, b: Model<TApi> | null | undefined): boolean {
	if (!a || !b) return false;
	return a.id === b.id && a.provider === b.provider;
}
