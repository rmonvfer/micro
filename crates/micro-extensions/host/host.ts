

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import * as components from "./host-components.ts";
import {
	activeTools,
	allTools,
	commands,
	contextFor,
	contextFrom,
	located,
	noteActiveTools,
	noted,
	sessionName,
	snapshot,
	thinkingLevel,
	where,
} from "./host-context.ts";
import {
	abortTool,
	renderToolCall,
	renderToolResult,
	runTool,
	type ToolDefinition,
	type ToolRenderAnswer,
	type ToolResult,
} from "./host-tools.ts";
import {
	type AutocompleteItem,
	dispatchApplyCompletion,
	dispatchSuggestions,
	dispatchTerminalInput,
	noteEditorText,
	uiFor,
} from "./host-ui.ts";
import { answered, type Json, send, wireFor } from "./host-wire.ts";

interface RegisteredCommand {
	description?: string;
	handler: (args: string, ctx: unknown) => unknown | Promise<unknown>;
}

interface Registration {
	path: string;
	/** What the extension itself said it may do, when it exported a `capabilities` list. */
	capabilities?: string[];
	/** What to call when this extension is let go, when it exported one. */
	deactivate?: () => unknown | Promise<unknown>;
	tools: Map<string, ToolDefinition>;
	commands: Map<string, RegisteredCommand>;
	handlers: Map<string, Array<(event: Json, ctx: unknown) => unknown>>;
	flags: Map<string, { description?: string; type: "boolean" | "string"; default?: boolean | string }>;
	shortcuts: Map<string, { description?: string; handler: (ctx: unknown) => unknown }>;
	providers: Map<string, Json>;
	renderers: Map<string, (data: unknown, options: { width: number }) => unknown>;
	markdownTransformers: Array<(markdown: string, context: Json) => string>;
}

/** Messages passed between extensions, and between an extension and itself. */
const bus = new Map<string, Array<(data: unknown) => void>>();

const events = {
	emit(channel: string, data: unknown): void {
		for (const handler of bus.get(channel) ?? []) {
			try {
				handler(data);
			} catch (error) {
				console.error(`an ${channel} handler failed: ${error}`);
			}
		}
	},

	/** Listen, and take the returned function to stop listening. */
	on(channel: string, handler: (data: unknown) => void): () => void {
		const handlers = bus.get(channel) ?? [];
		handlers.push(handler);
		bus.set(channel, handlers);
		return () => {
			const remaining = (bus.get(channel) ?? []).filter((kept) => kept !== handler);
			bus.set(channel, remaining);
		};
	},
};

/** Where this host is running, for the context handed to every handler. */
const loaded: Registration[] = [];
const failures: Array<{ path: string; error: string }> = [];
const flagValues = new Map<string, boolean | string>();

