

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
				
			}
		},
		setAttributes(attributes) {
			if (recordedSpan.settled) return;
			try {
				recordedSpan.attributes = mergeAttributes(recordedSpan.attributes, attributes);
			} catch {
				
			}
		},
		setStatus(status) {
			if (recordedSpan.settled) return;
			try {
				recordedSpan.status = copyStatus(status);
				recordedSpan.explicitStatus = true;
			} catch {
				
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

/** Backend-neutral reference implementation that records spans in process memory. */
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

/** Bind an explicit parent telemetry context to a callable span-starter. */
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

/** Constructing an Agent is bookkeeping and succeeds, but asking one to do anything would send a
 *  real request to a real model, and micro keeps the credentials for that out of the extension
 *  host's reach. Saying so beats answering `undefined`. */
function agentRunUnavailable(method: string): never {
	throw new Error(
		`pi-agent-core Agent.${method}() would send a real request to a real model, and micro keeps model credentials out of the extension host's reach. Ask through the extension API instead — the object passed to export default (micro) => {...}.`,
	);
}

export class Agent {
	constructor(_options?: unknown) {
		
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
