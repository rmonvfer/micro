// What an extension may do to the interface.
//
// Everything here is a request: the interface belongs to micro, and an extension asks it to
// show something rather than drawing anything itself. That is what keeps a third-party file
// inside the same policy as the rest of the interface.
//
// A handful of members answer synchronously in pi, because pi's interface lives in the same
// process as the extension and reading it costs nothing more than a property access. This
// process is not that one: the interface is a Rust program on the other end of a pipe, and
// nothing sent up it can come back before the function that asked has already returned. Two
// different answers to that, depending on what is being read:
//
// - A theme is a file on disk (the `themes` directory of micro's configuration directory,
//   the same place micro itself reads them from) or one of the two built in. Reading it
//   synchronously here means reading it the
//   same way micro does rather than asking micro to, which keeps `getTheme`, `setTheme` and
//   `getAllThemes` the synchronous functions pi declares them as.
// - The editor's text and whether tool output is expanded are not files anywhere; they are
//   live state inside micro's TUI. There is no synchronous way to read live state through a
//   pipe, so `getEditorText` and `getToolsExpanded` answer from a local echo of the last
//   value this extension itself set instead — accurate about what this extension asked for,
//   silent about a keystroke or a shortcut that changed it since. RPC mode, pi's own
//   out-of-process front end, resolves the same conflict the same way: a synchronous getter
//   with nothing behind a pipe to read yet, answering from what little it can know locally
//   rather than refusing to exist.
//
// A live component (a header, a footer, a widget, the editor, the result of `custom`) looks
// like a third kind of thing this process cannot answer honestly — pi's `Component` is an
// object with methods that draw and react to input frame by frame, which sounds like exactly
// the kind of thing that cannot cross a pipe. It does not have to cross it. The object stays
// here, in this process, for as long as it is wanted; micro holds only an id and calls it by
// name, the same way `registerTool`'s `execute` already runs here and is called from there.
// `./host-components.ts` is the registry that id is looked up in and the four operations
// micro drives one through — see it for the `Component` shape and for why `handleInput`
// answers whether it consumed a key rather than pi's plain `void`. `pushChanged` there is
// this side's own initiative: a timer, something arriving asynchronously, telling micro its
// lines are stale without waiting to be asked.
//
// `setWidget`'s component-factory overload, `setHeader`, `setFooter`, `custom`, and
// `setEditorComponent` all register through it. `addAutocompleteProvider` is a different
// shape again: `AutocompleteProvider` is pi-tui's own interface, not `Component`, and it is
// asked rather than driven — `dispatchSuggestions` for a completion list,
// `dispatchApplyCompletion` for what committing one of its items writes — since a menu of
// suggestions is a question with an answer, twice over, rather than something with a
// lifetime of its own.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join } from "node:path";
// pi-tui's own keybindings — vendored by the compat layer into this same process's
// `node_modules`, resolved the ordinary way rather than reached for by a relative path into
// where that layer happens to keep its files. `getKeybindings()` is pi-tui's own current
// table, the same one a factory receives in pi itself, so `keybindings.matches(data, name)`
// answers a `CustomEditor`'s own `super.handleInput` checks correctly instead of throwing.
import { getKeybindings, type KeybindingsManager } from "@earendil-works/pi-tui";
import { type Component, dispose as disposeComponent, pushChanged, registerComponent } from "./host-components.ts";
import { type Json, wireFor } from "./host-wire.ts";

/** A handler `ctx.ui.onTerminalInput` registered, and what it asked to do with a key. */
type TerminalInputHandler = (data: string) => { consume?: boolean; data?: string } | undefined;

/**
 * Extensions currently listening for raw terminal input, in the order they registered.
 *
 * Module-level rather than carried on the object `uiFor` returns: a fresh object is built
 * for every tool call, every command, every event, but a subscription made during one of
 * those has to outlive it, so it is kept where every call finds the same set.
 */
