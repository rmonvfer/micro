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

type Json = Record<string, unknown>;

interface ToolDefinition {
	name: string;
	description?: string;
	parameters?: Json;
	execute: (args: Json, ctx: unknown) => unknown | Promise<unknown>;
}

interface RegisteredCommand {
	description?: string;
	handler: (args: string, ctx: unknown) => unknown | Promise<unknown>;
}

interface Registration {
	path: string;
	tools: Map<string, ToolDefinition>;
	commands: Map<string, RegisteredCommand>;
	handlers: Map<string, Array<(event: Json, ctx: unknown) => unknown>>;
	flags: Map<string, { description?: string; type: "boolean" | "string"; default?: boolean | string }>;
	shortcuts: Map<string, { description?: string; handler: (ctx: unknown) => unknown }>;
	providers: Map<string, Json>;
}

const loaded: Registration[] = [];
const failures: Array<{ path: string; error: string }> = [];
const flagValues = new Map<string, boolean | string>();

/** Requests to micro that are waiting for an answer, by id. */
const waiting = new Map<string, (value: Json) => void>();
let nextRequestId = 0;

function send(message: Json): void {
	process.stdout.write(`${JSON.stringify(message)}\n`);
}

/** Ask micro for something and wait for the answer. */
function ask(request: Json): Promise<Json> {
	const id = `host-${nextRequestId++}`;
	return new Promise((resolve) => {
		waiting.set(id, resolve);
		send({ ...request, id });
	});
}

/** The API an extension is handed. Every entry either records something or asks micro. */
function apiFor(registration: Registration) {
	return {
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

		async setSessionName(name: string): Promise<boolean> {
			const answer = await ask({ type: "request", request: "set_session_name", name });
			return answer.ok === true;
		},

		async exec(command: string, args: string[], options?: Json): Promise<Json> {
			return ask({ type: "request", request: "exec", command, args, options: options ?? {} });
		},

		async getActiveTools(): Promise<string[]> {
			const answer = await ask({ type: "request", request: "get_active_tools" });
			return (answer.tools as string[]) ?? [];
		},

		/** Keep something in the session that the model never sees. */
		async appendEntry(customType: string, data?: unknown): Promise<boolean> {
			const answer = await ask({ type: "request", request: "append_entry", customType, data });
			return answer.ok === true;
		},

		/** Read back everything this or any other extension kept. */
		async getEntries(): Promise<Json[]> {
			const answer = await ask({ type: "request", request: "get_entries" });
			return (answer.entries as Json[]) ?? [];
		},

		/** Name an entry, or take its name away by passing nothing. */
		async setLabel(entryId: string, label?: string): Promise<boolean> {
			const answer = await ask({ type: "request", request: "set_label", entryId, label });
			return answer.ok === true;
		},

		async getAllTools(): Promise<string[]> {
			const answer = await ask({ type: "request", request: "get_all_tools" });
			return (answer.tools as string[]) ?? [];
		},

		async getCommands(): Promise<string[]> {
			const answer = await ask({ type: "request", request: "get_commands" });
			return (answer.commands as string[]) ?? [];
		},

		async getSessionName(): Promise<string | undefined> {
			const answer = await ask({ type: "request", request: "get_session_name" });
			return (answer.name as string | undefined) ?? undefined;
		},

		async getModel(): Promise<Json | undefined> {
			const answer = await ask({ type: "request", request: "get_model" });
			return answer.model as Json | undefined;
		},

		setModel(model: Json | string): void {
			send({ type: "action", action: "set_model", model });
		},

		async getThinkingLevel(): Promise<string> {
			const answer = await ask({ type: "request", request: "get_thinking_level" });
			return (answer.level as string) ?? "off";
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
	};

	try {
		const module = await import(path);
		const factory = module.default;
		if (typeof factory !== "function") {
			failures.push({ path, error: "the file has no default export to call" });
			return;
		}
		await factory(apiFor(registration));
		loaded.push(registration);
	} catch (error) {
		failures.push({ path, error: error instanceof Error ? (error.stack ?? error.message) : String(error) });
	}
}

/** What micro is told once loading has finished. */
function describe(): Json {
	return {
		type: "loaded",
		extensions: loaded.map((registration) => ({
			path: registration.path,
			tools: [...registration.tools.values()].map((tool) => ({
				name: tool.name,
				description: tool.description ?? "",
				parameters: tool.parameters ?? { type: "object", properties: {} },
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
		})),
		errors: failures,
	};
}

/** Run one of an extension's tools and report what it returned. */
async function runTool(id: string, name: string, args: Json): Promise<void> {
	for (const registration of loaded) {
		const tool = registration.tools.get(name);
		if (!tool) {
			continue;
		}
		try {
			const output = await tool.execute(args, {});
			send({ type: "tool_result", id, output: normalizeToolOutput(output) });
		} catch (error) {
			send({
				type: "tool_result",
				id,
				error: error instanceof Error ? error.message : String(error),
			});
		}
		return;
	}
	send({ type: "tool_result", id, error: `no extension registered a tool called ${name}` });
}

/** A tool may answer with a string, or with a shape carrying output and details. */
function normalizeToolOutput(output: unknown): string {
	if (typeof output === "string") {
		return output;
	}
	if (output && typeof output === "object" && "output" in output) {
		const inner = (output as { output: unknown }).output;
		return typeof inner === "string" ? inner : JSON.stringify(inner);
	}
	return JSON.stringify(output ?? null);
}

async function runCommand(id: string, name: string, args: string): Promise<void> {
	for (const registration of loaded) {
		const command = registration.commands.get(name);
		if (!command) {
			continue;
		}
		try {
			const output = await command.handler(args, {});
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
	for (const registration of loaded) {
		for (const handler of registration.handlers.get(event) ?? []) {
			try {
				const result = await handler(payload, {});
				if (result !== undefined && result !== null) {
					results.push(result);
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
		send({ type: "event_result", id, results });
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
			const paths = (message.paths as string[]) ?? [];
			for (const path of paths) {
				await load(path);
			}
			send(describe());
			return;
		}
		case "event":
			await dispatchEvent(message.id as string | undefined, message.event as string, (message.payload as Json) ?? {});
			return;
		case "tool_call":
			await runTool(message.id as string, message.name as string, (message.arguments as Json) ?? {});
			return;
		case "command":
			await runCommand(message.id as string, message.name as string, (message.args as string) ?? "");
			return;
		case "answer": {
			// micro answering something the host asked for.
			const resolve = waiting.get(message.id as string);
			if (resolve) {
				waiting.delete(message.id as string);
				resolve(message);
			}
			return;
		}
		case "set_flag":
			flagValues.set(message.name as string, message.value as boolean | string);
			return;
		case "shutdown":
			await dispatchEvent(undefined, "session_shutdown", {});
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
