// The extension host.
//
// micro runs this under Bun and talks to it over stdio, one JSON object per line. The host
// loads each extension, hands it the API it expects, and turns everything it registers
// into something micro can act on: a tool the model may call, a command a user may type, a
// handler that runs when something happens.
//
// Nothing here reaches the model or the workspace directly. An extension asks, this
// forwards the ask to micro, and micro decides — which is what keeps a third-party file
// inside the same policy as everything else.

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
	/** What the extension itself said it may do, when it exported a `capabilities` list.
	 *  Undefined for one that declared nothing, which micro treats differently from one
	 *  that declared an empty list. */
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

/**
 * Messages passed between extensions, and between an extension and itself.
 *
 * Separate from the lifecycle events micro announces: nothing here is micro's, it is a
 * place for extensions to talk. A handler that throws is reported and the rest still run,
 * so one extension cannot silence another.
 */
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

/**
 * Where this host is running, for the context handed to every handler.
 *
 * An extension is written against a workspace and against whether there is anyone to ask,
 * and both are the host's to say. Filled in when the extensions are loaded, which is
 * before anything can be called.
 */
const loaded: Registration[] = [];
const failures: Array<{ path: string; error: string }> = [];
const flagValues = new Map<string, boolean | string>();

/** The API an extension is handed. Every entry either records something or asks micro.
 *
 * The wire pair taken here carries this registration's own path on everything it sends, so
 * micro can hold an ask to what this extension declared. It shadows the module's own `send`
 * for the whole function, which is why nothing below has to name the extension itself. */
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

		/** Draw a custom entry of this type yourself. Entries are not sent to the model. */
		registerEntryRenderer(
			customType: string,
			renderer: (data: unknown, options: { width: number }) => unknown,
		): void {
			registration.renderers.set(customType, renderer);
		},

		/** Rewrite user and assistant markdown before it is drawn. Registered here so an
		 * extension written for pi loads without editing, and answered when asked for —
		 * nothing on this side of the wire draws the interactive transcript itself, so
		 * there is nowhere here to apply one at the moment text is about to be shown. */
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

		// Actions. Each one is micro's to carry out.
		sendUserMessage(content: string, options?: Json): void {
			send({ type: "action", action: "send_user_message", content, options: options ?? {} });
		},

		sendMessage(message: Json, options?: Json): void {
			send({ type: "action", action: "send_message", message, options: options ?? {} });
		},

		/** Name the session. Fire-and-forget, as pi's own is. */
		setSessionName(name: string): void {
			send({ type: "action", action: "set_session_name", name });
		},

		async exec(command: string, args: string[], options?: Json): Promise<Json> {
			return ask({ type: "request", request: "exec", command, args, options: options ?? {} });
		},

		/** The tools the model is being offered. Answered from the snapshot every event and
		 * command already takes, because pi's own is a plain array rather than a promise —
		 * an extension calling it without `await` gets what it expects. */
		getActiveTools(): string[] {
			return activeTools();
		},

		/** Choose which tools are offered from here on. Fire-and-forget, the same as
		 * setModel and setThinkingLevel: the interface picks it up on its own time rather
		 * than on this call's stack, so there is nothing here to await. */
		setActiveTools(toolNames: string[]): void {
			noteActiveTools(toolNames);
			send({ type: "action", action: "set_active_tools", toolNames });
		},

		/** Keep something in the session that the model never sees. Fire-and-forget: the
		 * session is the interface's to write, and pi's own answers nothing either. */
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

		/** Every tool that exists, described rather than merely named — parameters,
		 * guidelines and where it came from. Answered from the snapshot, since pi's own is
		 * a plain array an extension reads without awaiting. */
		getAllTools(): Json[] {
			return allTools();
		},

		/** Every command that can be typed, from micro's own and from every extension. */
		getCommands(): Json[] {
			return commands();
		},

		/** What the session is called, if anything. Answered from the snapshot every event
		 * and command already takes, because pi's own is a plain value rather than a
		 * promise — an extension calling it without `await` gets a string it can read. */
		getSessionName(): string | undefined {
			return sessionName();
		},

		async getModel(): Promise<Json | undefined> {
			const answer = await ask({ type: "request", request: "get_model" });
			return answer.model as Json | undefined;
		},

		/** Change the model, answering whether it could be. False when there is no
		 * credential for the service the chosen model is served by. */
		async setModel(model: Json | string): Promise<boolean> {
			const answer = await ask({ type: "request", request: "set_model", model });
			return answer.ok === true;
		},

		/** The thinking level in force, answered from the same snapshot and for the same
		 * reason as `getSessionName`. */
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
		// Read before the factory runs, because both are declarations about the file rather
		// than things it does: what it may ask micro for, and what to call when it is let
		// go. An extension that exports neither is one written before either existed.
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

