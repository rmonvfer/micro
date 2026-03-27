// What `@earendil-works/pi-agent-core` and `@mariozechner/pi-agent-core` resolve to for a
// pi extension running under micro.
//
// Evidence, not guesswork: every extension under
// `pi/packages/coding-agent/examples/extensions` that imports from
// `@earendil-works/pi-agent-core` imports only types (`AgentMessage`, `AgentToolResult`,
// `ThinkingLevel` — see `plan-mode/index.ts`, `subagent/index.ts`, `handoff.ts`). Those
// need nothing here: Bun erases `import type` before module resolution runs, the same as
// every other type-only import from this SDK. The one extension that looks agent-like,
// `subagent/index.ts`, spawns a separate `pi` child process per subagent rather than
// driving pi-agent-core's `Agent` class in this process — so even that extension takes the
// type-only path.
//
// What's real below: `uuidv7` (pi-agent-core's own `index.ts` re-exports it from pi-ai
// unchanged) and the telemetry primitives pi-agent-core's `index.ts` re-exports from
// `@earendil-works/pi-telemetry` (`NOOP_TELEMETRY_CONTEXT`, `InMemoryTelemetryContext`,
// `defineTelemetrySchema`, `createTypedSpanStarter`) — pure in-memory span recording, no
// network, no filesystem, ported faithfully from pi-telemetry's own source even though
// pi-telemetry itself is a third package outside this shim's two assigned modules.
//
// `Agent` constructs for real — nothing about building one needs pi's runtime — but every
// method on it throws the moment it is actually asked to do anything: running a turn means
// sending a real request to a real model, and resolving credentials for that is
// deliberately kept out of the extension host's reach (see the note on `modelRegistry` in
// `../../context.ts`), the same boundary drawn everywhere else this layer meets it. No
// real extension constructs one directly today (see above), but pi-subagents' watchdog
// review pattern is exactly the shape that would, so it fails with a specific, named
// reason rather than "does not provide an export named Agent".

export { uuidv7 } from "../pi-ai/index.ts";

export type AttributeValue = string | number | boolean | readonly string[] | readonly number[] | readonly boolean[];

export interface SpanAttributes {
	[name: string]: AttributeValue | undefined;
}

export interface SpanOptions {
	name: string;
	attributes?: SpanAttributes;
}

export type SpanStatus = { status: "ok" } | { status: "error"; error?: { name: string; message: string } };

export interface TelemetryContext {
	startSpan<T>(options: SpanOptions, callback: (span: TelemetrySpan) => T | Promise<T>): Promise<T>;
}

export interface TelemetrySpan extends TelemetryContext {
	addEvent(name: string, attributes?: SpanAttributes): void;
	setAttributes(attributes: SpanAttributes): void;
	setStatus(status: SpanStatus): void;
}

function startNoopSpan<T>(_options: SpanOptions, callback: (span: TelemetrySpan) => T | Promise<T>): Promise<T> {
	try {
		return Promise.resolve(callback(noopTelemetrySpan));
	} catch (error) {
		return Promise.reject(error);
	}
}

const noopTelemetrySpan: TelemetrySpan = {
	startSpan: startNoopSpan,
	addEvent: () => {},
	setAttributes: () => {},
	setStatus: () => {},
};
Object.freeze(noopTelemetrySpan);

/** Shared telemetry context used when an application does not provide one. */
export const NOOP_TELEMETRY_CONTEXT: TelemetryContext = noopTelemetrySpan;

export interface TelemetrySchemaDefinition {
	version: number;
	spans: Record<string, unknown>;
}

/** Typed identity helper for serializable telemetry schema data. */
export function defineTelemetrySchema<const T extends TelemetrySchemaDefinition>(schema: T): T {
	return schema;
}

export interface RecordedTelemetryEvent {
	readonly name: string;
	readonly attributes: Readonly<SpanAttributes>;
}

export interface RecordedTelemetrySpan {
	readonly id: number;
	readonly parentId: number | null;
	readonly name: string;
	readonly attributes: Readonly<SpanAttributes>;
	readonly events: readonly RecordedTelemetryEvent[];
	readonly status: SpanStatus;
	readonly settled: boolean;
	readonly endSequence?: number;
}

