
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute as isAbsolutePath, join, resolve as resolvePath } from "node:path";
import { randomUUID } from "node:crypto";

import { Editor, KeybindingsManager, TUI_KEYBINDINGS } from "@earendil-works/pi-tui";



function formatKeyPart(part: string, capitalize: boolean): string {
	const displayPart = process.platform === "darwin" && part.toLowerCase() === "alt" ? "option" : part;
	return capitalize ? displayPart.charAt(0).toUpperCase() + displayPart.slice(1) : displayPart;
}

/**
 * Ported unchanged from keybinding-hints.ts's `formatKeyText`: cosmetic-only, so it needs nothing
 * beyond the string it is given.
 */
function formatKeyText(key: string, capitalize = false): string {
	return key
		.split("/")
		.map((k) =>
			k
				.split("+")
				.map((part) => formatKeyPart(part, capitalize))
				.join("+"),
		)
		.join("/");
}


interface KeybindingsManagerLike {
	getKeys(keybinding: string): string[];
}

let cachedKeybindings: KeybindingsManagerLike | undefined;

async function keybindings(): Promise<KeybindingsManagerLike> {
	if (!cachedKeybindings) {
		const piTui = (await import("@earendil-works/pi-tui")) as { getKeybindings(): KeybindingsManagerLike };
		cachedKeybindings = piTui.getKeybindings();
	}
	return cachedKeybindings;
}

async function formatKeysFor(keybinding: string, capitalize: boolean): Promise<string> {
	const keys = (await keybindings()).getKeys(keybinding);
	if (keys.length === 0) return "";
	return formatKeyText(keys.join("/"), capitalize);
}

export async function keyText(keybinding: string): Promise<string> {
	return formatKeysFor(keybinding, false);
}

export async function keyDisplayText(keybinding: string): Promise<string> {
	return formatKeysFor(keybinding, true);
}

export async function keyHint(keybinding: string, description: string): Promise<string> {
	const text = await formatKeysFor(keybinding, false);
	return `${text} ${description}`;
}

export function rawKeyHint(key: string, description: string): string {
	return `${formatKeyText(key)} ${description}`;
}



/**
 * Matches `crates/micro-extensions/host/components.ts`'s `Component` shape structurally, without
 * importing it.
 */
interface RenderableComponent {
	render(width: number): string[];
	handleInput?(data: string): { consume?: boolean } | void;
	invalidate?(): void;
	dispose?(): void;
}

export class DynamicBorder implements RenderableComponent {
	private color: (str: string) => string;

	constructor(color: (str: string) => string = (str) => str) {
		this.color = color;
	}

	invalidate(): void {}

	render(width: number): string[] {
		return [this.color("─".repeat(Math.max(1, width)))];
	}
}



const APP_KEYBINDINGS = {
	"app.interrupt": { defaultKeys: "escape" },
	"app.clear": { defaultKeys: "ctrl+c" },
	"app.exit": { defaultKeys: "ctrl+d" },
	"app.suspend": { defaultKeys: process.platform === "win32" ? [] : "ctrl+z" },
	"app.thinking.cycle": { defaultKeys: "shift+tab" },
	"app.model.cycleForward": { defaultKeys: "ctrl+p" },
	"app.model.cycleBackward": { defaultKeys: "shift+ctrl+p" },
	"app.model.select": { defaultKeys: "ctrl+l" },
	"app.tools.expand": { defaultKeys: "ctrl+o" },
	"app.thinking.toggle": { defaultKeys: "ctrl+t" },
	"app.session.toggleNamedFilter": { defaultKeys: "ctrl+n" },
	"app.editor.external": { defaultKeys: "ctrl+g" },
	"app.message.copy": { defaultKeys: "ctrl+x" },
	"app.message.followUp": { defaultKeys: "alt+enter" },
	"app.message.dequeue": { defaultKeys: "alt+up" },
	"app.clipboard.pasteImage": { defaultKeys: process.platform === "win32" ? "alt+v" : "ctrl+v" },
	"app.session.new": { defaultKeys: [] },
	"app.session.tree": { defaultKeys: [] },
	"app.session.fork": { defaultKeys: [] },
	"app.session.resume": { defaultKeys: [] },
	"app.tree.foldOrUp": {
		defaultKeys: process.platform === "darwin" ? ["alt+left", "ctrl+left"] : ["ctrl+left", "alt+left"],
	},
	"app.tree.unfoldOrDown": {
		defaultKeys: process.platform === "darwin" ? ["alt+right", "ctrl+right"] : ["ctrl+right", "alt+right"],
	},
	"app.tree.editLabel": { defaultKeys: "shift+l" },
	"app.tree.toggleLabelTimestamp": { defaultKeys: "shift+t" },
	"app.session.togglePath": { defaultKeys: "ctrl+p" },
	"app.session.toggleSort": { defaultKeys: "ctrl+s" },
	"app.session.rename": { defaultKeys: "ctrl+r" },
	"app.session.delete": { defaultKeys: "ctrl+d" },
	"app.session.deleteNoninvasive": { defaultKeys: "ctrl+backspace" },
	"app.models.save": { defaultKeys: "ctrl+s" },
	"app.models.enableAll": { defaultKeys: "ctrl+a" },
	"app.models.clearAll": { defaultKeys: "ctrl+x" },
	"app.models.toggleProvider": { defaultKeys: "ctrl+p" },
	"app.models.reorderUp": { defaultKeys: "alt+up" },
	"app.models.reorderDown": { defaultKeys: "alt+down" },
	"app.tree.filter.default": { defaultKeys: "ctrl+d" },
	"app.tree.filter.noTools": { defaultKeys: "ctrl+t" },
	"app.tree.filter.userOnly": { defaultKeys: "ctrl+u" },
	"app.tree.filter.labeledOnly": { defaultKeys: "ctrl+l" },
	"app.tree.filter.all": { defaultKeys: "ctrl+a" },
	"app.tree.filter.cycleForward": { defaultKeys: "ctrl+o" },
	"app.tree.filter.cycleBackward": { defaultKeys: "shift+ctrl+o" },
} as const;

interface KeybindingsLike {
	matches(data: string, keybinding: string): boolean;
}

function looksLikeKeybindings(value: unknown): value is KeybindingsLike {
	return !!value && typeof (value as KeybindingsLike).matches === "function";
}

let fallbackKeybindings: KeybindingsLike | undefined;

/**
 * pi-tui's real `KeybindingsManager`, seeded with pi's real app-level defaults merged over pi-
 * tui's own real `tui.*` table.
 */
function defaultAppKeybindings(): KeybindingsLike {
	if (!fallbackKeybindings) {
		fallbackKeybindings = new KeybindingsManager({ ...TUI_KEYBINDINGS, ...APP_KEYBINDINGS } as never);
	}
	return fallbackKeybindings;
}

export class CustomEditor extends Editor {
	private keybindings: KeybindingsLike;
	actionHandlers: Map<string, () => void> = new Map();
	onEscape: (() => void) | undefined;
	onCtrlD: (() => void) | undefined;
	onPasteImage: (() => void) | undefined;
	onExtensionShortcut: ((data: string) => boolean) | undefined;

	constructor(tui: unknown, theme: unknown, keybindings?: unknown, options?: unknown) {
		// biome-ignore lint: Editor's real constructor signature, ported as-is.
		super(tui as never, theme as never, options as never);
		
		this.keybindings = looksLikeKeybindings(keybindings) ? keybindings : defaultAppKeybindings();
	}

	onAction(action: string, handler: () => void): void {
		this.actionHandlers.set(action, handler);
	}

