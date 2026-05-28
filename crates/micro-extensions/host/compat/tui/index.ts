
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


function unsupported(name: string): (..._args: unknown[]) => never {
	return function unsupportedPiTuiExport(): never {
		throw new Error(
			`pi-tui export ${name} is unavailable because micro owns the terminal`,
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