/** The API an extension is handed. */
function apiFor(registration: Registration) {
	const { ask, send } = wireFor(registration.path);
	return {
		events,

		on(event: string, handler: (event: Json, ctx: unknown) => unknown): void {
			const handlers = registration.handlers.get(event) ?? [];
			handlers.push(handler);
			registration.handlers.set(event, handlers);
		},

		registerTool(tool: ToolDefinition): void {
			registration.tools.set(tool.name, tool);
		},

		registerCommand(name: string, options: RegisteredCommand): void {
			registration.commands.set(name, options);
		},

		registerShortcut(shortcut: string, options: { description?: string; handler: (ctx: unknown) => unknown }): void {
			registration.shortcuts.set(shortcut, options);
		},

		registerFlag(
			name: string,
			options: { description?: string; type: "boolean" | "string"; default?: boolean | string },
		): void {
			registration.flags.set(name, options);
			if (options.default !== undefined && !flagValues.has(name)) {
				flagValues.set(name, options.default);
			}
		},

		getFlag(name: string): boolean | string | undefined {
			return flagValues.get(name);
		},

		/** Draw a custom message of this type yourself. */
		registerMessageRenderer(
			customType: string,
			renderer: (data: unknown, options: { width: number }) => unknown,
		): void {
			registration.renderers.set(customType, renderer);
		},

		/** Draw a custom entry of this type yourself. */
		registerEntryRenderer(
			customType: string,
			renderer: (data: unknown, options: { width: number }) => unknown,
		): void {
			registration.renderers.set(customType, renderer);
		},

		/** Rewrite user and assistant markdown before it is drawn. */
		registerMarkdownTransformer(transformer: (markdown: string, context: Json) => string): void {
			registration.markdownTransformers.push(transformer);
		},

		/** Declare a provider, or change one micro already knows. */
		registerProvider(name: string, config: Json): void {
			registration.providers.set(name, config);
		},

		unregisterProvider(name: string): void {
			registration.providers.delete(name);
		},

		
		sendUserMessage(content: string, options?: Json): void {
			send({ type: "action", action: "send_user_message", content, options: options ?? {} });
		},

		sendMessage(message: Json, options?: Json): void {
			send({ type: "action", action: "send_message", message, options: options ?? {} });
		},

		/** Name the session. */
		setSessionName(name: string): void {
			send({ type: "action", action: "set_session_name", name });
		},

		async exec(command: string, args: string[], options?: Json): Promise<Json> {
			return ask({ type: "request", request: "exec", command, args, options: options ?? {} });
		},

		/** The tools the model is being offered. */
		getActiveTools(): string[] {
			return activeTools();
		},

		/** Choose which tools are offered from here on. */
		setActiveTools(toolNames: string[]): void {
			noteActiveTools(toolNames);
			send({ type: "action", action: "set_active_tools", toolNames });
		},

		/** Keep something in the session that the model never sees. */
		appendEntry(customType: string, data?: unknown): void {
			send({ type: "action", action: "append_entry", customType, data });
		},

		/** Read back everything this or any other extension kept. */
		async getEntries(): Promise<Json[]> {
			const answer = await ask({ type: "request", request: "get_entries" });
			return (answer.entries as Json[]) ?? [];
		},

		/** Name an entry, or take its name away by passing nothing. */
		setLabel(entryId: string, label?: string): void {
			send({ type: "action", action: "set_label", entryId, label });
		},

		
		getAllTools(): Json[] {
			return allTools();
		},

		/** Every command that can be typed, from micro's own and from every extension. */
		getCommands(): Json[] {
			return commands();
		},

		/** What the session is called, if anything. */
		getSessionName(): string | undefined {
			return sessionName();
		},

		async getModel(): Promise<Json | undefined> {
			const answer = await ask({ type: "request", request: "get_model" });
			return answer.model as Json | undefined;
		},

		/** Change the model, answering whether it could be. */
		async setModel(model: Json | string): Promise<boolean> {
			const answer = await ask({ type: "request", request: "set_model", model });
			return answer.ok === true;
		},

		/**
 * The thinking level in force, answered from the same snapshot and for the same reason as
 * `getSessionName`.
 */
		getThinkingLevel(): string {
			return thinkingLevel();
		},

		setThinkingLevel(level: string): void {
			send({ type: "action", action: "set_thinking_level", level });
		},

		ui: {
			async select(title: string, options: string[]): Promise<string | undefined> {
				const answer = await ask({ type: "ui_request", method: "select", title, options });
				return answer.value as string | undefined;
			},
			async confirm(title: string, message: string): Promise<boolean> {
				const answer = await ask({ type: "ui_request", method: "confirm", title, message });
				return answer.confirmed === true;
			},
			async input(title: string, placeholder?: string): Promise<string | undefined> {
				const answer = await ask({ type: "ui_request", method: "input", title, placeholder });
				return answer.value as string | undefined;
			},
			notify(message: string, notifyType?: "info" | "warning" | "error"): void {
				send({ type: "ui_request", method: "notify", message, notifyType: notifyType ?? "info" });
			},
			setStatus(statusKey: string, statusText?: string): void {
				send({ type: "ui_request", method: "setStatus", statusKey, statusText });
			},
		},
	};
}

/** Load one extension file and let it register what it offers. */
async function load(path: string): Promise<void> {
	const registration: Registration = {
		path,
		tools: new Map(),
		commands: new Map(),
		handlers: new Map(),
		flags: new Map(),
		shortcuts: new Map(),
		providers: new Map(),
		renderers: new Map(),
		markdownTransformers: [],
	};

	try {
		const module = await import(path);
		const factory = module.default;
		if (typeof factory !== "function") {
			failures.push({ path, error: "the file has no default export to call" });
			return;
		}
		
		if (Array.isArray(module.capabilities)) {
			registration.capabilities = module.capabilities.map((capability: unknown) => String(capability));
		}
		if (typeof module.deactivate === "function") {
			registration.deactivate = module.deactivate;
		}
		await factory(apiFor(registration));
		loaded.push(registration);
	} catch (error) {
		const message = error instanceof Error ? (error.stack ?? error.message) : String(error);
		failures.push({ path, error: describeLoadFailure(path, message) });
	}
}