/**
 * A bare `Cannot find package 'X'` says an import did not resolve, but not why, or what to
 * do about it — indistinguishable from a resolution bug in this layer itself. When the
 * extension's own `package.json`, sitting beside it, already names `X` as a dependency, the
 * reason is ordinary and the fix is one command: point at both. Anything else — an
 * unexpected specifier, no adjacent manifest, or a failure that is not this kind of
 * resolution error at all — is left exactly as reported, since guessing at a cause this does
 * not recognize would risk saying something that is not true.
 */
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
			// Null rather than an empty list for an extension that declared nothing: micro
			// tells "this extension says it needs nothing" apart from "this extension has
			// never heard of capabilities", and only the second is asked about.
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

/** Find a tool by name across every extension that loaded, in load order, along with the
 *  extension that registered it — what a tool call is attributed to when it asks micro for
 *  something while it runs. */
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

/** How wide a tool's own renderCall/renderResult are told the screen is when nothing asked
 * for a specific width — drawing itself for the first time, or pushing a change of its own
 * accord. micro wraps or clips whatever comes back to the room it actually has, the same as
 * it already does for a widget's plain lines. */
const RENDER_WIDTH = 80;

/**
 * Ask a tool's renderCall/renderResult to draw themselves for a lifecycle moment micro
 * already reported, and tell micro what they drew. Not conditional on any extension having
 * registered a `micro.on()` handler for the same event — a tool's own renderer runs whether
 * or not anything else is listening, the same way execute() does.
 *
 * Rust already knows exactly when to ask for this: args arriving, a partial result, the
 * final one — so this is driven entirely from here, on the same events every extension is
 * already told about, rather than from a request Rust would otherwise have to send once per
 * state change. The one thing that does need to reach micro on its own schedule — a
 * renderer's own `ctx.invalidate()` — sends `component_changed` directly instead (see
 * `renderToolCall`/`renderToolResult` in tools.ts), independently of this function.
 */
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
			// Only a command handler gets `newSession`, `fork` and the rest of what moves
			// the conversation somewhere else — the same restriction pi places on
			// `ExtensionCommandContext` versus the plain `ExtensionContext` a tool or an
			// event handler is given.
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
	// Which extension gave each answer, in the same order: an answer is how an extension
	// changes what micro does, and micro decides whether it may by looking at who gave it.
	const sources: string[] = [];
	// `isIdle`/`signal`/`waitForIdle`, and a `newSession`/`fork`/`switchSession` waiting
	// on this session's own `session_start`, are answered from this, not from a round
	// trip, so it has to run before anything asks for a context built off it — including
	// this event's own handlers, if it is `agent_start` or `agent_settled` itself.
	noted(event, payload);
	// One snapshot for every handler this event reaches, not one per handler: they are
	// watching the same moment, and asking micro for it again for each one would answer
	// a question already answered. The context built from it is still per extension, since
	// what an extension asks through it has to name the extension that asked.
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
			// render/input answer what they were asked; invalidate/dispose carry no id at
			// all — nobody is waiting on either, the same way `set_flag` answers nobody.
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
					// Only `setEditorComponent`'s replacement is asked about a key that
					// carries the built-in editor's text alongside it — see `noteEditorText`
					// for why this is the one place that reads it rather than `components.ts`.
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
			// Not dispatched like an ordinary event: a listener here was registered
			// through `ctx.ui.onTerminalInput`, not through `micro.on`, so it lives in
			// `host-ui.ts`'s own set rather than in any extension's `registration`.
			if (message.event === "terminal_input") {
				const data = (message.payload as Json)?.data as string;
				const verdict = await dispatchTerminalInput(data);
				if (message.id) {
					send({ type: "event_result", id: message.id, results: [verdict] });
				}
				return;
			}
			// Same story as `terminal_input`: a listener registered through
			// `ctx.ui.addAutocompleteProvider` lives in `host-ui.ts`'s own chain, not in
			// any extension's `registration`.
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
			// What committing an extension's own completion item writes, through whichever
			// `applyCompletion` the provider chain settled on.
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
				// Ahead of the ordinary dispatch below, not instead of it: a tool's own
				// renderer is not a `micro.on()` handler, and running it here does not use
				// up the one chance an extension listening for the same event still has.
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
			// Applied in registration order, each transformer's output feeding the next —
			// the same fold pi runs its own transformer list through before drawing.
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
			// The extension's own `deactivate` runs first, so it can put back whatever it
			// changed out here — a file it watched, a process it started — and then its
			// registrations go, so nothing it added can be reached again. A `deactivate`
			// that throws is reported and the dropping still happens: an extension refusing
			// to leave is not a reason to keep offering what it registered.
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
			// micro answering something the host asked for.
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

// Strict JSON-lines framing, split on \n and nothing else: U+2028 and U+2029 are legal
// inside a JSON string, and a reader that treats them as breaks cuts records in half.
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
			// Not awaited: a command may ask micro something and wait for the answer, and
			// the answer arrives on this same stream. Waiting here would mean nothing
			// could ever be read while anything was waiting.
			void handle(line);
		}
	}
}
