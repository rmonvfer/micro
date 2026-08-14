// Running what an extension registered.
//
// A tool the model called, a command the user typed, a renderer asked for lines: each is a
// function an extension handed over, and each answer has to be turned back into something
// micro can act on.

import { type Component, dispose as disposeComponent, registerComponent } from "./host-components.ts";
import { type Json, send } from "./host-wire.ts";

/** One piece of what a tool answers with. */
export interface TextContent {
	type: "text";
	text: string;
}

/** An image a tool hands back for the model to look at, base64-encoded. */
export interface ImageContent {
	type: "image";
	data: string;
	mimeType: string;
}

/** What execute() resolves with, and what onUpdate() is called with along the way — pi's
 * AgentToolResult, member for member. */
export interface ToolResult<TDetails = unknown> {
	content: (TextContent | ImageContent)[];
	details?: TDetails;
	/** Nowhere to land on this side: a micro tool result carries content and nothing else,
	 * so a usage figure a tool reports for itself is read here and travels no further. */
	usage?: Json;
	/** Same story as usage — accepted so a pi-shaped result never fails for carrying it,
	 * but micro's agent loop does not yet let a tool call introduce new tools mid-run. */
	addedToolNames?: string[];
	/** And again: accepted, not honored. micro always finishes every call in a batch
	 * rather than stopping the batch early on one tool's say-so. */
	terminate?: boolean;
}

/** Called with the result so far, as many times as a tool likes before it settles. Calls
 * made after execute() has resolved or thrown are dropped — nobody is listening anymore. */
export type ToolUpdateCallback<TDetails = unknown> = (partial: ToolResult<TDetails>) => void;

/**
 * Tool definition for registerTool(), matching pi's contract member for member so a tool
 * written for pi runs here unchanged.
 *
 * label, promptSnippet, promptGuidelines, constrainedSampling, and executionMode describe
 * how pi's own process presents or schedules a call: a label for its TUI, a snippet and
 * guideline bullets for its system prompt, a sampling directive for its provider layer, a
 * batching hint for its loop. micro builds a provider request with no constrained-sampling
 * hook and always runs a batch of calls together rather than choosing sequential or
 * parallel per tool. These members are still read here and carried all the way to micro
 * instead of being dropped at this boundary, so an extension that sets one is never
 * silently ignored — but nothing on the Rust side consumes them yet.
 *
 * renderCall, renderResult and renderShell are real: see `renderToolCall`/
 * `renderToolResult` below. A Component itself never crosses the wire — it is three
 * methods, and a method cannot be written down as JSON — but a call to it by id can, which
 * is the same shape execute() already uses for a tool that stays in this process and is
 * driven by id rather than shipped over.
 */
export interface ToolDefinition<TDetails = unknown> {
	name: string;
	label?: string;
	description: string;
	promptSnippet?: string;
	promptGuidelines?: string[];
	parameters?: Json;
	constrainedSampling?: false | Json;
	renderShell?: "default" | "self";
	/** Reshape raw arguments before execute() sees them. A schema validation step would
	 * normally sit between this and execute() — micro runs none, for this tool or any
	 * other, so what this returns is exactly what execute() receives. Thrown, its message
	 * becomes the tool's error result rather than reaching execute() at all. */
	prepareArguments?: (args: unknown) => unknown;
	executionMode?: "sequential" | "parallel";
	execute: (
		toolCallId: string,
		params: unknown,
		signal: AbortSignal | undefined,
		onUpdate: ToolUpdateCallback<TDetails> | undefined,
		ctx: unknown,
	) => unknown | Promise<unknown>;
	/** Draw the call itself — the header a reader sees while, and after, a tool runs. */
	renderCall?: (args: unknown, theme: Json, context: ToolRenderContext) => Component;
	/** Draw what the call came back with. */
	renderResult?: (
		result: ToolResult<TDetails>,
		options: { expanded: boolean; isPartial: boolean },
		theme: Json,
		context: ToolRenderContext,
	) => Component;
}

/** What renderCall/renderResult are called with, matching pi's ToolRenderContext member for
 * member except for the members a live remote component cannot honestly offer — see the
 * note above `toolTheme`. */
export interface ToolRenderContext {
	args: unknown;
	toolCallId: string;
	/** Tell micro this row's rendering is stale, on this side's own schedule rather than
	 * only when args or a result changed — a spinner mid-animation, a countdown, anything a
	 * renderer wants to keep moving between the moments micro would otherwise ask again. */
	invalidate: () => void;
	/** Whatever this same renderer — call or result, kept separate — returned last time for
	 * this tool call, for a renderer that wants to reuse or diff against its own object
	 * rather than build fresh every time. */
	lastComponent: Component | undefined;
	/** Scratch state this renderer owns across every call for this one tool row. Shared
	 * between renderCall and renderResult for the same row, the way pi shares it. */
	state: Json;
	cwd: string;
	executionStarted: boolean;
	argsComplete: boolean;
	isPartial: boolean;
	expanded: boolean;
	showImages: boolean;
	isError: boolean;
}

/** Tool calls running right now, keyed the same way micro asked for them, so an abort from
 * micro can find the right one to stop. */