/** Add installation guidance to missing-package errors. */
function describeLoadFailure(path: string, message: string): string {
	const missing = /Cannot find package '([^']+)' from/.exec(message)?.[1];
	if (!missing) {
		return message;
	}

	const directory = dirname(resolve(path));
	const manifestPath = join(directory, "package.json");
	if (!existsSync(manifestPath)) {
		return message;
	}

	try {
		const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
		const declared = {
			...manifest.dependencies,
			...manifest.devDependencies,
			...manifest.peerDependencies,
		};
		if (!(missing in declared)) {
			return message;
		}
	} catch {
		return message;
	}

	return `${path} declares "${missing}" in its own package.json, but it is not installed. Run \`bun install\` in ${directory} to install it.`;
}

/** What micro is told once loading has finished. */
function describe(): Json {
	return {
		type: "loaded",
		extensions: loaded.map((registration) => ({
			path: registration.path,
			// null means undeclared; [] means explicitly empty.
			capabilities: registration.capabilities ?? null,
			tools: [...registration.tools.values()].map((tool) => ({
				name: tool.name,
				label: tool.label ?? null,
				description: tool.description ?? "",
				parameters: tool.parameters ?? { type: "object", properties: {} },
				prompt_snippet: tool.promptSnippet ?? null,
				prompt_guidelines: tool.promptGuidelines ?? [],
				constrained_sampling: tool.constrainedSampling ?? null,
				render_shell: tool.renderShell ?? "default",
				execution_mode: tool.executionMode ?? null,
			})),
			commands: [...registration.commands.entries()].map(([name, command]) => ({
				name,
				description: command.description ?? "",
			})),
			flags: [...registration.flags.entries()].map(([name, flag]) => ({
				name,
				description: flag.description ?? "",
				type: flag.type,
				default: flag.default ?? null,
			})),
			shortcuts: [...registration.shortcuts.entries()].map(([key, shortcut]) => ({
				key,
				description: shortcut.description ?? "",
			})),
			events: [...registration.handlers.keys()],
			providers: [...registration.providers.entries()].map(([name, config]) => ({ name, config })),
			renderers: [...registration.renderers.keys()],
		})),
		errors: failures,
	};
}

/** Find a registered tool and its owning extension. */
function findRegistered(name: string): { registration: Registration; tool: ToolDefinition } | undefined {
	for (const registration of loaded) {
		const tool = registration.tools.get(name);
		if (tool) {
			return { registration, tool };
		}
	}
	return undefined;
}

/** The tool itself, for the callers that only draw with it. */
function findTool(name: string): ToolDefinition | undefined {
	return findRegistered(name)?.tool;
}

/** Default width before TUI layout. */
const RENDER_WIDTH = 80;

/** Render extension tool events without another host round trip. */
function autoRenderTool(event: string, payload: Json): void {
	const name = payload.toolName as string | undefined;
	const tool = name ? findTool(name) : undefined;
	if (!tool || (!tool.renderCall && !tool.renderResult)) {
		return;
	}

	const toolCallId = (payload.toolCallId as string) ?? "";
	const fields = {
		toolCallId,
		cwd: where.cwd,
		executionStarted: true,
		argsComplete: true,
		isPartial: event !== "tool_execution_end",
		expanded: false,
		showImages: true,
		isError: payload.isError === true,
	};

	const call = renderToolCall(tool, payload.args, fields);
	if (call.supported && call.componentId) {
		send({
			type: "ui_request",
			method: "tool_call_rendered",
			title: toolCallId,
			detail: call.componentId,
			options: components.render(call.componentId, RENDER_WIDTH),
		});
	}

	const result =
		event === "tool_execution_update"
			? payload.partialResult
			: event === "tool_execution_end"
				? payload.result
				: undefined;
	if (result) {
		const rendered = renderToolResult(
			tool,
			result as ToolResult,
			{ expanded: fields.expanded, isPartial: fields.isPartial },
			payload.args,
			fields,
		);
		if (rendered.supported && rendered.componentId) {
			send({
				type: "ui_request",
				method: "tool_result_rendered",
				title: toolCallId,
				detail: rendered.componentId,
				options: components.render(rendered.componentId, RENDER_WIDTH),
			});
		}
	}
}

/** Whatever a renderer returned, as the lines micro will draw. */
function asLines(drawn: unknown): string[] {
	if (typeof drawn === "string") {
		return drawn.split("\n");
	}
	if (Array.isArray(drawn)) {
		return drawn.map((line) => String(line));
	}
	if (drawn === undefined || drawn === null) {
		return [];
	}
	return String(drawn).split("\n");
}

