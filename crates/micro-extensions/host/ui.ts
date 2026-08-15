

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join } from "node:path";

import { getKeybindings, type KeybindingsManager } from "@earendil-works/pi-tui";
import { type Component, dispose as disposeComponent, pushChanged, registerComponent } from "./host-components.ts";
import { type Json, wireFor } from "./host-wire.ts";

/** A handler `ctx.ui.onTerminalInput` registered, and what it asked to do with a key. */
type TerminalInputHandler = (data: string) => { consume?: boolean; data?: string } | undefined;

/** Extensions currently listening for raw terminal input, in the order they registered. */
const terminalInputHandlers = new Set<TerminalInputHandler>();

/** Hand a key micro read to every registered handler, in registration order, until one consumes it. */
export async function dispatchTerminalInput(data: string): Promise<Json> {
	for (const handler of terminalInputHandlers) {
		const said = handler(data);
		if (said?.consume) {
			return { consume: true };
		}
	}
	return { consume: false };
}

/** The last text this extension itself put in the editor, echoed back by `getEditorText`. */
let echoedEditorText = "";


export function noteEditorText(text: string): void {
	echoedEditorText = text;
}

/** The last state this extension itself set tool output expansion to. */
let echoedToolsExpanded = false;



/** A color as pi's theme schema carries it: a hex string, a var name, or a 256-color index. */
type ColorValue = string | number;

/**
 * One token's resolved color, in the two forms `fg`/`bg` know how to wrap: a hex string, or a
 * 256-color index.
 */
type Resolved = { hex: string } | { index: number };

function resolveVar(value: ColorValue, vars: Record<string, ColorValue>, seen: Set<string>): Resolved {
	if (typeof value === "number") {
		return { index: value };
	}
	if (value === "" || value.startsWith("#")) {
		return { hex: value };
	}
	if (seen.has(value)) {
		throw new Error(`circular variable reference: ${value}`);
	}
	seen.add(value);
	const next = vars[value];
	if (next === undefined) {
		throw new Error(`unknown variable: ${value}`);
	}
	return resolveVar(next, vars, seen);
}

/** Every token pi's theme schema requires, transcribed from `theme/palette.rs`'s `TOKENS`. */
const THEME_TOKENS = [
	"accent",
	"border",
	"borderAccent",
	"borderMuted",
	"success",
	"error",
	"warning",
	"muted",
	"dim",
	"text",
	"thinkingText",
	"selectedBg",
	"userMessageBg",
	"userMessageText",
	"customMessageBg",
	"customMessageText",
	"customMessageLabel",
	"toolPendingBg",
	"toolSuccessBg",
	"toolErrorBg",
	"toolTitle",
	"toolOutput",
	"mdHeading",
	"mdLink",
	"mdLinkUrl",
	"mdCode",
	"mdCodeBlock",
	"mdCodeBlockBorder",
	"mdQuote",
	"mdQuoteBorder",
	"mdHr",
	"mdListBullet",
	"toolDiffAdded",
	"toolDiffRemoved",
	"toolDiffContext",
	"syntaxComment",
	"syntaxKeyword",
	"syntaxFunction",
	"syntaxVariable",
	"syntaxString",
	"syntaxNumber",
	"syntaxType",
	"syntaxOperator",
	"syntaxPunctuation",
	"thinkingOff",
	"thinkingMinimal",
	"thinkingLow",
	"thinkingMedium",
	"thinkingHigh",
	"thinkingXhigh",
	"thinkingMax",
	"bashMode",
] as const;


