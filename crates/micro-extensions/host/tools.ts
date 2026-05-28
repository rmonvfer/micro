

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


export interface ToolResult<TDetails = unknown> {
	content: (TextContent | ImageContent)[];
	details?: TDetails;
	
	usage?: Json;
	/** Same story as usage. */
	addedToolNames?: string[];
	/** And again: accepted, not honored. */
	terminate?: boolean;
}

/** Called with the result so far, as many times as a tool likes before it settles. */
export type ToolUpdateCallback<TDetails = unknown> = (partial: ToolResult<TDetails>) => void;

/**
 * Tool definition for registerTool(), matching pi's contract member for member so a tool written
 * for pi runs here unchanged.
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
	/** Reshape raw arguments before execute() sees them. */
	prepareArguments?: (args: unknown) => unknown;
	executionMode?: "sequential" | "parallel";
	execute: (
		toolCallId: string,
		params: unknown,
		signal: AbortSignal | undefined,
		onUpdate: ToolUpdateCallback<TDetails> | undefined,
		ctx: unknown,
	) => unknown | Promise<unknown>;
	
	renderCall?: (args: unknown, theme: Json, context: ToolRenderContext) => Component;
	/** Draw what the call came back with. */
	renderResult?: (
		result: ToolResult<TDetails>,
		options: { expanded: boolean; isPartial: boolean },
		theme: Json,
		context: ToolRenderContext,
	) => Component;
}


export interface ToolRenderContext {
	args: unknown;
	toolCallId: string;
	
	invalidate: () => void;
	/** Whatever this same renderer. */
	lastComponent: Component | undefined;
	/** Scratch state this renderer owns across every call for this one tool row. */
	state: Json;
	cwd: string;
	executionStarted: boolean;
	argsComplete: boolean;
	isPartial: boolean;
	expanded: boolean;
	showImages: boolean;
	isError: boolean;
}


const running = new Map<string, AbortController>();


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


export function abortTool(id: string): void {
	running.get(id)?.abort();
}

/** The theme renderCall/renderResult receive as their second argument. */
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
	/**
 * False either because the tool declared no renderer of this kind, or because the one it declared
 * threw.
 */
	supported: boolean;
	error?: string;
}


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

/** The renderResult counterpart to `renderToolCall`. */
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

/**
 * Forget everything kept for a tool call's renderers, and retire whatever components are still
 * registered for it.
 */
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

/** Whatever a tool returned or reported mid-run, as the content array micro reads. */
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
		
		for (const carried of ["details", "usage", "addedToolNames", "terminate"] as const) {
			if (carried in shape) {
				answer[carried] = shape[carried];
			}
		}
		return answer;
	}
	if (value && typeof value === "object") {
		
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