interface MutableRecordedTelemetryEvent {
	name: string;
	attributes: SpanAttributes;
}

interface MutableRecordedTelemetrySpan {
	id: number;
	parentId: number | null;
	name: string;
	attributes: SpanAttributes;
	events: MutableRecordedTelemetryEvent[];
	status: SpanStatus;
	explicitStatus: boolean;
	settled: boolean;
	endSequence?: number;
}

interface InMemoryTelemetryState {
	spans: MutableRecordedTelemetrySpan[];
	nextSpanId: number;
	nextEndSequence: number;
}

function copyAttributeValue(value: AttributeValue): AttributeValue {
	return Array.isArray(value) ? ([...value] as AttributeValue) : value;
}

function copyAttributes(attributes?: SpanAttributes): SpanAttributes {
	const copy: SpanAttributes = {};
	if (!attributes) return copy;
	for (const [name, value] of Object.entries(attributes)) {
		if (value !== undefined) copy[name] = copyAttributeValue(value);
	}
	return copy;
}

function mergeAttributes(current: SpanAttributes, attributes: SpanAttributes): SpanAttributes {
	const merged = copyAttributes(current);
	for (const [name, value] of Object.entries(attributes)) {
		if (value !== undefined) merged[name] = copyAttributeValue(value);
	}
	return merged;
}

function copyStatus(status: SpanStatus): SpanStatus {
	if (status.status === "ok") return { status: "ok" };
	return status.error ? { status: "error", error: { name: status.error.name, message: status.error.message } } : { status: "error" };
}

function automaticErrorStatus(error: unknown): SpanStatus {
	try {
		if (error instanceof Error) {
			return { status: "error", error: { name: error.name, message: error.message } };
		}
	} catch {
		// Error inspection is passive. Fall through to an error status without details.
	}
	return { status: "error" };
}

function settleSpan(state: InMemoryTelemetryState, span: MutableRecordedTelemetrySpan, failed: boolean, error?: unknown): void {
	if (span.settled) return;
	if (failed && !span.explicitStatus) span.status = automaticErrorStatus(error);
	span.settled = true;
	span.endSequence = state.nextEndSequence++;
}

function createSpan(
	state: InMemoryTelemetryState,
	parent: MutableRecordedTelemetrySpan | undefined,
	options: SpanOptions,
): MutableRecordedTelemetrySpan {
	return {
		id: state.nextSpanId++,
		parentId: parent?.id ?? null,
		name: options.name,
		attributes: copyAttributes(options.attributes),
		events: [],
		status: { status: "ok" },
		explicitStatus: false,
		settled: false,
	};
}

function startInMemorySpan<T>(
	state: InMemoryTelemetryState,
	parent: MutableRecordedTelemetrySpan | undefined,
	options: SpanOptions,
	callback: (span: TelemetrySpan) => T | Promise<T>,
): Promise<T> {
	if (parent?.settled) return NOOP_TELEMETRY_CONTEXT.startSpan(options, callback);

	let recordedSpan: MutableRecordedTelemetrySpan;
	try {
		recordedSpan = createSpan(state, parent, options);
		state.spans.push(recordedSpan);
	} catch {
		return NOOP_TELEMETRY_CONTEXT.startSpan(options, callback);
	}

	const span: TelemetrySpan = {
		startSpan: <Result>(childOptions: SpanOptions, childCallback: (child: TelemetrySpan) => Result | Promise<Result>) =>
			startInMemorySpan(state, recordedSpan, childOptions, childCallback),
		addEvent(name, attributes) {
			if (recordedSpan.settled) return;
			try {
				recordedSpan.events.push({ name, attributes: copyAttributes(attributes) });
			} catch {
				// Recording is passive. Ignore malformed or unreadable telemetry payloads.
			}
		},
		setAttributes(attributes) {
			if (recordedSpan.settled) return;
			try {
				recordedSpan.attributes = mergeAttributes(recordedSpan.attributes, attributes);
			} catch {
				// Recording is passive. Ignore malformed or unreadable telemetry payloads.
			}
		},
		setStatus(status) {
			if (recordedSpan.settled) return;
			try {
				recordedSpan.status = copyStatus(status);
				recordedSpan.explicitStatus = true;
			} catch {
				// Recording is passive. Ignore malformed or unreadable telemetry payloads.
			}
		},
	};

	let result: T | Promise<T>;
	try {
		result = callback(span);
	} catch (error) {
		settleSpan(state, recordedSpan, true, error);
		return Promise.reject(error);
	}

	return Promise.resolve(result).then(
		(value) => {
			settleSpan(state, recordedSpan, false);
			return value;
		},
		(error: unknown) => {
			settleSpan(state, recordedSpan, true, error);
			throw error;
		},
	);
}