const BUILT_IN_THEMES: Record<string, Record<string, string>> = {
	dark: {
		accent: "#8abeb7",
		border: "#5f87ff",
		borderAccent: "#00d7ff",
		borderMuted: "#505050",
		success: "#b5bd68",
		error: "#cc6666",
		warning: "#ffff00",
		muted: "#808080",
		dim: "#666666",
		text: "#d4d4d4",
		thinkingText: "#808080",
		selectedBg: "#3a3a4a",
		userMessageBg: "#343541",
		userMessageText: "#d4d4d4",
		customMessageBg: "#2d2838",
		customMessageText: "#d4d4d4",
		customMessageLabel: "#9575cd",
		toolPendingBg: "#282832",
		toolSuccessBg: "#283228",
		toolErrorBg: "#3c2828",
		toolTitle: "#d4d4d4",
		toolOutput: "#808080",
		mdHeading: "#f0c674",
		mdLink: "#81a2be",
		mdLinkUrl: "#666666",
		mdCode: "#8abeb7",
		mdCodeBlock: "#b5bd68",
		mdCodeBlockBorder: "#808080",
		mdQuote: "#808080",
		mdQuoteBorder: "#808080",
		mdHr: "#808080",
		mdListBullet: "#8abeb7",
		toolDiffAdded: "#b5bd68",
		toolDiffRemoved: "#cc6666",
		toolDiffContext: "#808080",
		syntaxComment: "#6A9955",
		syntaxKeyword: "#569CD6",
		syntaxFunction: "#DCDCAA",
		syntaxVariable: "#9CDCFE",
		syntaxString: "#CE9178",
		syntaxNumber: "#B5CEA8",
		syntaxType: "#4EC9B0",
		syntaxOperator: "#D4D4D4",
		syntaxPunctuation: "#D4D4D4",
		thinkingOff: "#505050",
		thinkingMinimal: "#6e6e6e",
		thinkingLow: "#5f87af",
		thinkingMedium: "#81a2be",
		thinkingHigh: "#b294bb",
		thinkingXhigh: "#d183e8",
		thinkingMax: "#ff5fff",
		bashMode: "#b5bd68",
	},
	light: {
		accent: "#5a8080",
		border: "#547da7",
		borderAccent: "#5a8080",
		borderMuted: "#b0b0b0",
		success: "#588458",
		error: "#aa5555",
		warning: "#9a7326",
		muted: "#6c6c6c",
		dim: "#767676",
		text: "#1f2328",
		thinkingText: "#6c6c6c",
		selectedBg: "#d0d0e0",
		userMessageBg: "#e8e8e8",
		userMessageText: "#1f2328",
		customMessageBg: "#ede7f6",
		customMessageText: "#1f2328",
		customMessageLabel: "#7e57c2",
		toolPendingBg: "#e8e8f0",
		toolSuccessBg: "#e8f0e8",
		toolErrorBg: "#f0e8e8",
		toolTitle: "#1f2328",
		toolOutput: "#6c6c6c",
		mdHeading: "#9a7326",
		mdLink: "#547da7",
		mdLinkUrl: "#767676",
		mdCode: "#5a8080",
		mdCodeBlock: "#588458",
		mdCodeBlockBorder: "#6c6c6c",
		mdQuote: "#6c6c6c",
		mdQuoteBorder: "#6c6c6c",
		mdHr: "#6c6c6c",
		mdListBullet: "#588458",
		toolDiffAdded: "#588458",
		toolDiffRemoved: "#aa5555",
		toolDiffContext: "#6c6c6c",
		syntaxComment: "#008000",
		syntaxKeyword: "#0000FF",
		syntaxFunction: "#795E26",
		syntaxVariable: "#001080",
		syntaxString: "#A31515",
		syntaxNumber: "#098658",
		syntaxType: "#267F99",
		syntaxOperator: "#000000",
		syntaxPunctuation: "#000000",
		thinkingOff: "#b0b0b0",
		thinkingMinimal: "#767676",
		thinkingLow: "#547da7",
		thinkingMedium: "#5a8080",
		thinkingHigh: "#875f87",
		thinkingXhigh: "#8b008b",
		thinkingMax: "#af005f",
		bashMode: "#588458",
	},
};


function themesDir(): string {
	return join(configDir(), "themes");
}


function configDir(): string {
	const configured = process.env.MICRO_DIR?.trim();
	if (configured) {
		return configured;
	}
	const home = process.env.HOME?.trim() || process.env.USERPROFILE?.trim() || homedir();
	const legacy = join(home, ".micro");
	if (existsSync(legacy)) {
		return legacy;
	}
	const base = process.env.XDG_CONFIG_HOME?.trim();
	return join(base && isAbsolute(base) ? base : join(home, ".config"), "micro");
}

/** A theme file's `colors` block, every token resolved through its `vars` block. */
function readThemeFile(path: string): { name: string; colors: Record<string, Resolved> } {
	const document = JSON.parse(readFileSync(path, "utf-8")) as {
		name?: string;
		vars?: Record<string, ColorValue>;
		colors?: Record<string, ColorValue>;
	};
	if (!document.name) {
		throw new Error("a theme needs a name");
	}
	if (!document.colors) {
		throw new Error("a theme needs a colors block");
	}
	const vars = document.vars ?? {};
	const colors: Record<string, Resolved> = {};
	for (const token of THEME_TOKENS) {
		const value = document.colors[token];
		if (value === undefined) {
			throw new Error(`missing color: ${token}`);
		}
		colors[token] = resolveVar(value, vars, new Set());
	}
	return { name: document.name, colors };
}