	handleInput(data: string): void {
		if (this.onExtensionShortcut?.(data)) {
			return;
		}

		if (this.keybindings.matches(data, "app.clipboard.pasteImage")) {
			this.onPasteImage?.();
			return;
		}

		if (this.keybindings.matches(data, "app.interrupt")) {
			if (!this.isShowingAutocomplete()) {
				const handler = this.onEscape ?? this.actionHandlers.get("app.interrupt");
				if (handler) {
					handler();
					return;
				}
			}
			super.handleInput(data);
			return;
		}

		if (this.keybindings.matches(data, "app.exit")) {
			if (this.getText().length === 0) {
				const handler = this.onCtrlD ?? this.actionHandlers.get("app.exit");
				if (handler) handler();
				return;
			}
		}

		if (
			this.keybindings.matches(data, "tui.editor.historyPrevious") ||
			this.keybindings.matches(data, "tui.editor.historyNext")
		) {
			super.handleInput(data);
			return;
		}

		for (const [action, handler] of this.actionHandlers) {
			if (action !== "app.interrupt" && action !== "app.exit" && this.keybindings.matches(data, action)) {
				handler();
				return;
			}
		}

		super.handleInput(data);
	}
}



const LOADER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LOADER_INTERVAL_MS = 80;

export class BorderedLoader implements RenderableComponent {
	private border: DynamicBorder;
	private startedAt = Date.now();
	private message: string;
	private cancellable: boolean;
	private spinnerColor: (text: string) => string;
	private messageColor: (text: string) => string;
	private aborted = false;
	onAbort: (() => void) | undefined;

	constructor(
		_tui: unknown,
		theme: { fg(color: string, text: string): string },
		message: string,
		options?: { cancellable?: boolean },
	) {
		this.border = new DynamicBorder((text) => theme.fg("border", text));
		this.message = message;
		this.cancellable = options?.cancellable ?? true;
		this.spinnerColor = (text) => theme.fg("accent", text);
		this.messageColor = (text) => theme.fg("muted", text);
	}

	get signal(): AbortSignal {
		const controller = new AbortController();
		if (this.aborted) controller.abort();
		return controller.signal;
	}

	render(width: number): string[] {
		const elapsed = Date.now() - this.startedAt;
		const frame = LOADER_FRAMES[Math.floor(elapsed / LOADER_INTERVAL_MS) % LOADER_FRAMES.length];
		const spinnerLine = `${this.spinnerColor(frame)} ${this.messageColor(this.message)}`;
		const lines = [...this.border.render(width), "", spinnerLine];
		if (this.cancellable) {
			lines.push("", `${rawKeyHint("esc", "cancel")}`);
		}
		lines.push("", ...this.border.render(width));
		return lines;
	}

	handleInput(data: string): { consume?: boolean } | void {
		if (!this.cancellable) return;
		
		if (data === "\x1b") {
			this.aborted = true;
			this.onAbort?.();
			return { consume: true };
		}
	}

	invalidate(): void {}
}



const LANGUAGE_BY_EXTENSION: Record<string, string> = {
	ts: "typescript",
	tsx: "typescript",
	js: "javascript",
	jsx: "javascript",
	mjs: "javascript",
	cjs: "javascript",
	py: "python",
	rs: "rust",
	go: "go",
	rb: "ruby",
	java: "java",
	kt: "kotlin",
	c: "c",
	h: "c",
	cpp: "cpp",
	cc: "cpp",
	hpp: "cpp",
	cs: "csharp",
	php: "php",
	sh: "bash",
	bash: "bash",
	zsh: "bash",
	json: "json",
	yaml: "yaml",
	yml: "yaml",
	toml: "toml",
	md: "markdown",
	html: "html",
	css: "css",
	scss: "scss",
	sql: "sql",
	swift: "swift",
};

export function getLanguageFromPath(path: string): string | undefined {
	const match = /\.([a-zA-Z0-9]+)$/.exec(path);
	if (!match) return undefined;
	return LANGUAGE_BY_EXTENSION[match[1].toLowerCase()];
}

export function highlightCode(code: string, _language?: string): string {
	return code;
}

function ansiTag(code: number): (text: string) => string {
	return (text: string) => `\x1b[${code}m${text}\x1b[0m`;
}

export function getMarkdownTheme(): Record<string, (text: string) => string> {
	return {
		heading: ansiTag(33),
		link: ansiTag(34),
		linkUrl: ansiTag(90),
		code: ansiTag(36),
		codeBlock: ansiTag(32),
		codeBlockBorder: ansiTag(90),
		quote: ansiTag(90),
		quoteBorder: ansiTag(90),
		hr: ansiTag(90),
		listBullet: ansiTag(36),
		bold: (text: string) => `\x1b[1m${text}\x1b[0m`,
		italic: (text: string) => `\x1b[3m${text}\x1b[0m`,
		strikethrough: (text: string) => `\x1b[9m${text}\x1b[0m`,
		underline: (text: string) => `\x1b[4m${text}\x1b[0m`,
	};
}

export function getSelectListTheme(): Record<string, (text: string) => string> {
	return {
		selectedPrefix: ansiTag(34),
		selectedText: ansiTag(34),
		description: ansiTag(90),
		scrollInfo: ansiTag(90),
		noMatch: ansiTag(90),
	};
}

export function getSettingsListTheme(): {
	label: (text: string, selected: boolean) => string;
	value: (text: string, selected: boolean) => string;
	description: (text: string) => string;
	cursor: string;
	hint: (text: string) => string;
} {
	return {
		label: (text, selected) => (selected ? ansiTag(34)(text) : text),
		value: (text, selected) => (selected ? ansiTag(34)(text) : ansiTag(90)(text)),
		description: ansiTag(90),
		cursor: ansiTag(34)("→ "),
		hint: ansiTag(90),
	};
}



export function getAgentDir(): string {
	const configured = process.env.MICRO_DIR?.trim();
	if (configured) {
		return configured;
	}
	const home = process.env.HOME?.trim() || process.env.USERPROFILE?.trim() || "";
	const legacy = join(home, ".micro");
	if (existsSync(legacy)) {
		return legacy;
	}
	const base = process.env.XDG_DATA_HOME?.trim();
	return join(base && isAbsolutePath(base) ? base : join(home, ".local", "share"), "micro");
}



function parseFrontmatterYaml(yamlText: string): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	for (const rawLine of yamlText.split("\n")) {
		const line = rawLine.trimEnd();
		if (!line.trim() || line.trimStart().startsWith("#")) continue;
		const separator = line.indexOf(":");
		if (separator === -1) continue;
		const key = line.slice(0, separator).trim();
		let value: unknown = line.slice(separator + 1).trim();
		if (value === "") continue;
		if (typeof value === "string") {
			if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
				value = value.slice(1, -1);
			} else if (value === "true") {
				value = true;
			} else if (value === "false") {
				value = false;
			} else if (/^-?\d+(\.\d+)?$/.test(value)) {
				value = Number(value);
			} else if (value.startsWith("[") && value.endsWith("]")) {
				value = value
					.slice(1, -1)
					.split(",")
					.map((item) => item.trim().replace(/^["']|["']$/g, ""))
					.filter((item) => item.length > 0);
			}
		}
		result[key] = value;
	}
	return result;
}

export function parseFrontmatter(content: string): { frontmatter: Record<string, unknown>; body: string } {
	const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
	if (!normalized.startsWith("---")) {
		return { frontmatter: {}, body: normalized };
	}
	const endIndex = normalized.indexOf("\n---", 3);
	if (endIndex === -1) {
		return { frontmatter: {}, body: normalized };
	}
	const yamlText = normalized.slice(4, endIndex);
	const body = normalized.slice(endIndex + 4).trim();
	return { frontmatter: parseFrontmatterYaml(yamlText), body };
}

export function stripFrontmatter(content: string): string {
	return parseFrontmatter(content).body;
}

// ============================================================================
// Conversation serialization for summarization.


const SERIALIZE_TOOL_RESULT_MAX_CHARS = 2000;

function messageContentText(content: unknown, fallback = ""): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return fallback;
	return content
		.filter((block): block is { type: string; text?: string } => !!block && typeof block === "object")
		.filter((block) => block.type === "text")
		.map((block) => block.text ?? "")
		.join("");
}