const running = new Map<string, AbortController>();

/**
 * Run one of an extension's tools under pi's contract: an id it can use to tell its own
 * calls apart, a signal that fires if micro gives up on this call, and a way to say what it
 * has done before it is done doing it.
 *
 * The id passed as toolCallId is the one this host and micro correlate the call by, not
 * the id the model's own tool-call block carries — micro's tool trait does not thread the
 * model's id down to an individual tool, only the id used for reporting from the loop
 * around it. It is still unique to this call and stable for its whole run, which is what
 * the contract actually needs it for.
 */
export async function runTool(id: string, tool: ToolDefinition, rawArguments: Json, ctx: unknown): Promise<void> {
	let args: unknown = rawArguments;
	if (tool.prepareArguments) {
		try {
			args = tool.prepareArguments(rawArguments);
		} catch (error) {
			send({ type: "tool_result", id, error: reason(error) });
			return;
		}
	}

	const controller = new AbortController();
	running.set(id, controller);

	// Nothing sent after the call settles is going anywhere: the id it would be tagged
	// with no longer means anything to whoever was waiting on it.
	let settled = false;
	const onUpdate: ToolUpdateCallback = (partial) => {
		if (settled) return;
		send({ type: "tool_update", id, ...toolAnswer(partial) });
	};

	try {
		const output = await tool.execute(id, args, controller.signal, onUpdate, ctx);
		settled = true;
		send({ type: "tool_result", id, ...toolAnswer(output) });
	} catch (error) {
		settled = true;
		send({ type: "tool_result", id, error: reason(error) });
	} finally {
		running.delete(id);
	}
}

/**
 * micro giving up on a call — a turn that was aborted, or one that ran past its own
 * patience. A call already settled, or one this host never started, is simply not found:
 * there is nothing left here to stop.
 */
export function abortTool(id: string): void {
	running.get(id)?.abort();
}

/**
 * The theme renderCall/renderResult receive as their second argument.
 *
 * pi's Theme is a constructor argument to the renderer, not something reached through
 * `ctx.ui` — a tool's renderer draws with whatever it is handed here, before a context
 * object with a `ui` member ever comes into it. Rather than a second copy of micro's full
 * palette (`ctx.ui.theme` already carries one, built for a different call site), colors are
 * mapped to the basic ANSI palette by what they mean: an unstyled token still reads
 * correctly, in either color scheme, without a second source of truth to keep in sync with
 * `crates/micro-tui/src/theme/palette.rs`.
 */
const FG_ANSI: Record<string, number> = {
	error: 31,
	success: 32,
	warning: 33,
	accent: 34,
	border: 34,
	borderAccent: 34,
	mdLink: 34,
	muted: 90,
	dim: 90,
	toolOutput: 90,
	mdQuote: 90,
	text: 37,
	toolTitle: 37,
};
const BG_ANSI: Record<string, number> = {
	toolErrorBg: 41,
	toolSuccessBg: 42,
	toolPendingBg: 100,
	selectedBg: 44,
};

const toolTheme: Json = {
	name: "micro",
	fg(color: string, text: string): string {
		const code = FG_ANSI[color];
		return code === undefined ? text : `\x1b[${code}m${text}\x1b[39m`;
	},
	bg(color: string, text: string): string {
		const code = BG_ANSI[color];
		return code === undefined ? text : `\x1b[${code}m${text}\x1b[49m`;
	},
};

/** What is kept per tool call for its renderers: shared scratch state, and the last
 * component each renderer returned, tracked apart because pi tracks them apart. */
interface ToolRenderRow {
	state: Json;
	callComponent?: Component;
	callComponentId?: string;
	resultComponent?: Component;
	resultComponentId?: string;
}
const toolRenderRows = new Map<string, ToolRenderRow>();

function rowFor(toolCallId: string): ToolRenderRow {
	let row = toolRenderRows.get(toolCallId);
	if (!row) {
		row = { state: {} };
		toolRenderRows.set(toolCallId, row);
	}
	return row;
}

/** What running a tool's renderCall/renderResult answers back to micro. */
export interface ToolRenderAnswer {
	/** Set only when the renderer ran and returned a component. */
	componentId?: string;
	/** False either because the tool declared no renderer of this kind, or because the one
	 * it declared threw — `error` tells the two apart. */
	supported: boolean;
	error?: string;
}

/** Everything renderCall and renderResult are both given beyond their own first argument,
 * read once from what micro sent over the wire. */
interface RenderFields {
	toolCallId: string;
	cwd: string;
	executionStarted: boolean;
	argsComplete: boolean;
	isPartial: boolean;
	expanded: boolean;
	showImages: boolean;
	isError: boolean;
}

function contextFor(fields: RenderFields, row: ToolRenderRow, args: unknown, kind: "call" | "result"): ToolRenderContext {
	return {
		args,
		toolCallId: fields.toolCallId,
		invalidate: () => {
			const id = kind === "call" ? row.callComponentId : row.resultComponentId;
			if (id) {
				send({ type: "component_changed", componentId: id });
			}
		},
		lastComponent: kind === "call" ? row.callComponent : row.resultComponent,
		state: row.state,
		cwd: fields.cwd,
		executionStarted: fields.executionStarted,
		argsComplete: fields.argsComplete,
		isPartial: fields.isPartial,
		expanded: fields.expanded,
		showImages: fields.showImages,
		isError: fields.isError,
	};
}