/** A built-in theme's colors, already resolved. */
function builtInColors(name: string): Record<string, Resolved> {
	const colors: Record<string, Resolved> = {};
	for (const [token, hex] of Object.entries(BUILT_IN_THEMES[name])) {
		colors[token] = { hex };
	}
	return colors;
}

/** A theme by name: one of the two built in, or a file in the themes directory. */
function resolveTheme(name: string): { name: string; colors: Record<string, Resolved> } | undefined {
	if (name in BUILT_IN_THEMES) {
		return { name, colors: builtInColors(name) };
	}
	
	if (!name || name.includes("/") || name.includes("\\") || name.includes("..")) {
		return undefined;
	}
	const path = join(themesDir(), `${name}.json`);
	if (!existsSync(path)) {
		return undefined;
	}
	return readThemeFile(path);
}

function fgAnsi(color: Resolved): string {
	if ("index" in color) {
		return `\x1b[38;5;${color.index}m`;
	}
	if (color.hex === "") {
		return "\x1b[39m";
	}
	const value = Number.parseInt(color.hex.slice(1), 16);
	return `\x1b[38;2;${(value >> 16) & 0xff};${(value >> 8) & 0xff};${value & 0xff}m`;
}

function bgAnsi(color: Resolved): string {
	if ("index" in color) {
		return `\x1b[48;5;${color.index}m`;
	}
	if (color.hex === "") {
		return "\x1b[49m";
	}
	const value = Number.parseInt(color.hex.slice(1), 16);
	return `\x1b[48;2;${(value >> 16) & 0xff};${(value >> 8) & 0xff};${value & 0xff}m`;
}


class ExtensionTheme {
	readonly name: string;
	private readonly colors: Record<string, Resolved>;

	constructor(name: string, colors: Record<string, Resolved>) {
		this.name = name;
		this.colors = colors;
	}

	fg(color: string, text: string): string {
		const resolved = this.colors[color];
		if (!resolved) {
			throw new Error(`unknown theme color: ${color}`);
		}
		return `${fgAnsi(resolved)}${text}\x1b[39m`;
	}

	bg(color: string, text: string): string {
		const resolved = this.colors[color];
		if (!resolved) {
			throw new Error(`unknown theme background color: ${color}`);
		}
		return `${bgAnsi(resolved)}${text}\x1b[49m`;
	}

	
	bold(text: string): string {
		return `\x1b[1m${text}\x1b[22m`;
	}

	italic(text: string): string {
		return `\x1b[3m${text}\x1b[23m`;
	}

	underline(text: string): string {
		return `\x1b[4m${text}\x1b[24m`;
	}

	inverse(text: string): string {
		return `\x1b[7m${text}\x1b[27m`;
	}

	strikethrough(text: string): string {
		return `\x1b[9m${text}\x1b[29m`;
	}

	/** The colour on its own, for a caller placing it and closing it itself. */
	getFgAnsi(color: string): string {
		const resolved = this.colors[color];
		if (!resolved) {
			throw new Error(`unknown theme color: ${color}`);
		}
		return fgAnsi(resolved);
	}

	getBgAnsi(color: string): string {
		const resolved = this.colors[color];
		if (!resolved) {
			throw new Error(`unknown theme background color: ${color}`);
		}
		return bgAnsi(resolved);
	}
}

/** The theme `ctx.ui.theme` currently answers with. */
let currentTheme = new ExtensionTheme("dark", builtInColors("dark"));




function tuiHandle(id: { current?: string }): { requestRender(): void } {
	return {
		requestRender(): void {
			if (id.current) {
				pushChanged(id.current);
			}
		},
	};
}

/** Live widgets set through `setWidget`'s factory overload, by the key they were set under. */
const widgetComponentIds = new Map<string, string>();

let headerComponentId: string | undefined;
let footerComponentId: string | undefined;

/**
 * The factory `setEditorComponent` was last given, and the id its component is registered under
 * while it is active.
 */
let editorComponentFactory: EditorFactory | undefined;
let editorComponentId: string | undefined;