function truncateForSummary(text: string, maxChars: number): string {
	if (text.length <= maxChars) return text;
	const truncatedChars = text.length - maxChars;
	return `${text.slice(0, maxChars)}\n\n[... ${truncatedChars} more characters truncated]`;
}

export function serializeConversation(messages: Record<string, unknown>[]): string {
	const parts: string[] = [];
	for (const msg of messages) {
		if (msg.role === "user") {
			const content = messageContentText(msg.content, "");
			if (content) parts.push(`[User]: ${content}`);
		} else if (msg.role === "assistant") {
			const content = Array.isArray(msg.content) ? msg.content : [];
			const thinkingParts = content
				.filter((block: Record<string, unknown>) => block?.type === "thinking")
				.map((block: Record<string, unknown>) => String(block.thinking ?? ""));
			const toolCalls = content
				.filter((block: Record<string, unknown>) => block?.type === "toolCall")
				.map((block: Record<string, unknown>) => {
					const args = (block.arguments as Record<string, unknown>) ?? {};
					const argsStr = Object.entries(args)
						.map(([k, v]) => `${k}=${JSON.stringify(v)}`)
						.join(", ");
					return `${block.name}(${argsStr})`;
				});
			if (thinkingParts.length > 0) parts.push(`[Assistant thinking]: ${thinkingParts.join("\n")}`);
			if (content.some((block: Record<string, unknown>) => block?.type === "text")) {
				parts.push(`[Assistant]: ${messageContentText(msg.content)}`);
			}
			if (toolCalls.length > 0) parts.push(`[Assistant tool calls]: ${toolCalls.join("; ")}`);
		} else if (msg.role === "toolResult") {
			const content = messageContentText(msg.content, "");
			if (content) parts.push(`[Tool result]: ${truncateForSummary(content, SERIALIZE_TOOL_RESULT_MAX_CHARS)}`);
		}
	}
	return parts.join("\n\n");
}



interface SimpleTool {
	name: string;
	description: string;
	parameters: unknown;
	execute: (args: Record<string, unknown>) => Promise<string>;
}

interface ToolsOptions {
	read?: unknown;
	grep?: unknown;
	find?: unknown;
	ls?: unknown;
}

function resolveUnder(cwd: string, path: string): string {
	return resolvePath(cwd, path);
}

function simpleReadTool(cwd: string): SimpleTool {
	return {
		name: "read",
		description: "Read a file's contents.",
		parameters: {
			type: "object",
			properties: { path: { type: "string" } },
			required: ["path"],
		},
		async execute(args) {
			const path = resolveUnder(cwd, String(args.path ?? ""));
			return readFileSync(path, "utf-8");
		},
	};
}

function simpleGrepTool(cwd: string): SimpleTool {
	return {
		name: "grep",
		description: "Search for a pattern across files under a directory.",
		parameters: {
			type: "object",
			properties: { pattern: { type: "string" }, path: { type: "string" } },
			required: ["pattern"],
		},
		async execute(args) {
			const pattern = new RegExp(String(args.pattern ?? ""));
			const root = resolveUnder(cwd, String(args.path ?? "."));
			const matches: string[] = [];
			for (const file of walk(root)) {
				const lines = readFileSync(file, "utf-8").split("\n");
				lines.forEach((line, index) => {
					if (pattern.test(line)) {
						matches.push(`${file}:${index + 1}:${line}`);
					}
				});
			}
			return matches.join("\n");
		},
	};
}

function simpleFindTool(cwd: string): SimpleTool {
	return {
		name: "find",
		description: "Find files under a directory whose name matches a pattern.",
		parameters: {
			type: "object",
			properties: { pattern: { type: "string" }, path: { type: "string" } },
			required: ["pattern"],
		},
		async execute(args) {
			const pattern = new RegExp(String(args.pattern ?? ""));
			const root = resolveUnder(cwd, String(args.path ?? "."));
			return [...walk(root)].filter((file) => pattern.test(file)).join("\n");
		},
	};
}

function simpleLsTool(cwd: string): SimpleTool {
	return {
		name: "ls",
		description: "List a directory's entries.",
		parameters: {
			type: "object",
			properties: { path: { type: "string" } },
			required: [],
		},
		async execute(args) {
			const root = resolveUnder(cwd, String(args.path ?? "."));
			return readdirSync(root)
				.map((entry) => (statSync(join(root, entry)).isDirectory() ? `${entry}/` : entry))
				.join("\n");
		},
	};
}

function* walk(root: string): Generator<string> {
	let entries: string[];
	try {
		entries = readdirSync(root);
	} catch {
		return;
	}
	for (const entry of entries) {
		if (entry === "node_modules" || entry === ".git") continue;
		const full = join(root, entry);
		const info = statSync(full);
		if (info.isDirectory()) {
			yield* walk(full);
		} else {
			yield full;
		}
	}
}

export function createReadOnlyTools(cwd: string, _options?: ToolsOptions): SimpleTool[] {
	return [simpleReadTool(cwd), simpleGrepTool(cwd), simpleFindTool(cwd), simpleLsTool(cwd)];
}



interface MicroWire {
	ask(request: Record<string, unknown>): Promise<Record<string, unknown>>;
}

function wire(): MicroWire {
	const published = (globalThis as { __MICRO_WIRE__?: MicroWire }).__MICRO_WIRE__;
	if (!published) {
		throw new Error(
			"micro's wire (globalThis.__MICRO_WIRE__) is not published yet — this tool was called before host/wire.ts ran, which should not happen for an extension loaded normally.",
		);
	}
	return published;
}

async function runBuiltinTool(tool: string, cwd: string, arguments_: Record<string, unknown>): Promise<string> {
	const answer = await wire().ask({ type: "request", request: "run_builtin_tool", tool, root: cwd, arguments: arguments_ });
	if (typeof answer.error === "string") {
		throw new Error(answer.error);
	}
	return String(answer.result ?? "");
}

/** Either spelling of a field name, preferring whichever is actually present. */
function either(args: Record<string, unknown>, snakeCase: string, camelCase: string): unknown {
	return args[snakeCase] ?? args[camelCase];
}

export function createReadTool(cwd: string): SimpleTool {
	return {
		name: "read",
		description: "Read a file from the workspace. Returns the contents with 1-indexed line numbers.",
		parameters: {
			type: "object",
			properties: {
				path: { type: "string" },
				offset: { type: "integer" },
				limit: { type: "integer" },
			},
			required: ["path"],
		},
		execute: (args) => runBuiltinTool("read", cwd, args),
	};
}

export function createWriteTool(cwd: string): SimpleTool {
	return {
		name: "write",
		description: "Write a file, creating parent directories and overwriting any existing contents.",
		parameters: {
			type: "object",
			properties: { path: { type: "string" }, content: { type: "string" } },
			required: ["path", "content"],
		},
		execute: (args) => runBuiltinTool("write", cwd, args),
	};
}

export function createEditTool(cwd: string): SimpleTool {
	return {
		name: "edit",
		description: "Replace an exact string in a file. The old string must appear exactly once.",
		parameters: {
			type: "object",
			properties: {
				path: { type: "string" },
				old_string: { type: "string" },
				new_string: { type: "string" },
			},
			required: ["path", "old_string", "new_string"],
		},
		execute: (args) =>
			runBuiltinTool("edit", cwd, {
				path: args.path,
				old_string: either(args, "old_string", "oldString"),
				new_string: either(args, "new_string", "newString"),
			}),
	};
}