const terminalInputHandlers = new Set<TerminalInputHandler>();

/**
 * Hand a key micro read to every registered handler, in registration order, until one
 * consumes it. Called from `host.ts` when micro asks the `terminal_input` event — not
 * dispatched through the ordinary event bus, because a handler here is registered through
 * `ctx.ui`, not through `micro.on`.
 */
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

/** Told, not asked: micro pushes the built-in editor's real text along with every key it
 * sends a `setEditorComponent` replacement, since a key the component does not consume falls
 * straight through to that editor and the component — and `getEditorText` — would otherwise
 * have no way to know what it now says. Called from `host.ts` when a `component` "input"
 * message carries one. This is what keeps `getEditorText` a synchronous read, the way pi's
 * own is, rather than turning it into a round trip: pi's `CustomEditor` sees the same thing
 * for free by inheriting the base editor's buffer, and this is this process's way of getting
 * the same answer without inheriting anything. */
export function noteEditorText(text: string): void {
	echoedEditorText = text;
}

/** The last state this extension itself set tool output expansion to. */
let echoedToolsExpanded = false;

// ============================================================================
// Themes
// ============================================================================

/** A color as pi's theme schema carries it: a hex string, a var name, or a 256-color index. */
type ColorValue = string | number;

/** One token's resolved color, in the two forms `fg`/`bg` know how to wrap: a hex string, or
 * a 256-color index. Kept apart from `ColorValue` because a var name is not a color yet. */
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

/** Every token pi's theme schema requires, transcribed from `theme/palette.rs`'s
 * `TOKENS` — the same 48 names both built-in themes and every valid user theme carry. */
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

/** Every color both of micro's built-in themes carry, transcribed from
 * `crates/micro-tui/src/theme/palette.rs`'s `DARK_COLORS`/`LIGHT_COLORS`. Read the same way
 * `getTheme`/`getAllThemes` read a user's own theme file, so the two paths answer alike;
 * kept as a literal table because, unlike a user theme, nothing on disk names a built-in. */
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

/** Where a user's own themes live: the `themes` directory of micro's configuration
 * directory, the same place `crates/micro-tui/src/theme/custom.rs`'s `themes_dir` reads. */
function themesDir(): string {
	return join(configDir(), "themes");
}

/** micro's configuration directory, resolved the way `crates/micro-dirs` resolves it:
 * `MICRO_DIR` holds everything when it names a directory, an existing `~/.micro` holds
 * everything when it is there, and otherwise what the user wrote lives under
 * `XDG_CONFIG_HOME`. A relative XDG path is not a base directory, so it is read as unset. */
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

/** A theme file's `colors` block, every token resolved through its `vars` block. Throws the
 * way `crates/micro-tui/src/theme/custom.rs`'s `parse` does: by naming exactly what is
 * wrong, so a broken theme is a message rather than a silent fallback. */
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

/** A built-in theme's colors, already resolved — nothing in the literal table above is a
 * var reference, so this never throws the way [`readThemeFile`] can. */
function builtInColors(name: string): Record<string, Resolved> {
	const colors: Record<string, Resolved> = {};
	for (const [token, hex] of Object.entries(BUILT_IN_THEMES[name])) {
		colors[token] = { hex };
	}
	return colors;
}

/** A theme by name: one of the two built in, or a file in the themes directory. `undefined`
 * for a name that is neither, the same way `getThemeByName` answers in pi. */