type EditorFactory = (tui: unknown, theme: ExtensionTheme, keybindings: KeybindingsManager) => Component;

/**
 * A handler `ctx.ui.addAutocompleteProvider` registered, wrapping whatever provider came before
 * it.
 */
type AutocompleteProviderFactory = (current: AutocompleteProvider) => AutocompleteProvider;

/** Shaped exactly as pi-tui's own `AutocompleteItem`. */
export interface AutocompleteItem {
	value: string;
	label: string;
	description?: string;
}

interface AutocompleteSuggestions {
	items: AutocompleteItem[];
	prefix: string;
}

/** Also shaped after pi-tui's real interface. */
interface AutocompleteProvider {
	triggerCharacters?: string[];
	getSuggestions(
		lines: string[],
		cursorLine: number,
		cursorCol: number,
		options: { signal: AbortSignal; force?: boolean },
	): Promise<AutocompleteSuggestions | null>;
	applyCompletion(
		lines: string[],
		cursorLine: number,
		cursorCol: number,
		item: AutocompleteItem,
		prefix: string,
	): { lines: string[]; cursorLine: number; cursorCol: number };
	shouldTriggerFileCompletion?(lines: string[], cursorLine: number, cursorCol: number): boolean;
}

/**
 * The built-in splice every provider in the chain can fall back to by delegating to the one it
 * wrapped: replace `prefix`.
 */
function spliceCompletion(
	lines: string[],
	cursorLine: number,
	cursorCol: number,
	item: AutocompleteItem,
	prefix: string,
): { lines: string[]; cursorLine: number; cursorCol: number } {
	const line = lines[cursorLine] ?? "";
	const start = Math.max(0, cursorCol - prefix.length);
	const value = `${item.value} `;
	const nextLines = lines.slice();
	nextLines[cursorLine] = line.slice(0, start) + value + line.slice(cursorCol);
	return { lines: nextLines, cursorLine, cursorCol: start + value.length };
}


const noAutocompleteSuggestions: AutocompleteProvider = {
	getSuggestions: async () => null,
	applyCompletion: spliceCompletion,
};

let autocompleteProvider: AutocompleteProvider | undefined;


const autocompleteTriggers = new Set<string>();

/** Ask whatever `addAutocompleteProvider` chain is registered for suggestions at the cursor. */
export async function dispatchSuggestions(
	lines: string[],
	cursorLine: number,
	cursorCol: number,
	force: boolean,
): Promise<Json | undefined> {
	if (!autocompleteProvider) {
		return undefined;
	}
	const controller = new AbortController();
	const suggestions = await autocompleteProvider.getSuggestions(lines, cursorLine, cursorCol, {
		signal: controller.signal,
		force,
	});
	return suggestions ? { items: suggestions.items, prefix: suggestions.prefix } : undefined;
}


export async function dispatchApplyCompletion(
	lines: string[],
	cursorLine: number,
	cursorCol: number,
	item: AutocompleteItem,
	prefix: string,
): Promise<Json> {
	const provider = autocompleteProvider ?? noAutocompleteSuggestions;
	return provider.applyCompletion(lines, cursorLine, cursorCol, item, prefix);
}