async function runCommand(id: string, name: string, args: string): Promise<void> {
	for (const registration of loaded) {
		const command = registration.commands.get(name);
		if (!command) {
			continue;
		}
		try {
			
			const output = await command.handler(
				args,
				await contextFor(uiFor(registration.path), registration.path, true),
			);
			send({ type: "command_result", id, output: output === undefined ? null : output });
		} catch (error) {
			send({
				type: "command_result",
				id,
				error: error instanceof Error ? error.message : String(error),
			});
		}
		return;
	}
	send({ type: "command_result", id, error: `no extension registered a command called ${name}` });
}

/** Hand an event to every extension listening for it, and report what they changed. */
async function dispatchEvent(id: string | undefined, event: string, payload: Json): Promise<void> {
	const results: unknown[] = [];
	
	const sources: string[] = [];
	
	noted(event, payload);
	
	const now = await snapshot(false);
	for (const registration of loaded) {
		const handlers = registration.handlers.get(event) ?? [];
		if (handlers.length === 0) {
			continue;
		}
		const ctx = contextFrom(now, uiFor(registration.path), registration.path);
		for (const handler of handlers) {
			try {
				const result = await handler(payload, ctx);
				if (result !== undefined && result !== null) {
					results.push(result);
					sources.push(registration.path);
				}
			} catch (error) {
				send({
					type: "extension_error",
					path: registration.path,
					event,
					error: error instanceof Error ? error.message : String(error),
				});
			}
		}
	}
	if (id) {
		send({ type: "event_result", id, results, sources });
	}
}

