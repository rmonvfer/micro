// What `@earendil-works/pi-tui` and `@mariozechner/pi-tui` resolve to for a pi extension
// running under micro.
//
// pi-tui is two different things wearing one package name: a handful of pure functions
// and small render-to-string-lines components with no dependency on how a terminal is
// actually driven (`visibleWidth`, `wrapTextWithAnsi`, `matchesKey`, `Box`, `Text`,
// `Input`, `Editor`, `getKeybindings`, `HStack`/`VStack`/`ScrollView`/`SelectList`, the
// autocomplete provider contract, `renderLatex`, ...), and the terminal driver itself —
// raw mode, differential rendering, focus, overlays, image protocols. The first kind is
// vendored here unchanged (see `./utils.ts`, `./keys.ts`, `./keybindings.ts`, `./fuzzy.ts`,
// `./kill-ring.ts`, `./undo-stack.ts`, `./word-navigation.ts`, `./layout-node.ts`,
// `./autocomplete.ts`, `./latex.ts`, `./components/`) because nothing about it depends on
// which process owns the terminal. The second kind cannot be: micro's own Rust TUI already
// owns the terminal this process is attached to, and running a second one from inside the
// extension host would not draw anywhere a reader could see it, only race the one that
// does. Reaching for it is answered with exactly that, once, at the point of use — not
// with a working-looking object that quietly draws nothing.
//
// `AutocompleteItem`/`AutocompleteProvider`/`AutocompleteSuggestions` are the types an
// extension implementing `ctx.ui.addAutocompleteProvider` writes its provider against —
// `getSuggestions(lines, cursorLine, cursorCol, {signal, force})` and the required
// `applyCompletion(...)` are `./autocomplete.ts`'s own contract, unchanged, so an
// extension type-checking against these and calling through `ctx.ui` sees one contract,
// not two that happen to almost agree.
import { Marked } from "marked";
import { CombinedAutocompleteProvider } from "./autocomplete.ts";
import { Box } from "./components/box.ts";
import { CancellableLoader } from "./components/cancellable-loader.ts";
import { Editor } from "./components/editor.ts";
import { HStack } from "./components/h-stack.ts";
import { Input } from "./components/input.ts";
import { Loader } from "./components/loader.ts";
import { Markdown } from "./components/markdown.ts";
import { ScrollView } from "./components/scroll-view.ts";
import { SelectList } from "./components/select-list.ts";
import { SettingsList } from "./components/settings-list.ts";
import { Spacer } from "./components/spacer.ts";
import { Text } from "./components/text.ts";
import { TruncatedText } from "./components/truncated-text.ts";
import { VStack } from "./components/v-stack.ts";
import { fuzzyFilter, fuzzyMatch } from "./fuzzy.ts";
import {
	getKeybindings,
	KeybindingsManager,
	setKeybindings,
	TUI_KEYBINDINGS,
} from "./keybindings.ts";
import {
	decodeKittyPrintable,
	isKeyRelease,
	isKeyRepeat,
	isKittyProtocolActive,
	Key,
	matchesKey,
	parseKey,
	setKittyProtocolActive,
} from "./keys.ts";
import { renderLatex } from "./latex.ts";
import { getCapabilities, hyperlink } from "./terminal-image.ts";
import { CURSOR_MARKER, Container, compositeTuiLine, isFocusable } from "./tui.ts";
import {
	getOsc8LinkAtColumn,
	sliceByColumn,
	stripTerminalSequences,
	truncateToWidth,
	visibleWidth,
	wrapTextWithAnsi,
} from "./utils.ts";

export {
	Box,
	CancellableLoader,
	CombinedAutocompleteProvider,
	compositeTuiLine,
	Container,
	CURSOR_MARKER,
	decodeKittyPrintable,
	Editor,
	fuzzyFilter,
	fuzzyMatch,
	getCapabilities,
	getKeybindings,
	getOsc8LinkAtColumn,
	HStack,
	hyperlink,
	Input,
	isFocusable,
	isKeyRelease,
	isKeyRepeat,
	isKittyProtocolActive,
	Key,
	KeybindingsManager,
	Loader,
	Marked,
	Markdown,
	matchesKey,
	parseKey,
	renderLatex,
	ScrollView,
	SelectList,
	setKeybindings,
	SettingsList,
	setKittyProtocolActive,
	sliceByColumn,
	Spacer,
	stripTerminalSequences,
	Text,
	truncateToWidth,
	TruncatedText,
	TUI_KEYBINDINGS,
	visibleWidth,
	VStack,
	wrapTextWithAnsi,
};

export type { Token, Tokens } from "marked";

export type { AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, SlashCommand } from "./autocomplete.ts";
export type { StackChild, StackEntry, StackEntryOptions, StackOptions } from "./components/stack.ts";

/** What reaching for an unsupported export is told, precisely and once, at the point it
 * is actually used rather than at import time — an import naming it is not yet a mistake,
 * since most of what pi-tui exports is erased TypeScript types by the time this runs. */
function unsupported(name: string): (..._args: unknown[]) => never {
	return function unsupportedPiTuiExport(): never {
		throw new Error(
			`pi-tui's ${name} needs a terminal of its own to draw into, which micro's compatibility layer for pi extensions does not give it — micro's own Rust TUI already owns the terminal this process is attached to. Text-only helpers (visibleWidth, wrapTextWithAnsi, truncateToWidth, matchesKey, Key, fuzzyFilter, Box, Container, Spacer, Text, Input, getKeybindings, ...) are real; a live terminal-driving component is not.`,
		);
	};
}

const STUB_NAMES = [
	"Image",
	"StdinBuffer",
	"ProcessTerminal",
	"parseOsc11BackgroundColor",
	"parseTerminalColorSchemeReport",
	"allocateImageId",
	"calculateImageRows",
	"deleteAllKittyImages",
	"deleteKittyImage",
	"detectCapabilities",
	"encodeITerm2",
	"encodeKitty",
	"getCellDimensions",
	"getGifDimensions",
	"getImageDimensions",
	"getJpegDimensions",
	"getPngDimensions",
	"getWebpDimensions",
	"imageFallback",
	"renderImage",
	"resetCapabilitiesCache",
	"setCapabilities",
	"setCellDimensions",
	"isViewportTUI",
	"TuiAltScreen",
	"TuiMainScreen",
] as const;

const stubs = Object.fromEntries(STUB_NAMES.map((name) => [name, unsupported(name)])) as Record<
	(typeof STUB_NAMES)[number],
	(..._args: unknown[]) => never
>;

export const {
	Image,
	StdinBuffer,
	ProcessTerminal,
	parseOsc11BackgroundColor,
	parseTerminalColorSchemeReport,
	allocateImageId,
	calculateImageRows,
	deleteAllKittyImages,
	deleteKittyImage,
	detectCapabilities,
	encodeITerm2,
	encodeKitty,
	getCellDimensions,
	getGifDimensions,
	getImageDimensions,
	getJpegDimensions,
	getPngDimensions,
	getWebpDimensions,
	imageFallback,
	renderImage,
	resetCapabilitiesCache,
	setCapabilities,
	setCellDimensions,
	isViewportTUI,
	TuiAltScreen,
	TuiMainScreen,
} = stubs;