/** The interface as an extension sees it. */
export function uiFor(extension: string): Json {
	const { ask, send } = wireFor(extension);
	return {
		async select(title: string, options: string[], opts?: Json): Promise<string | undefined> {
			const answer = await ask({ type: "ui_request", method: "select", title, options, opts });
			return answer.value as string | undefined;
		},

		async confirm(title: string, message: string, opts?: Json): Promise<boolean> {
			const answer = await ask({ type: "ui_request", method: "confirm", title, message, opts });
			return answer.confirmed === true;
		},

		async input(title: string, placeholder?: string, opts?: Json): Promise<string | undefined> {
			const answer = await ask({ type: "ui_request", method: "input", title, placeholder, opts });
			return answer.value as string | undefined;
		},

		notify(message: string, notifyType?: "info" | "warning" | "error"): void {
			send({ type: "ui_request", method: "notify", message, notifyType: notifyType ?? "info" });
		},

		/** Listen to every key micro reads, before it decides what to do with it itself. */
		onTerminalInput(handler: TerminalInputHandler): () => void {
			
			const goingFromNoneToSome = terminalInputHandlers.size === 0;
			terminalInputHandlers.add(handler);
			if (goingFromNoneToSome) {
				send({ type: "action", action: "watch_terminal_input" });
			}
			return () => {
				const had = terminalInputHandlers.delete(handler);
				if (had && terminalInputHandlers.size === 0) {
					send({ type: "action", action: "unwatch_terminal_input" });
				}
			};
		},

		setStatus(statusKey: string, statusText?: string): void {
			send({ type: "ui_request", method: "setStatus", statusKey, statusText });
		},

		/** Call with nothing to restore the built-in message. */
		setWorkingMessage(message?: string): void {
			send({ type: "ui_request", method: "setWorkingMessage", message });
		},

		setWorkingVisible(visible: boolean): void {
			send({ type: "ui_request", method: "setWorkingVisible", visible });
		},

		/** Call with nothing to restore the default animated spinner. */
		setWorkingIndicator(options?: { frames?: string[]; intervalMs?: number }): void {
			send({
				type: "ui_request",
				method: "setWorkingIndicator",
				
				reset: options === undefined,
				frames: options?.frames ?? [],
				intervalMs: options?.intervalMs,
			});
		},

		/** Call with nothing to restore the default label. */
		setHiddenThinkingLabel(label?: string): void {
			send({ type: "ui_request", method: "setHiddenThinkingLabel", label });
		},

		/** Set a widget to display above or below the editor. */
		setWidget(key: string, content: unknown, options?: { placement?: "aboveEditor" | "belowEditor" }): void {
			const placement = options?.placement ?? "aboveEditor";
			const existingId = widgetComponentIds.get(key);
			if (existingId) {
				disposeComponent(existingId);
				widgetComponentIds.delete(key);
			}
			if (content === undefined) {
				send({ type: "ui_request", method: "setWidget", key, lines: [], placement });
				return;
			}
			if (typeof content !== "function") {
				send({ type: "ui_request", method: "setWidget", key, lines: content as string[], placement });
				return;
			}
			const id: { current?: string } = {};
			const component = (content as (tui: unknown, theme: ExtensionTheme) => Component)(
				tuiHandle(id),
				currentTheme,
			);
			const registered = registerComponent(component, extension);
			id.current = registered.id;
			widgetComponentIds.set(key, registered.id);
			send({ type: "ui_request", method: "setWidget", key, componentId: registered.id, placement });
		},

		/** Replace the built-in footer, or restore it by passing nothing. */
		setFooter(factory: unknown): void {
			if (footerComponentId) {
				disposeComponent(footerComponentId);
				footerComponentId = undefined;
			}
			if (factory === undefined) {
				send({ type: "ui_request", method: "setFooter", componentId: undefined });
				return;
			}
			const id: { current?: string } = {};
			const component = (factory as (tui: unknown, theme: ExtensionTheme, footerData: Json) => Component)(
				tuiHandle(id),
				currentTheme,
				{},
			);
			const registered = registerComponent(component, extension);
			id.current = registered.id;
			footerComponentId = registered.id;
			send({ type: "ui_request", method: "setFooter", componentId: registered.id });
		},

		/** Replace the built-in opening screen, or restore it by passing nothing. */
		setHeader(factory: unknown): void {
			if (headerComponentId) {
				disposeComponent(headerComponentId);
				headerComponentId = undefined;
			}
			if (factory === undefined) {
				send({ type: "ui_request", method: "setHeader", componentId: undefined });
				return;
			}
			const id: { current?: string } = {};
			const component = (factory as (tui: unknown, theme: ExtensionTheme) => Component)(tuiHandle(id), currentTheme);
			const registered = registerComponent(component, extension);
			id.current = registered.id;
			headerComponentId = registered.id;
			send({ type: "ui_request", method: "setHeader", componentId: registered.id });
		},

		setTitle(title: string): void {
			send({ type: "ui_request", method: "setTitle", title });
		},

		/** Show a component with keyboard focus, and wait for it to finish. */
		async custom(
			factory: (
				tui: unknown,
				theme: ExtensionTheme,
				keybindings: KeybindingsManager,
				done: (result: unknown) => void,
			) => Component | Promise<Component>,
		): Promise<unknown> {
			
			let finish: (result: unknown) => void = () => {};
			const idBox: { current?: string } = {};
			const component = await factory(tuiHandle(idBox), currentTheme, getKeybindings(), (result) => finish(result));
			const { id } = registerComponent(component, extension);
			idBox.current = id;

			return new Promise((resolve) => {
				let settled = false;
				finish = (result: unknown) => {
					
					if (settled) {
						return;
					}
					settled = true;
					disposeComponent(id);
					
					send({ type: "ui_request", method: "customDone", result });
					resolve(result);
				};

				void ask({ type: "ui_request", method: "custom", componentId: id }).then((answer) => {
					if (settled) {
						return;
					}
					settled = true;
					disposeComponent(id);
					resolve(answer.value);
				});
			});
		},

		pasteToEditor(text: string): void {
			echoedEditorText += text;
			send({ type: "ui_request", method: "pasteToEditor", text });
		},

		setEditorText(text: string): void {
			echoedEditorText = text;
			send({ type: "ui_request", method: "setEditorText", text });
		},

		/** The text this extension itself last put in the editor. */
		getEditorText(): string {
			return echoedEditorText;
		},

		/** Show a multi-line editor with keyboard focus, and wait for it to close. */
		async editor(title: string, prefill?: string): Promise<string | undefined> {
			const answer = await ask({ type: "ui_request", method: "editor", title, prefill });
			return answer.value as string | undefined;
		},

		/** Stack a completion source on top of whatever is already registered. */
		addAutocompleteProvider(factory: AutocompleteProviderFactory): void {
			autocompleteProvider = factory(autocompleteProvider ?? noAutocompleteSuggestions);
			for (const trigger of autocompleteProvider.triggerCharacters ?? []) {
				autocompleteTriggers.add(trigger);
			}
			send({ type: "action", action: "watch_autocomplete", triggers: [...autocompleteTriggers] });
		},

		/** Replace the built-in editor with a component, or restore it by passing nothing. */
		setEditorComponent(factory: EditorFactory | undefined): void {
			if (editorComponentId) {
				disposeComponent(editorComponentId);
				editorComponentId = undefined;
			}
			editorComponentFactory = factory;
			if (factory === undefined) {
				send({ type: "ui_request", method: "setEditorComponent", componentId: undefined });
				return;
			}
			const id: { current?: string } = {};
			const component = factory(tuiHandle(id), currentTheme, getKeybindings());
			const registered = registerComponent(component, extension);
			id.current = registered.id;
			editorComponentId = registered.id;
			send({ type: "ui_request", method: "setEditorComponent", componentId: registered.id });
		},

		/**
 * The factory last given to `setEditorComponent`, or `undefined` while the default editor is in
 * use.
 */
		getEditorComponent(): EditorFactory | undefined {
			return editorComponentFactory;
		},

		get theme(): ExtensionTheme {
			return currentTheme;
		},

		getAllThemes(): { name: string; path: string | undefined }[] {
			const themes: { name: string; path: string | undefined }[] = [
				{ name: "dark", path: undefined },
				{ name: "light", path: undefined },
			];
			const dir = themesDir();
			if (!existsSync(dir)) {
				return themes;
			}
			for (const entry of readdirSync(dir)) {
				if (entry.endsWith(".json")) {
					themes.push({ name: entry.slice(0, -".json".length), path: join(dir, entry) });
				}
			}
			return themes;
		},

		getTheme(name: string): ExtensionTheme | undefined {
			try {
				const resolved = resolveTheme(name);
				return resolved && new ExtensionTheme(resolved.name, resolved.colors);
			} catch {
				
				return undefined;
			}
		},

		setTheme(theme: string | ExtensionTheme): { success: boolean; error?: string } {
			try {
				const resolved = typeof theme === "string" ? resolveTheme(theme) : resolveTheme(theme.name);
				if (!resolved) {
					return { success: false, error: `no theme named ${typeof theme === "string" ? theme : theme.name}` };
				}
				currentTheme = new ExtensionTheme(resolved.name, resolved.colors);
				send({
					type: "ui_request",
					method: "setTheme",
					name: resolved.name,
					colors: Object.fromEntries(
						Object.entries(resolved.colors).map(([token, color]) => [
							token,
							"hex" in color ? color.hex : color.index,
						]),
					),
				});
				return { success: true };
			} catch (error) {
				return { success: false, error: error instanceof Error ? error.message : String(error) };
			}
		},

		/** The state this extension itself last set tool output expansion to. */
		getToolsExpanded(): boolean {
			return echoedToolsExpanded;
		},

		setToolsExpanded(expanded: boolean): void {
			echoedToolsExpanded = expanded;
			send({ type: "ui_request", method: "setToolsExpanded", expanded });
		},
	};
}