export function createGrepTool(cwd: string): SimpleTool {
	return {
		name: "grep",
		description: "Search file contents with a regular expression. Respects .gitignore and skips binary files.",
		parameters: {
			type: "object",
			properties: {
				pattern: { type: "string" },
				path: { type: "string" },
				glob: { type: "string" },
				case_insensitive: { type: "boolean" },
				literal: { type: "boolean" },
			},
			required: ["pattern"],
		},
		execute: (args) =>
			runBuiltinTool("grep", cwd, {
				pattern: args.pattern,
				path: args.path,
				glob: args.glob,
				case_insensitive: either(args, "case_insensitive", "caseInsensitive"),
				literal: args.literal,
			}),
	};
}

export function createFindTool(cwd: string): SimpleTool {
	return {
		name: "find",
		description: "Find files by glob pattern, most recently modified first. Respects .gitignore.",
		parameters: {
			type: "object",
			properties: { pattern: { type: "string" }, path: { type: "string" }, limit: { type: "integer" } },
			required: ["pattern"],
		},
		execute: (args) => runBuiltinTool("find", cwd, args),
	};
}

export function createLsTool(cwd: string): SimpleTool {
	return {
		name: "ls",
		description: "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories.",
		parameters: {
			type: "object",
			properties: { path: { type: "string" }, limit: { type: "integer" } },
		},
		execute: (args) => runBuiltinTool("ls", cwd, args),
	};
}

export function createBashTool(cwd: string): SimpleTool {
	return {
		name: "bash",
		description: "Run a shell command in the workspace root. Returns combined stdout and stderr along with the exit code.",
		parameters: {
			type: "object",
			properties: { command: { type: "string" }, timeout: { type: "number" } },
			required: ["command"],
		},
		execute: (args) => runBuiltinTool("bash", cwd, args),
	};
}

export function createCodingTools(cwd: string, _options?: ToolsOptions): SimpleTool[] {
	
	return [createReadTool(cwd), createBashTool(cwd), createEditTool(cwd), createWriteTool(cwd)];
}



export const COMPACTION_SUMMARY_PREFIX = `The conversation history before this point was compacted into the following summary:

<summary>
`;
export const COMPACTION_SUMMARY_SUFFIX = `
</summary>`;
export const BRANCH_SUMMARY_PREFIX = `The following is a summary of a branch that this conversation came back from:

<summary>
`;
export const BRANCH_SUMMARY_SUFFIX = `</summary>`;

interface AgentMessageLike {
	role: string;
	[key: string]: unknown;
}

export function bashExecutionToText(msg: {
	command: string;
	output: string;
	cancelled: boolean;
	exitCode: number | null | undefined;
	truncated: boolean;
	fullOutputPath?: string;
}): string {
	let text = `Ran \`${msg.command}\`\n`;
	text += msg.output ? `\`\`\`\n${msg.output}\n\`\`\`` : "(no output)";
	if (msg.cancelled) {
		text += "\n\n(command cancelled)";
	} else if (msg.exitCode !== null && msg.exitCode !== undefined && msg.exitCode !== 0) {
		text += `\n\nCommand exited with code ${msg.exitCode}`;
	}
	if (msg.truncated && msg.fullOutputPath) {
		text += `\n\n[Output truncated. Full output: ${msg.fullOutputPath}]`;
	}
	return text;
}

export function createBranchSummaryMessage(summary: string, fromId: string, timestamp: string) {
	return { role: "branchSummary", summary, fromId, timestamp: new Date(timestamp).getTime() };
}

export function createCompactionSummaryMessage(summary: string, tokensBefore: number, timestamp: string) {
	return { role: "compactionSummary", summary, tokensBefore, timestamp: new Date(timestamp).getTime() };
}

export function createCustomMessage(
	customType: string,
	content: unknown,
	display: boolean,
	details: unknown,
	timestamp: string,
) {
	return { role: "custom", customType, content, display, details, timestamp: new Date(timestamp).getTime() };
}

export function convertToLlm(messages: AgentMessageLike[]): unknown[] {
	return messages
		.map((m) => {
			switch (m.role) {
				case "bashExecution":
					if (m.excludeFromContext) return undefined;
					return { role: "user", content: [{ type: "text", text: bashExecutionToText(m as never) }], timestamp: m.timestamp };
				case "custom": {
					const content = typeof m.content === "string" ? [{ type: "text", text: m.content }] : m.content;
					return { role: "user", content, timestamp: m.timestamp };
				}
				case "branchSummary":
					return {
						role: "user",
						content: [{ type: "text", text: BRANCH_SUMMARY_PREFIX + m.summary + BRANCH_SUMMARY_SUFFIX }],
						timestamp: m.timestamp,
					};
				case "compactionSummary":
					return {
						role: "user",
						content: [{ type: "text", text: COMPACTION_SUMMARY_PREFIX + m.summary + COMPACTION_SUMMARY_SUFFIX }],
						timestamp: m.timestamp,
					};
				case "user":
				case "assistant":
				case "toolResult":
					return m;
				default:
					return undefined;
			}
		})
		.filter((m) => m !== undefined);
}



interface NormalizedMessageEntry {
	type: "message";
	id: string;
	parentId: string | null;
	timestamp: string;
	message: Record<string, unknown>;
}
interface NormalizedCustomEntry {
	type: "custom";
	id: string;
	parentId: string | null;
	timestamp: string;
	customType: string;
	data: unknown;
}
interface NormalizedCompactionEntry {
	type: "compaction";
	id: string;
	parentId: string | null;
	timestamp: string;
	summary: string;
	firstKeptEntryId: string | null;
	tokensBefore: number;
}
type NormalizedEntry = NormalizedMessageEntry | NormalizedCustomEntry | NormalizedCompactionEntry;

interface NormalizedHeader {
	type: "session";
	version: number;
	id: string;
	timestamp: string;
	cwd: string;
	parentSession?: string;
}

function isoTimestamp(value: unknown): string {
	if (typeof value === "string") return value;
	if (typeof value === "number") return new Date(value).toISOString();
	return new Date().toISOString();
}


function translateContentBlock(block: unknown): unknown {
	if (!block || typeof block !== "object") return block;
	const b = block as Record<string, unknown>;
	switch (b.type) {
		case "redacted_thinking":
			return { type: "redactedThinking", data: b.data };
		case "tool_call":
			return {
				type: "toolCall",
				id: b.id,
				name: b.name,
				arguments: b.arguments ?? {},
				...(b.signature !== undefined ? { signature: b.signature } : {}),
			};
		case "image":
			return { type: "image", data: b.data, mimeType: b.mime_type ?? b.mimeType };
		default:
			return b;
	}
}

/**
 * A message, translated from micro's `Message` shape (`crates/micro-types/src/lib.rs`: snake_case
 * fields, `role` tag values.
 */
function translateMessage(message: unknown): Record<string, unknown> {
	if (!message || typeof message !== "object") return message as Record<string, unknown>;
	const m = message as Record<string, unknown>;
	const content = Array.isArray(m.content) ? m.content.map(translateContentBlock) : m.content;
	switch (m.role) {
		case "user":
			return { role: "user", content, timestamp: m.timestamp };
		case "assistant": {
			const usage = (m.usage as Record<string, unknown>) ?? {};
			const input = Number(usage.input ?? 0);
			const output = Number(usage.output ?? 0);
			const cacheRead = Number(usage.cache_read ?? usage.cacheRead ?? 0);
			const cacheWrite = Number(usage.cache_write ?? usage.cacheWrite ?? 0);
			const rawStopReason = String(m.stop_reason ?? m.stopReason ?? "stop");
			return {
				role: "assistant",
				content,
				provider: m.provider,
				model: m.model,
				usage: {
					input,
					output,
					cacheRead,
					cacheWrite,
					totalTokens: input + output + cacheRead + cacheWrite,
					
					cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
				},
				stopReason: rawStopReason === "tool_use" ? "toolUse" : rawStopReason,
				...((m.error ?? m.errorMessage) !== undefined ? { errorMessage: m.error ?? m.errorMessage } : {}),
				timestamp: m.timestamp,
			};
		}
		case "tool_result":
		case "toolResult":
			return {
				role: "toolResult",
				toolCallId: m.tool_call_id ?? m.toolCallId,
				toolName: m.tool_name ?? m.toolName,
				content,
				isError: m.is_error ?? m.isError ?? false,
				timestamp: m.timestamp,
			};
		default:
			return m;
	}
}