function resolveTheme(name: string): { name: string; colors: Record<string, Resolved> } | undefined {
	if (name in BUILT_IN_THEMES) {
		return { name, colors: builtInColors(name) };
	}
	// A name is a file stem, so anything that could climb out of the themes directory —
	// a slash, a `..` segment — is refused rather than resolved.
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

/** A theme as `ctx.ui.theme`, `getTheme` and `setTheme` hand it to an extension: named, and
 * carrying the two methods pi's `Theme` class wraps a color in. Each wraps the ANSI escape
 * a truecolor terminal reads, so a widget an extension colors with it looks the way it
 * would coming out of pi itself. */
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

	// The five styles a theme carries beyond colour. Each closes with the code that turns
	// only itself off, rather than a blanket reset, so styling something inside already
	// styled text leaves what surrounded it still in force — the same reason `fg` resets
	// only the foreground.
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

/**
 * The theme `ctx.ui.theme` currently answers with.
 *
 * Starts dark, which is what a session with nothing said about its theme yet opens in.
 * Replaced the moment this extension's own `setTheme` resolves one, since resolving is
 * what deciding to switch already required. A theme switched by the reader or by another
 * extension is not reflected here — nothing today tells this process when that happens —
 * so this is a remembered choice rather than a live mirror of what is on screen.
 */
let currentTheme = new ExtensionTheme("dark", builtInColors("dark"));

// ============================================================================
// Components
// ============================================================================

/** A stand-in for pi-tui's `TUI`, carrying the one thing a factory reaches for that this
 * process can actually do something with: asking for a repaint once a component changes on
 * its own schedule — a timer, something arriving asynchronously — rather than in answer to
 * a keystroke, which micro already re-renders after on its own. Everything else pi-tui's
 * real `TUI` offers (focus, overlays, raw input) belongs to a rendering process, and is left
 * off rather than faked.
 *
 * Takes a mutable box for the component's id rather than the id itself: the factory has to
 * be called to produce the component before `registerComponent` can hand back an id for it,
 * but the `tui` argument the factory reads has to exist before that call is made. The box is
 * what lets the same closure work before and after the id it needs is known. */
function tuiHandle(id: { current?: string }): { requestRender(): void } {
	return {
		requestRender(): void {
			if (id.current) {
				pushChanged(id.current);
			}
		},
	};
}

/** Live widgets set through `setWidget`'s factory overload, by the key they were set under —
 * a widget is addressed by its key everywhere else in the interface, so its component is
 * retired the same way rather than under a second id nothing else would recognise. */
const widgetComponentIds = new Map<string, string>();

let headerComponentId: string | undefined;
let footerComponentId: string | undefined;

/** The factory `setEditorComponent` was last given, and the id its component is registered
 * under while it is active. `getEditorComponent` hands back the factory alone, matching
 * pi's own signature; the id is this file's own bookkeeping for tearing the old component
 * down when a new one replaces it. */
let editorComponentFactory: EditorFactory | undefined;
let editorComponentId: string | undefined;

type EditorFactory = (tui: unknown, theme: ExtensionTheme, keybindings: KeybindingsManager) => Component;

/** A handler `ctx.ui.addAutocompleteProvider` registered, wrapping whatever provider came
 * before it — the same chain pi builds, outermost registration first. */
type AutocompleteProviderFactory = (current: AutocompleteProvider) => AutocompleteProvider;

/** Shaped exactly as pi-tui's own `AutocompleteItem` — pi's `ExtensionUIContext` imports the
 * type straight from `@earendil-works/pi-tui` rather than declaring a narrower one of its
 * own, so an extension written against pi's real type has to fit this, not a simplified
 * stand-in. `value` is what committing writes; `label` is what is shown, which is free to
 * differ from it — a `@user` item might show "Jordan (@user)" and write "@user". */
export interface AutocompleteItem {
	value: string;
	label: string;
	description?: string;
}

interface AutocompleteSuggestions {
	items: AutocompleteItem[];
	prefix: string;
}

/** Also shaped after pi-tui's real interface. `applyCompletion` is not optional there —
 * every provider in the chain has to answer it, if only by delegating to the one it wrapped
 * — so it is not optional here either, and micro asks it for real: see
 * `dispatchApplyCompletion`. `signal` is a genuine, working `AbortSignal`; nothing on
 * micro's side ever calls `.abort()` on it yet, since there is no cancel-in-flight message
 * on the wire today, but the object handed to a provider is not a decoy — reading
 * `signal.aborted` behaves exactly as it would in pi. */
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

/** The built-in splice every provider in the chain can fall back to by delegating to the one
 * it wrapped: replace `prefix` — the triggered word `getSuggestions` was asked about — with
 * the item's value and a trailing space, on the line the cursor sits on. The same edit
 * `Menu::commit` already writes for the built-in `/` and `@` menus, so a provider that never
 * overrides `applyCompletion` behaves exactly like one of those. */
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

/** The built-in provider an extension's first registration wraps: nothing of its own to
 * suggest, since micro's own file and command completion runs entirely on the Rust side and
 * has no reason to ask this process about it. Wrapping it is still worth doing — it is what
 * lets a chain of several `addAutocompleteProvider` calls each wrap the one before it, the
 * way pi's own chain does, and what a provider that never overrides `applyCompletion` falls
 * back to. */
const noAutocompleteSuggestions: AutocompleteProvider = {
	getSuggestions: async () => null,
	applyCompletion: spliceCompletion,
};

let autocompleteProvider: AutocompleteProvider | undefined;

/** Every trigger character any provider in the chain has registered, accumulated the same
 * way pi's own `setupAutocompleteProvider` does: a provider does not have to repeat what the
 * one it wrapped already declared for its own `triggerCharacters` to still open a menu. */
const autocompleteTriggers = new Set<string>();

/** Ask whatever `addAutocompleteProvider` chain is registered for suggestions at the
 * cursor. Called from `host.ts` when micro asks the `get_suggestions` event. `undefined`
 * when nothing is registered or the chain found nothing to add, so micro can tell "nothing
 * to add" apart from "asked and got nothing back". */
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

/** Turn the item micro says the reader picked into the edit committing it makes, through
 * whichever `applyCompletion` the provider chain settled on — the built-in splice unless an
 * extension overrode it. Called from `host.ts` when micro asks the `apply_completion` event. */
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

/** The interface as an extension sees it.
 *
 * Built per extension rather than once, so every request below carries the path of whoever
 * made it: micro decides what an ask is allowed to do by looking at who asked, and one
 * shared interface object would leave every one of them anonymous. The pair taken here
 * shadows the module's own `ask`/`send` for the whole of this function, which is why
 * nothing inside it has to mention the extension itself. */
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
			// The first listener is what makes it worth micro's while to offer every key
			// here at all: asking about a key nobody is listening for would cost a round
			// trip to this process for nothing, once per keystroke, for the rest of the
			// session — so micro is told only when there is a first, and again when there
			// is a last.
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
				// A call with no argument at all is told apart from one that gave an empty
				// `frames` array — the first restores the default spinner, the second hides
				// the indicator outright — since both would otherwise send the same options.
				reset: options === undefined,
				frames: options?.frames ?? [],
				intervalMs: options?.intervalMs,
			});
		},

		/** Call with nothing to restore the default label. */
		setHiddenThinkingLabel(label?: string): void {
			send({ type: "ui_request", method: "setHiddenThinkingLabel", label });
		},

		/** Set a widget to display above or below the editor. Accepts a string array or a
		 * component factory; both reach the screen — see the note at the top of this file
		 * for how a factory does it without ever leaving this process. */
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

		/** Replace the built-in footer, or restore it by passing nothing. pi hands the
		 * factory a `footerData` provider carrying the git branch and extension statuses;
		 * nothing here tracks those apart from what micro already draws in its own footer,
		 * so a factory reading it sees an empty one rather than micro's state duplicated
		 * through a second path. */
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

		/** Show a component with keyboard focus, and wait for it to finish.
		 *
		 * The factory's `done` callback is what finishes it: calling `done(result)` is what
		 * resolves the promise this returns, whether that happens because the component
		 * decided it was finished or because a handler closed over `done` from outside one.
		 * Closing the overlay any other way — the reader pressing escape, micro's own
		 * `signal` firing — resolves it with `undefined` instead, the same as `done`
		 * receiving nothing. */
		async custom(
			factory: (
				tui: unknown,
				theme: ExtensionTheme,
				keybindings: KeybindingsManager,
				done: (result: unknown) => void,
			) => Component | Promise<Component>,
		): Promise<unknown> {
			// `done` has to exist before the factory is called, since the factory is what
			// gets a reference to it — but it cannot finish anything until `finish` below
			// exists to call, which needs the id `registerComponent` has not handed back
			// yet. The indirection is what breaks that circle: `done` always calls whatever
			// `finish` currently is, and `finish` starts as a no-op.
			let finish: (result: unknown) => void = () => {};
			const idBox: { current?: string } = {};
			const component = await factory(tuiHandle(idBox), currentTheme, getKeybindings(), (result) => finish(result));
			const { id } = registerComponent(component, extension);
			idBox.current = id;

			return new Promise((resolve) => {
				let settled = false;
				finish = (result: unknown) => {
					// Finished through the other path below already — the reader closed the
					// overlay before the component called `done` itself.
					if (settled) {
						return;
					}
					settled = true;
					disposeComponent(id);
					// Told rather than waited for: this side has already decided the answer,
					// and there is nothing to gain from a round trip before acting on it.
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

		/** The text this extension itself last put in the editor. See the note at the top
		 * of this file: nothing keystroked by the reader reaches this echo. */
		getEditorText(): string {
			return echoedEditorText;
		},

		/** Show a multi-line editor with keyboard focus, and wait for it to close. Resolves
		 * with what was typed on submit, `undefined` if the reader backs out instead — the
		 * same shape `select`/`input` answer with, since this is the same kind of question,
		 * only over more than one line. */
		async editor(title: string, prefill?: string): Promise<string | undefined> {
			const answer = await ask({ type: "ui_request", method: "editor", title, prefill });
			return answer.value as string | undefined;
		},

		/** Stack a completion source on top of whatever is already registered — not on `@`,
		 * which is micro's own and never asks this process about it, but on whatever trigger
		 * characters this provider and the ones before it declared. Asked for suggestions
		 * whenever a word starting with one of them changes; see the module doc for why that
		 * ask never holds up a keystroke. */
		addAutocompleteProvider(factory: AutocompleteProviderFactory): void {
			autocompleteProvider = factory(autocompleteProvider ?? noAutocompleteSuggestions);
			for (const trigger of autocompleteProvider.triggerCharacters ?? []) {
				autocompleteTriggers.add(trigger);
			}
			send({ type: "action", action: "watch_autocomplete", triggers: [...autocompleteTriggers] });
		},

		/** Replace the built-in editor with a component, or restore it by passing nothing.
		 * Every keystroke that would otherwise reach the built-in editor is handed to the
		 * component's `handleInput` instead, the same full capture an overlay already gets;
		 * a `CURSOR_MARKER` the component emits in its render output places the hardware
		 * cursor, exactly as pi's own does. */
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

		/** The factory last given to `setEditorComponent`, or `undefined` while the default
		 * editor is in use — pi's own signature hands back the factory itself, not a live
		 * component, so there is nothing here that needs to leave this process either. */
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
				// A theme file that will not parse is not there as far as a reader asking
				// for it by name is concerned — the same answer a missing file gets.
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

		/** The state this extension itself last set tool output expansion to. See the note
		 * at the top of this file: a reader's own shortcut does not reach this echo. */
		getToolsExpanded(): boolean {
			return echoedToolsExpanded;
		},

		setToolsExpanded(expanded: boolean): void {
			echoedToolsExpanded = expanded;
			send({ type: "ui_request", method: "setToolsExpanded", expanded });
		},
	};
}