async function handle(line: string): Promise<void> {
	let message: Json;
	try {
		message = JSON.parse(line);
	} catch (error) {
		send({ type: "host_error", error: `unreadable line: ${String(error)}` });
		return;
	}

	switch (message.type) {
		case "load": {
			located(message);
			const paths = (message.paths as string[]) ?? [];
			for (const path of paths) {
				await load(path);
			}
			send(describe());
			return;
		}
		case "tool_call": {
			const id = message.id as string;
			const name = message.name as string;
			const found = findRegistered(name);
			if (!found) {
				send({ type: "tool_result", id, error: `no extension registered a tool called ${name}` });
				return;
			}
			const owner = found.registration.path;
			await runTool(id, found.tool, (message.arguments as Json) ?? {}, await contextFor(uiFor(owner), owner));
			return;
		}
		case "abort_tool":
			abortTool(message.id as string);
			return;
		case "component": {
			
			const componentId = message.componentId as string;
			switch (message.method) {
				case "render":
					send({
						type: "component_result",
						id: message.id,
						lines: components.render(componentId, (message.width as number) ?? 80),
					});
					return;
				case "input": {
					
					if (typeof message.text === "string") {
						noteEditorText(message.text);
					}
					send({
						type: "component_result",
						id: message.id,
						...components.input(componentId, (message.data as string) ?? ""),
					});
					return;
				}
				case "invalidate":
					components.invalidate(componentId);
					return;
				case "dispose":
					components.dispose(componentId);
					return;
				default:
					send({ type: "component_result", id: message.id, error: `unknown component method: ${message.method}` });
					return;
			}
		}
		case "render_tool": {
			const id = message.id as string;
			const name = message.name as string;
			const tool = findTool(name);
			if (!tool) {
				send({ type: "render_tool_result", id, supported: false, error: `no extension registered a tool called ${name}` });
				return;
			}
			const fields = {
				toolCallId: (message.toolCallId as string) ?? "",
				cwd: (message.cwd as string) ?? "",
				executionStarted: message.executionStarted === true,
				argsComplete: message.argsComplete === true,
				isPartial: message.isPartial === true,
				expanded: message.expanded === true,
				showImages: message.showImages !== false,
				isError: message.isError === true,
			};
			const answer: ToolRenderAnswer =
				message.kind === "result"
					? renderToolResult(
							tool,
							(message.result as ToolResult) ?? { content: [] },
							{ expanded: fields.expanded, isPartial: fields.isPartial },
							message.args,
							fields,
						)
					: renderToolCall(tool, message.args, fields);
			send({ type: "render_tool_result", id, ...answer });
			return;
		}
		case "command":
			await runCommand(message.id as string, message.name as string, (message.args as string) ?? "");
			return;
		case "event": {
			
			if (message.event === "terminal_input") {
				const data = (message.payload as Json)?.data as string;
				const verdict = await dispatchTerminalInput(data);
				if (message.id) {
					send({ type: "event_result", id: message.id, results: [verdict] });
				}
				return;
			}
			
			if (message.event === "get_suggestions") {
				const payload = (message.payload as Json) ?? {};
				const suggestions = await dispatchSuggestions(
					(payload.lines as string[]) ?? [],
					(payload.cursorLine as number) ?? 0,
					(payload.cursorCol as number) ?? 0,
					(payload.force as boolean) ?? false,
				);
				if (message.id) {
					send({ type: "event_result", id: message.id, results: suggestions ? [suggestions] : [] });
				}
				return;
			}
			
			if (message.event === "apply_completion") {
				const payload = (message.payload as Json) ?? {};
				const edit = await dispatchApplyCompletion(
					(payload.lines as string[]) ?? [],
					(payload.cursorLine as number) ?? 0,
					(payload.cursorCol as number) ?? 0,
					payload.item as AutocompleteItem,
					(payload.prefix as string) ?? "",
				);
				if (message.id) {
					send({ type: "event_result", id: message.id, results: [edit] });
				}
				return;
			}
			if (
				message.event === "tool_execution_start" ||
				message.event === "tool_execution_update" ||
				message.event === "tool_execution_end"
			) {
				
				autoRenderTool(message.event, (message.payload as Json) ?? {});
			}
			if (message.event === "shortcut") {
				const key = (message.payload as Json)?.key as string;
				for (const registration of loaded) {
					const shortcut = registration.shortcuts.get(key);
					if (shortcut) {
						try {
							await shortcut.handler(
								await contextFor(uiFor(registration.path), registration.path),
							);
						} catch (error) {
							send({
								type: "extension_error",
								path: registration.path,
								event: "shortcut",
								error: error instanceof Error ? error.message : String(error),
							});
						}
					}
				}
				if (message.id) {
					send({ type: "event_result", id: message.id, results: [] });
				}
				return;
			}
			await dispatchEvent(message.id as string | undefined, message.event as string, (message.payload as Json) ?? {});
			return;
		}
		case "render": {
			const customType = message.customType as string;
			const width = (message.width as number) ?? 80;
			for (const registration of loaded) {
				const renderer = registration.renderers.get(customType);
				if (!renderer) {
					continue;
				}
				try {
					const drawn = await renderer(message.data, { width });
					send({ type: "render_result", id: message.id, lines: asLines(drawn) });
				} catch (error) {
					send({
						type: "render_result",
						id: message.id,
						error: error instanceof Error ? error.message : String(error),
					});
				}
				return;
			}
			send({ type: "render_result", id: message.id, lines: [] });
			return;
		}
		case "transform_markdown": {
			
			let markdown = message.markdown as string;
			const context = (message.context as Json) ?? {};
			try {
				for (const registration of loaded) {
					for (const transformer of registration.markdownTransformers) {
						markdown = transformer(markdown, context);
					}
				}
				send({ type: "transform_markdown_result", id: message.id, markdown });
			} catch (error) {
				send({
					type: "transform_markdown_result",
					id: message.id,
					error: error instanceof Error ? error.message : String(error),
				});
			}
			return;
		}
		case "deactivate": {
			
			const id = message.id as string;
			const path = message.path as string;
			const registration = loaded.find((held) => held.path === path);
			if (!registration) {
				send({ type: "deactivated", id, error: `${path} is not loaded` });
				return;
			}
			let failure: string | undefined;
			try {
				await registration.deactivate?.();
			} catch (error) {
				failure = error instanceof Error ? error.message : String(error);
			}
			const retired = components.disposeOwnedBy(path);
			loaded.splice(loaded.indexOf(registration), 1);
			send({ type: "deactivated", id, path, components: retired, error: failure });
			return;
		}
		case "answer":
			
			answered(message.id as string, message);
			return;
		case "set_flag":
			flagValues.set(message.name as string, message.value as boolean | string);
			return;
		case "shutdown":
			await dispatchEvent(undefined, "session_shutdown", { reason: message.reason ?? "quit" });
			process.exit(0);
			return;
		default:
			send({ type: "host_error", error: `unknown message: ${String(message.type)}` });
	}
}


let buffer = "";
const decoder = new TextDecoder();

for await (const chunk of Bun.stdin.stream()) {
	buffer += decoder.decode(chunk as Uint8Array, { stream: true });
	while (true) {
		const newline = buffer.indexOf("\n");
		if (newline === -1) {
			break;
		}
		const line = buffer.slice(0, newline).replace(/\r$/, "");
		buffer = buffer.slice(newline + 1);
		if (line.trim().length > 0) {
			
			void handle(line);
		}
	}
}