type ParsedLine =
	| { kind: "header"; header: NormalizedHeader }
	| { kind: "entry"; entry: NormalizedMessageEntry }
	| { kind: "custom"; entry: NormalizedCustomEntry }
	| { kind: "compaction"; entry: NormalizedCompactionEntry }
	| { kind: "label"; entryId: string; label: string | undefined }
	| { kind: "bare"; message: unknown };


function parseLine(raw: unknown): ParsedLine | undefined {
	if (!raw || typeof raw !== "object") return undefined;
	const obj = raw as Record<string, unknown>;

	if (obj.type === "session") {
		return {
			kind: "header",
			header: {
				type: "session",
				version: Number(obj.version ?? CURRENT_SESSION_VERSION),
				id: String(obj.id ?? ""),
				timestamp: isoTimestamp(obj.timestamp),
				cwd: String(obj.cwd ?? ""),
				parentSession: typeof obj.parentSession === "string" ? obj.parentSession : undefined,
			},
		};
	}
	if ("message" in obj) {
		return {
			kind: "entry",
			entry: {
				type: "message",
				id: String(obj.id),
				parentId: (obj.parentId ?? obj.parent_id ?? null) as string | null,
				timestamp: isoTimestamp(obj.timestamp),
				message: translateMessage(obj.message),
			},
		};
	}
	if ("custom_type" in obj || "customType" in obj) {
		return {
			kind: "custom",
			entry: {
				type: "custom",
				id: String(obj.id),
				parentId: (obj.parentId ?? obj.parent_id ?? null) as string | null,
				timestamp: isoTimestamp(obj.timestamp),
				customType: String(obj.custom_type ?? obj.customType),
				data: obj.data,
			},
		};
	}
	if ("entry_id" in obj && "summary" in obj) {
		
		const entryId = String(obj.entry_id);
		return {
			kind: "compaction",
			entry: {
				type: "compaction",
				id: `compaction-${entryId}`,
				parentId: entryId,
				timestamp: isoTimestamp(obj.timestamp),
				summary: String(obj.summary),
				firstKeptEntryId: (obj.first_kept ?? obj.firstKeptEntryId ?? null) as string | null,
				tokensBefore: Number(obj.tokensBefore ?? 0),
			},
		};
	}
	if ("entry_id" in obj && "label" in obj) {
		return { kind: "label", entryId: String(obj.entry_id), label: obj.label == null ? undefined : String(obj.label) };
	}
	if ("targetId" in obj && "label" in obj) {
		
		return { kind: "label", entryId: String(obj.targetId), label: obj.label == null ? undefined : String(obj.label) };
	}
	if (typeof obj.role === "string") {
		return { kind: "bare", message: obj };
	}
	return undefined;
}

interface ParsedSession {
	header: NormalizedHeader | null;
	entries: NormalizedEntry[];
	byId: Map<string, NormalizedEntry>;
	labels: Map<string, string>;
	leafId: string | null;
}

/** Replay a session log into entries, labels and the current leaf. */
function parseSessionFile(path: string): ParsedSession {
	const raw = readFileSync(path, "utf-8");
	let header: NormalizedHeader | null = null;
	const entries: NormalizedEntry[] = [];
	const byId = new Map<string, NormalizedEntry>();
	const labels = new Map<string, string>();
	let leafId: string | null = null;
	let messageCount = 0;

	for (const line of raw.split("\n")) {
		if (!line.trim()) continue;
		let parsed: unknown;
		try {
			parsed = JSON.parse(line);
		} catch {
			continue;
		}
		const result = parseLine(parsed);
		if (!result) continue;

		switch (result.kind) {
			case "header":
				header = result.header;
				break;
			case "entry":
				entries.push(result.entry);
				byId.set(result.entry.id, result.entry);
				leafId = result.entry.id;
				messageCount++;
				break;
			case "custom":
				entries.push(result.entry);
				byId.set(result.entry.id, result.entry);
				break;
			case "compaction":
				entries.push(result.entry);
				byId.set(result.entry.id, result.entry);
				break;
			case "label":
				if (result.label === undefined) labels.delete(result.entryId);
				else labels.set(result.entryId, result.label);
				break;
			case "bare": {
				messageCount++;
				const id = String(messageCount);
				const entry: NormalizedMessageEntry = {
					type: "message",
					id,
					parentId: leafId,
					timestamp: isoTimestamp((result.message as Record<string, unknown>).timestamp),
					message: translateMessage(result.message),
				};
				entries.push(entry);
				byId.set(id, entry);
				leafId = id;
				break;
			}
		}
	}

	return { header, entries, byId, labels, leafId };
}

interface MetaSidecar {
	id: string;
	workspace: string;
	createdAt: number;
	title: string;
	parent?: string;
}


function readMetaSidecar(sessionPath: string): MetaSidecar | null {
	const metaPath = sessionPath.replace(/\.jsonl$/, ".meta.json");
	try {
		const raw = JSON.parse(readFileSync(metaPath, "utf-8")) as Record<string, unknown>;
		return {
			id: String(raw.id ?? ""),
			workspace: String(raw.workspace ?? ""),
			createdAt: Number(raw.created_at ?? raw.createdAt ?? 0),
			title: String(raw.title ?? ""),
			parent: typeof raw.parent === "string" ? raw.parent : undefined,
		};
	} catch {
		return null;
	}
}

function newEntryId(taken: Set<string>): string {
	for (let attempt = 0; attempt < 100; attempt++) {
		const id = randomUUID().slice(0, 8);
		if (!taken.has(id)) return id;
	}
	return randomUUID();
}

export class SessionManager {
	private file: string;
	private dir: string;
	private cwd: string;
	private sessionId: string;
	private sessionName: string | undefined;
	private header: NormalizedHeader | null;
	private entries: NormalizedEntry[];
	private byId: Map<string, NormalizedEntry>;
	private labels: Map<string, string>;
	private leafId: string | null;

	private constructor(file: string, dir: string, parsed: ParsedSession, meta: MetaSidecar | null) {
		this.file = file;
		this.dir = dir;
		this.entries = parsed.entries;
		this.byId = parsed.byId;
		this.labels = parsed.labels;
		this.leafId = parsed.leafId;

		if (parsed.header) {
			this.header = parsed.header;
			this.cwd = parsed.header.cwd;
			this.sessionId = parsed.header.id;
			this.sessionName = undefined;
		} else if (meta) {
			this.header = {
				type: "session",
				version: CURRENT_SESSION_VERSION,
				id: meta.id,
				timestamp: String(meta.createdAt),
				cwd: meta.workspace,
				parentSession: meta.parent,
			};
			this.cwd = meta.workspace;
			this.sessionId = meta.id;
			
			this.sessionName = meta.title || undefined;
		} else {
			this.header = null;
			this.cwd = "";
			this.sessionId = "";
			this.sessionName = undefined;
		}
	}

	static open(path: string, sessionDir?: string): SessionManager {
		const resolved = resolvePath(path);
		const parsed = parseSessionFile(resolved);
		const meta = parsed.header ? null : readMetaSidecar(resolved);
		return new SessionManager(resolved, sessionDir ?? dirname(resolved), parsed, meta);
	}

	getCwd(): string {
		return this.cwd;
	}

	getSessionDir(): string {
		return this.dir;
	}

	getSessionId(): string {
		return this.sessionId;
	}