/** Run a tool's renderCall, registering whatever Component it returns and retiring the one
 * from this same tool call's previous invocation — a renderer is asked again on every
 * state change (args streaming in, execution starting), and nothing should keep a stale
 * component registered once a fresher one exists to answer for it. */
export function renderToolCall(tool: ToolDefinition, args: unknown, fields: RenderFields): ToolRenderAnswer {
	if (!tool.renderCall) {
		return { supported: false };
	}
	const row = rowFor(fields.toolCallId);
	try {
		const component = tool.renderCall(args, toolTheme, contextFor(fields, row, args, "call"));
		if (row.callComponentId) {
			disposeComponent(row.callComponentId);
		}
		const handle = registerComponent(component);
		row.callComponent = component;
		row.callComponentId = handle.id;
		return { supported: true, componentId: handle.id };
	} catch (error) {
		return { supported: false, error: reason(error) };
	}
}

/** The renderResult counterpart to `renderToolCall` — same retire-then-register handling,
 * called instead once a result (partial or final) has arrived to draw. */
export function renderToolResult(
	tool: ToolDefinition,
	result: ToolResult,
	options: { expanded: boolean; isPartial: boolean },
	args: unknown,
	fields: RenderFields,
): ToolRenderAnswer {
	if (!tool.renderResult) {
		return { supported: false };
	}
	const row = rowFor(fields.toolCallId);
	try {
		const component = tool.renderResult(result, options, toolTheme, contextFor(fields, row, args, "result"));
		if (row.resultComponentId) {
			disposeComponent(row.resultComponentId);
		}
		const handle = registerComponent(component);
		row.resultComponent = component;
		row.resultComponentId = handle.id;
		return { supported: true, componentId: handle.id };
	} catch (error) {
		return { supported: false, error: reason(error) };
	}
}

/** Forget everything kept for a tool call's renderers, and retire whatever components are
 * still registered for it. Not tied to any lifecycle event micro sends today — a session
 * that never calls this simply keeps every tool call's rendering state for its own
 * lifetime, which ends anyway the moment the process does — but real code rather than a gap
 * for whichever caller does want to reclaim it, such as a session that forks or resets. */
export function forgetToolRender(toolCallId: string): void {
	const row = toolRenderRows.get(toolCallId);
	if (!row) {
		return;
	}
	if (row.callComponentId) {
		disposeComponent(row.callComponentId);
	}
	if (row.resultComponentId) {
		disposeComponent(row.resultComponentId);
	}
	toolRenderRows.delete(toolCallId);
}

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** Whatever a renderer returned, as the lines micro will draw. */
export function renderedLines(value: unknown): string[] {
	if (typeof value === "string") {
		return value.split("\n");
	}
	if (Array.isArray(value)) {
		return value.map((line) => String(line));
	}
	return [];
}

/**
 * Whatever a tool returned or reported mid-run, as the content array micro reads.
 *
 * A tool may answer with a bare string, an array of content blocks, or the full shape
 * pi's contract describes. Whatever form it took, what reaches micro is always
 * `{ content: [...] }` plus whatever else rode along with it, so a final result and a
 * partial update are read the same way on the other end.
 */
export function toolAnswer(value: unknown): Json {
	if (typeof value === "string") {
		return { content: [{ type: "text", text: value }] };
	}
	if (Array.isArray(value)) {
		return { content: normalizeContent(value) };
	}
	if (value && typeof value === "object" && "content" in (value as Json)) {
		const shape = value as Json;
		const answer: Json = { content: normalizeContent(shape.content) };
		// Carried as-is rather than reshaped: nothing on the Rust side reads these yet
		// (see ToolResult), but a tool that set them should not see them vanish either.
		for (const carried of ["details", "usage", "addedToolNames", "terminate"] as const) {
			if (carried in shape) {
				answer[carried] = shape[carried];
			}
		}
		return answer;
	}
	if (value && typeof value === "object") {
		// Shaped like an object but not a ToolResult: read as JSON, the way an
		// unrecognized answer has always been carried rather than lost.
		return { content: [{ type: "text", text: JSON.stringify(value) }] };
	}
	return { content: [{ type: "text", text: value === undefined ? "" : String(value) }] };
}

function normalizeContent(value: unknown): Json[] {
	if (!Array.isArray(value)) {
		return [{ type: "text", text: value === undefined ? "" : String(value) }];
	}
	return value.map((block) => {
		if (block && typeof block === "object" && (block as Json).type === "image") {
			const image = block as Json;
			return { type: "image", data: image.data, mimeType: image.mimeType };
		}
		if (block && typeof block === "object" && "text" in (block as Json)) {
			return { type: "text", text: String((block as Json).text) };
		}
		return { type: "text", text: typeof block === "string" ? block : JSON.stringify(block) };
	});
}