/** Backend-neutral reference implementation that records spans in process memory. Create
 *  a fresh instance to isolate tests or independent recording scopes. */
export class InMemoryTelemetryContext implements TelemetryContext {
	private readonly state: InMemoryTelemetryState = { spans: [], nextSpanId: 1, nextEndSequence: 1 };

	startSpan<T>(options: SpanOptions, callback: (span: TelemetrySpan) => T | Promise<T>): Promise<T> {
		return startInMemorySpan(this.state, undefined, options, callback);
	}

	/** Returns detached snapshots in span-start order. */
	getSpans(): readonly RecordedTelemetrySpan[] {
		return this.state.spans.map((span) => ({
			id: span.id,
			parentId: span.parentId,
			name: span.name,
			attributes: copyAttributes(span.attributes),
			events: span.events.map((event) => ({ name: event.name, attributes: copyAttributes(event.attributes) })),
			status: copyStatus(span.status),
			settled: span.settled,
			...(span.endSequence === undefined ? {} : { endSequence: span.endSequence }),
		}));
	}
}

/**
 * Bind an explicit parent telemetry context to a callable span-starter. pi-agent-core's
 * own version infers a per-schema-name overload set for compile-time attribute checking;
 * that type machinery is `import type`-only (free via elision) so this keeps the runtime
 * behavior — bind once, call with any span name the context accepts — without reproducing
 * the overload inference itself.
 */
export function createTypedSpanStarter(
	telemetryContext: TelemetryContext,
	_schemas: readonly TelemetrySchemaDefinition[],
): (name: string, attributes: SpanAttributes, callback: (span: TelemetrySpan, startChildSpan: unknown) => unknown) => Promise<unknown> {
	const startSpan = (
		name: string,
		attributes: SpanAttributes,
		callback: (span: TelemetrySpan, startChildSpan: unknown) => unknown,
	): Promise<unknown> => telemetryContext.startSpan({ name, attributes }, (span) => callback(span, bindChild(span)));
	const bindChild = (context: TelemetryContext) => createTypedSpanStarter(context, _schemas);
	return startSpan;
}

function agentRunUnavailable(method: string): never {
	throw new Error(
		`pi-agent-core's Agent.${method}() would send a real request to a real model. Resolving credentials for that is deliberately kept out of the extension host's reach — micro does not hand extensions a way to authenticate as the user. An extension that needs a model call should ask through the ordinary extension API instead (the object passed to export default (micro) => {...}), not by constructing its own Agent.`,
	);
}

export class Agent {
	constructor(_options?: unknown) {
		// Constructing an Agent is bookkeeping — options are read, nothing is sent
		// anywhere — so it succeeds for real. What it returns is a Proxy rather than
		// `this`: pi-agent-core's Agent has a wide method surface (`run`, `ask`,
		// `prompt`, `stream`, event subscriptions, and more depending on version) and
		// this layer would rather every one of them fail the same specific way than
		// silently answer `undefined` for whichever names it did not happen to list.
		return new Proxy(
			{},
			{
				get(_target, property) {
					if (typeof property !== "string") return undefined;
					return (..._args: unknown[]) => agentRunUnavailable(property);
				},
			},
		);
	}
}