	getSessionFile(): string {
		return this.file;
	}

	getSessionName(): string | undefined {
		return this.sessionName;
	}

	getHeader(): NormalizedHeader | null {
		return this.header;
	}

	getEntries(): NormalizedEntry[] {
		return this.entries;
	}

	getLeafId(): string | null {
		return this.leafId;
	}

	getLeafEntry(): NormalizedEntry | undefined {
		return this.leafId ? this.byId.get(this.leafId) : undefined;
	}

	getEntry(id: string): NormalizedEntry | undefined {
		return this.byId.get(id);
	}

	getLabel(id: string): string | undefined {
		return this.labels.get(id);
	}

	/** Walk from `fromId` (or the current leaf) to the root, root-first. */
	getBranch(fromId?: string): NormalizedEntry[] {
		const path: NormalizedEntry[] = [];
		let current = this.byId.get(fromId ?? this.leafId ?? "");
		while (current) {
			path.push(current);
			current = current.parentId ? this.byId.get(current.parentId) : undefined;
		}
		path.reverse();
		return path;
	}

	
	buildContextEntries(): NormalizedEntry[] {
		return this.getBranch();
	}

	getTree(): { entry: NormalizedEntry; children: unknown[]; label?: string }[] {
		const childrenOf = new Map<string | null, NormalizedEntry[]>();
		for (const entry of this.entries) {
			const siblings = childrenOf.get(entry.parentId) ?? [];
			siblings.push(entry);
			childrenOf.set(entry.parentId, siblings);
		}
		const node = (entry: NormalizedEntry): { entry: NormalizedEntry; children: unknown[]; label?: string } => ({
			entry,
			children: (childrenOf.get(entry.id) ?? []).map(node),
			label: this.labels.get(entry.id),
		});
		return (childrenOf.get(null) ?? []).map(node);
	}

	createBranchedSession(leafId: string): string | undefined {
		const path = this.getBranch(leafId);
		if (path.length === 0) {
			throw new Error(`Entry ${leafId} not found`);
		}

		let parentId: string | null = null;
		const rewritten = path.map((entry) => {
			const copy = { ...entry, parentId };
			parentId = entry.id;
			return copy;
		});

		const newSessionId = newEntryId(new Set(this.entries.map((e) => e.id)));
		const timestamp = new Date().toISOString();
		const fileTimestamp = timestamp.replace(/[:.]/g, "-");
		const newSessionFile = join(this.dir, `${fileTimestamp}_${newSessionId}.jsonl`);

		const newHeader: NormalizedHeader = {
			type: "session",
			version: CURRENT_SESSION_VERSION,
			id: newSessionId,
			timestamp,
			cwd: this.cwd,
			parentSession: this.file,
		};

		mkdirSync(dirname(newSessionFile), { recursive: true });
		const lines = [newHeader, ...rewritten].map((entry) => JSON.stringify(entry));
		writeFileSync(newSessionFile, `${lines.join("\n")}\n`, "utf-8");
		return newSessionFile;
	}
}



export const DEFAULT_MAX_LINES = 2000;
export const DEFAULT_MAX_BYTES = 50 * 1024;
export const GREP_MAX_LINE_LENGTH = 500;

interface TruncationOptions {
	maxLines?: number;
	maxBytes?: number;
}

interface TruncationResult {
	content: string;
	truncated: boolean;
	truncatedBy: "lines" | "bytes" | null;
	totalLines: number;
	totalBytes: number;
	outputLines: number;
	outputBytes: number;
	lastLinePartial: boolean;
	firstLineExceedsLimit: boolean;
	maxLines: number;
	maxBytes: number;
}

function splitLinesForCounting(content: string): string[] {
	if (content.length === 0) return [];
	const lines = content.split("\n");
	if (content.endsWith("\n")) lines.pop();
	return lines;
}

export function formatSize(bytes: number): string {
	if (bytes < 1024) return `${bytes}B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
	return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

export function truncateHead(content: string, options: TruncationOptions = {}): TruncationResult {
	const maxLines = options.maxLines ?? DEFAULT_MAX_LINES;
	const maxBytes = options.maxBytes ?? DEFAULT_MAX_BYTES;
	const totalBytes = Buffer.byteLength(content, "utf-8");
	const lines = splitLinesForCounting(content);
	const totalLines = lines.length;

	if (totalLines <= maxLines && totalBytes <= maxBytes) {
		return {
			content,
			truncated: false,
			truncatedBy: null,
			totalLines,
			totalBytes,
			outputLines: totalLines,
			outputBytes: totalBytes,
			lastLinePartial: false,
			firstLineExceedsLimit: false,
			maxLines,
			maxBytes,
		};
	}

	const firstLineBytes = Buffer.byteLength(lines[0], "utf-8");
	if (firstLineBytes > maxBytes) {
		return {
			content: "",
			truncated: true,
			truncatedBy: "bytes",
			totalLines,
			totalBytes,
			outputLines: 0,
			outputBytes: 0,
			lastLinePartial: false,
			firstLineExceedsLimit: true,
			maxLines,
			maxBytes,
		};
	}

	const outputLinesArr: string[] = [];
	let outputBytesCount = 0;
	let truncatedBy: "lines" | "bytes" = "lines";
	for (let i = 0; i < lines.length && i < maxLines; i++) {
		const line = lines[i];
		const lineBytes = Buffer.byteLength(line, "utf-8") + (i > 0 ? 1 : 0);
		if (outputBytesCount + lineBytes > maxBytes) {
			truncatedBy = "bytes";
			break;
		}
		outputLinesArr.push(line);
		outputBytesCount += lineBytes;
	}
	if (outputLinesArr.length >= maxLines && outputBytesCount <= maxBytes) truncatedBy = "lines";

	const outputContent = outputLinesArr.join("\n");
	return {
		content: outputContent,
		truncated: true,
		truncatedBy,
		totalLines,
		totalBytes,
		outputLines: outputLinesArr.length,
		outputBytes: Buffer.byteLength(outputContent, "utf-8"),
		lastLinePartial: false,
		firstLineExceedsLimit: false,
		maxLines,
		maxBytes,
	};
}

function truncateStringToBytesFromEnd(str: string, maxBytes: number): string {
	const buf = Buffer.from(str, "utf-8");
	if (buf.length <= maxBytes) return str;
	let start = buf.length - maxBytes;
	while (start < buf.length && (buf[start] & 0xc0) === 0x80) start++;
	return buf.subarray(start).toString("utf-8");
}

export function truncateTail(content: string, options: TruncationOptions = {}): TruncationResult {
	const maxLines = options.maxLines ?? DEFAULT_MAX_LINES;
	const maxBytes = options.maxBytes ?? DEFAULT_MAX_BYTES;
	const totalBytes = Buffer.byteLength(content, "utf-8");
	const lines = splitLinesForCounting(content);
	const totalLines = lines.length;

	if (totalLines <= maxLines && totalBytes <= maxBytes) {
		return {
			content,
			truncated: false,
			truncatedBy: null,
			totalLines,
			totalBytes,
			outputLines: totalLines,
			outputBytes: totalBytes,
			lastLinePartial: false,
			firstLineExceedsLimit: false,
			maxLines,
			maxBytes,
		};
	}

	const outputLinesArr: string[] = [];
	let outputBytesCount = 0;
	let truncatedBy: "lines" | "bytes" = "lines";
	let lastLinePartial = false;
	for (let i = lines.length - 1; i >= 0 && outputLinesArr.length < maxLines; i--) {
		const line = lines[i];
		const lineBytes = Buffer.byteLength(line, "utf-8") + (outputLinesArr.length > 0 ? 1 : 0);
		if (outputBytesCount + lineBytes > maxBytes) {
			truncatedBy = "bytes";
			if (outputLinesArr.length === 0) {
				const truncatedLine = truncateStringToBytesFromEnd(line, maxBytes);
				outputLinesArr.unshift(truncatedLine);
				outputBytesCount = Buffer.byteLength(truncatedLine, "utf-8");
				lastLinePartial = true;
			}
			break;
		}
		outputLinesArr.unshift(line);
		outputBytesCount += lineBytes;
	}
	if (outputLinesArr.length >= maxLines && outputBytesCount <= maxBytes) truncatedBy = "lines";

	const outputContent = outputLinesArr.join("\n");
	return {
		content: outputContent,
		truncated: true,
		truncatedBy,
		totalLines,
		totalBytes,
		outputLines: outputLinesArr.length,
		outputBytes: Buffer.byteLength(outputContent, "utf-8"),
		lastLinePartial,
		firstLineExceedsLimit: false,
		maxLines,
		maxBytes,
	};
}

export function truncateLine(line: string, maxChars: number = GREP_MAX_LINE_LENGTH): { text: string; wasTruncated: boolean } {
	if (line.length <= maxChars) return { text: line, wasTruncated: false };
	return { text: `${line.slice(0, maxChars)}... [truncated]`, wasTruncated: true };
}



const fileMutationQueues = new Map<string, Promise<void>>();
let registrationQueue = Promise.resolve();

function isMissingPathError(error: unknown): boolean {
	return (
		typeof error === "object" &&
		error !== null &&
		"code" in error &&
		((error as { code?: string }).code === "ENOENT" || (error as { code?: string }).code === "ENOTDIR")
	);
}

async function getMutationQueueKey(filePath: string): Promise<string> {
	const resolved = resolvePath(filePath);
	try {
		const { realpath } = await import("node:fs/promises");
		return await realpath(resolved);
	} catch (error) {
		if (isMissingPathError(error)) return resolved;
		throw error;
	}
}

export async function withFileMutationQueue<T>(filePath: string, fn: () => Promise<T>): Promise<T> {
	const registration = registrationQueue.then(async () => {
		const key = await getMutationQueueKey(filePath);
		const currentQueue = fileMutationQueues.get(key) ?? Promise.resolve();
		let releaseNext!: () => void;
		const nextQueue = new Promise<void>((resolveQueue) => {
			releaseNext = resolveQueue;
		});
		const chainedQueue = currentQueue.then(() => nextQueue);
		fileMutationQueues.set(key, chainedQueue);
		return { key, currentQueue, chainedQueue, releaseNext };
	});
	registrationQueue = registration.then(
		() => undefined,
		() => undefined,
	);

	const { key, currentQueue, chainedQueue, releaseNext } = await registration;
	await currentQueue;
	try {
		return await fn();
	} finally {
		releaseNext();
		if (fileMutationQueues.get(key) === chainedQueue) {
			fileMutationQueues.delete(key);
		}
	}
}



interface LooseEntry {
	type?: string;
	id?: string;
	parentId?: string | null;
	timestamp?: string;
	[key: string]: unknown;
}

export function getLatestCompactionEntry(entries: LooseEntry[]): LooseEntry | null {
	for (let i = entries.length - 1; i >= 0; i--) {
		if (entries[i].type === "compaction") return entries[i];
	}
	return null;
}

/** One selected entry, projected into the messages it contributes to LLM context. */
export function sessionEntryToContextMessages(entry: LooseEntry): unknown[] {
	switch (entry.type) {
		case "message":
			return [entry.message];
		case "compaction":
			return [createCompactionSummaryMessage(String(entry.summary ?? ""), Number(entry.tokensBefore ?? 0), String(entry.timestamp ?? ""))];
		case "custom_message":
			return [
				createCustomMessage(
					String(entry.customType ?? ""),
					(entry.content as string | unknown[] | undefined) ?? [],
					Boolean(entry.display),
					entry.details,
					String(entry.timestamp ?? ""),
				),
			];
		case "branch_summary":
			if (!entry.summary) return [];
			return [createBranchSummaryMessage(String(entry.summary), String(entry.fromId ?? ""), String(entry.timestamp ?? ""))];
		default:
			return [];
	}
}

export function parseSessionEntries(content: string): LooseEntry[] {
	const entries: LooseEntry[] = [];
	for (const line of content.trim().split("\n")) {
		if (!line.trim()) continue;
		try {
			entries.push(JSON.parse(line) as LooseEntry);
		} catch {
			
		}
	}
	return entries;
}

function sessionEntryPath(entries: LooseEntry[], leafId?: string | null, byId?: Map<string, LooseEntry>): LooseEntry[] {
	const index = byId ?? new Map(entries.map((entry) => [entry.id ?? "", entry]));
	let leaf: LooseEntry | undefined;
	if (leafId === null) return [];
	if (leafId) leaf = index.get(leafId);
	leaf ??= entries[entries.length - 1];
	if (!leaf) return [];

	const path: LooseEntry[] = [];
	let current: LooseEntry | undefined = leaf;
	while (current) {
		path.push(current);
		current = current.parentId ? index.get(current.parentId) : undefined;
	}
	path.reverse();
	return path;
}


export function buildContextEntries(entries: LooseEntry[], leafId?: string | null, byId?: Map<string, LooseEntry>): LooseEntry[] {
	const path = sessionEntryPath(entries, leafId, byId);
	let compaction: LooseEntry | null = null;
	for (const entry of path) {
		if (entry.type === "compaction") compaction = entry;
	}
	if (!compaction) return path;

	const compactionIndex = path.findIndex((entry) => entry.id === compaction.id);
	if (compactionIndex < 0) return path;

	const contextEntries: LooseEntry[] = [compaction];
	let foundFirstKept = false;
	for (let i = 0; i < compactionIndex; i++) {
		if (path[i].id === compaction.firstKeptEntryId) foundFirstKept = true;
		if (foundFirstKept) contextEntries.push(path[i]);
	}
	contextEntries.push(...path.slice(compactionIndex + 1));
	return contextEntries;
}

/** `{messages, thinkingLevel, model}` for the LLM. */
export function buildSessionContext(
	entries: LooseEntry[],
	leafId?: string | null,
	byId?: Map<string, LooseEntry>,
): { messages: unknown[]; thinkingLevel: string; model: { provider: string; modelId: string } | null } {
	const path = sessionEntryPath(entries, leafId, byId);
	let thinkingLevel = "off";
	let model: { provider: string; modelId: string } | null = null;
	for (const entry of path) {
		if (entry.type === "thinking_level_change") {
			thinkingLevel = String(entry.thinkingLevel ?? thinkingLevel);
		} else if (entry.type === "model_change") {
			model = { provider: String(entry.provider ?? ""), modelId: String(entry.modelId ?? "") };
		} else if (entry.type === "message") {
			const message = entry.message as Record<string, unknown> | undefined;
			if (message?.role === "assistant") {
				model = { provider: String(message.provider ?? ""), modelId: String(message.model ?? "") };
			}
		}
	}
	const messages = buildContextEntries(entries, leafId, byId).flatMap(sessionEntryToContextMessages);
	return { messages, thinkingLevel, model };
}



function migrateV1ToV2(entries: LooseEntry[]): void {
	const ids = new Set<string>();
	let previousId: string | null = null;
	for (const entry of entries) {
		if (entry.type === "session") {
			entry.version = 2;
			continue;
		}
		entry.id = newEntryId(ids);
		ids.add(entry.id);
		entry.parentId = previousId;
		previousId = entry.id;

		if (entry.type === "compaction" && typeof entry.firstKeptEntryIndex === "number") {
			const target = entries[entry.firstKeptEntryIndex as number];
			if (target && target.type !== "session") {
				entry.firstKeptEntryId = target.id;
			}
			delete entry.firstKeptEntryIndex;
		}
	}
}

function migrateV2ToV3(entries: LooseEntry[]): void {
	for (const entry of entries) {
		if (entry.type === "session") {
			entry.version = 3;
			continue;
		}
		if (entry.type === "message") {
			const message = entry.message as Record<string, unknown> | undefined;
			if (message?.role === "hookMessage") {
				message.role = "custom";
			}
		}
	}
}

export function migrateSessionEntries(entries: LooseEntry[]): void {
	const header = entries.find((entry) => entry.type === "session");
	const version = Number(header?.version ?? 1);
	if (version >= CURRENT_SESSION_VERSION) return;
	if (version < 2) migrateV1ToV2(entries);
	if (version < 3) migrateV2ToV3(entries);
}



export function defineTool<T>(tool: T): T {
	return tool;
}

interface ToolNamed {
	toolName: string;
}

export function isBashToolResult(e: ToolNamed): boolean {
	return e.toolName === "bash";
}
export function isReadToolResult(e: ToolNamed): boolean {
	return e.toolName === "read";
}
export function isEditToolResult(e: ToolNamed): boolean {
	return e.toolName === "edit";
}
export function isWriteToolResult(e: ToolNamed): boolean {
	return e.toolName === "write";
}
export function isGrepToolResult(e: ToolNamed): boolean {
	return e.toolName === "grep";
}
export function isFindToolResult(e: ToolNamed): boolean {
	return e.toolName === "find";
}
export function isLsToolResult(e: ToolNamed): boolean {
	return e.toolName === "ls";
}
export function isToolCallEventType(toolName: string, event: ToolNamed): boolean {
	return event.toolName === toolName;
}



export const CONFIG_DIR_NAME = ".micro";
export const VERSION = "0.0.0-micro-compat";
export const CURRENT_SESSION_VERSION = 3;



function unsupported(name: string): (..._args: unknown[]) => never {
	return function unsupportedPiCodingAgentExport(): never {
		throw new Error(
			`pi-coding-agent export ${name} is unavailable in micro; use the extension API instead`,
		);
	};
}

const STUB_NAMES = [
	"parseArgs",
	"getDocsPath",
	"getExamplesPath",
	"getPackageDir",
	"getReadmePath",
	"AgentSession",
	"readStoredCredential",
	"calculateContextTokens",
	"collectEntriesForBranchSummary",
	"compact",
	"DEFAULT_COMPACTION_SETTINGS",
	"estimateTokens",
	"findCutPoint",
	"findTurnStartIndex",
	"generateBranchSummary",
	"generateSummary",
	"generateSummaryWithUsage",
	"getLastAssistantUsage",
	"prepareBranchEntries",
	"shouldCompact",
	"createEventBus",
	"createExtensionRuntime",
	"discoverAndLoadExtensions",
	"ExtensionRunner",
	"wrapRegisteredTool",
	"wrapRegisteredTools",
	"ModelRegistry",
	"resolveCliModel",
	"resolveModelScopeWithDiagnostics",
	"CredentialSynchronizationError",
	"ModelRuntime",
	"DefaultPackageManager",
	"DefaultResourceLoader",
	"loadProjectContextFiles",
	"AgentSessionRuntime",
	"createAgentSession",
	"createAgentSessionFromServices",
	"createAgentSessionRuntime",
	"createAgentSessionServices",
	"SettingsManager",
	"formatSkillsForPrompt",
	"loadSkills",
	"loadSkillsFromDir",
	"createSyntheticSourceInfo",
	"generateDiffString",
	"generateUnifiedPatch",
	"createBashToolDefinition",
	"createEditToolDefinition",
	"createFindToolDefinition",
	"createGrepToolDefinition",
	"createLocalBashOperations",
	"createLsToolDefinition",
	"createReadToolDefinition",
	"createWriteToolDefinition",
	"hasTrustRequiringProjectResources",
	"ProjectTrustStore",
	"main",
	"InteractiveMode",
	"RpcClient",
	"runPrintMode",
	"runRpcMode",
	"ArminComponent",
	"AssistantMessageComponent",
	"BashExecutionComponent",
	"BranchSummaryMessageComponent",
	"CompactionSummaryMessageComponent",
	"CustomMessageComponent",
	"ExtensionEditorComponent",
	"ExtensionInputComponent",
	"ExtensionSelectorComponent",
	"FooterComponent",
	"LoginDialogComponent",
	"ModelSelectorComponent",
	"OAuthSelectorComponent",
	"renderDiff",
	"SessionSelectorComponent",
	"SettingsSelectorComponent",
	"ShowImagesSelectorComponent",
	"SkillInvocationMessageComponent",
	"ThemeSelectorComponent",
	"ThinkingSelectorComponent",
	"ToolExecutionComponent",
	"TreeSelectorComponent",
	"truncateToVisualLines",
	"UserMessageComponent",
	"UserMessageSelectorComponent",
	"initTheme",
	"Theme",
	"copyToClipboard",
	"convertToPng",
	"formatDimensionNote",
	"resizeImage",
	"getShellConfig",
] as const;

const stubs = Object.fromEntries(STUB_NAMES.map((name) => [name, unsupported(name)])) as Record<
	(typeof STUB_NAMES)[number],
	(..._args: unknown[]) => never
>;

export const {
	parseArgs,
	getDocsPath,
	getExamplesPath,
	getPackageDir,
	getReadmePath,
	AgentSession,
	readStoredCredential,
	calculateContextTokens,
	collectEntriesForBranchSummary,
	compact,
	DEFAULT_COMPACTION_SETTINGS,
	estimateTokens,
	findCutPoint,
	findTurnStartIndex,
	generateBranchSummary,
	generateSummary,
	generateSummaryWithUsage,
	getLastAssistantUsage,
	prepareBranchEntries,
	shouldCompact,
	createEventBus,
	createExtensionRuntime,
	discoverAndLoadExtensions,
	ExtensionRunner,
	wrapRegisteredTool,
	wrapRegisteredTools,
	ModelRegistry,
	resolveCliModel,
	resolveModelScopeWithDiagnostics,
	CredentialSynchronizationError,
	ModelRuntime,
	DefaultPackageManager,
	DefaultResourceLoader,
	loadProjectContextFiles,
	AgentSessionRuntime,
	createAgentSession,
	createAgentSessionFromServices,
	createAgentSessionRuntime,
	createAgentSessionServices,
	SettingsManager,
	formatSkillsForPrompt,
	loadSkills,
	loadSkillsFromDir,
	createSyntheticSourceInfo,
	generateDiffString,
	generateUnifiedPatch,
	createBashToolDefinition,
	createEditToolDefinition,
	createFindToolDefinition,
	createGrepToolDefinition,
	createLocalBashOperations,
	createLsToolDefinition,
	createReadToolDefinition,
	createWriteToolDefinition,
	hasTrustRequiringProjectResources,
	ProjectTrustStore,
	main,
	InteractiveMode,
	RpcClient,
	runPrintMode,
	runRpcMode,
	ArminComponent,
	AssistantMessageComponent,
	BashExecutionComponent,
	BranchSummaryMessageComponent,
	CompactionSummaryMessageComponent,
	CustomMessageComponent,
	ExtensionEditorComponent,
	ExtensionInputComponent,
	ExtensionSelectorComponent,
	FooterComponent,
	LoginDialogComponent,
	ModelSelectorComponent,
	OAuthSelectorComponent,
	renderDiff,
	SessionSelectorComponent,
	SettingsSelectorComponent,
	ShowImagesSelectorComponent,
	SkillInvocationMessageComponent,
	ThemeSelectorComponent,
	ThinkingSelectorComponent,
	ToolExecutionComponent,
	TreeSelectorComponent,
	truncateToVisualLines,
	UserMessageComponent,
	UserMessageSelectorComponent,
	initTheme,
	Theme,
	copyToClipboard,
	convertToPng,
	formatDimensionNote,
	resizeImage,
	getShellConfig,
} = stubs;
