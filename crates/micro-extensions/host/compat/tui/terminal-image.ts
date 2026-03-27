// A stand-in for pi-tui's own terminal-image.ts, carrying only the three exports
// `Markdown` actually reaches for: `isImageLine`, `hyperlink`, and `getCapabilities().
// hyperlinks`. The real file is several hundred lines of Kitty/iTerm2 graphics-protocol
// encoding and terminal-capability probing — a live back-and-forth with the terminal this
// process is not attached to (micro's own Rust TUI is). None of that machinery is needed
// for what `Markdown` actually asks of it:
//
// - `isImageLine` answers whether a line already carries a rendered image, so a reflow
//   pass can leave it alone. This compat layer never produces one — same reasoning as
//   `../tui.ts`'s own `isImageLine` — so `false` is the correct answer, not a placeholder.
// - `hyperlink` wraps text in an OSC 8 hyperlink escape sequence. That is pure string
//   formatting with no dependency on what the terminal supports; ported unchanged.
// - `getCapabilities()` reports what this layer can honestly claim: no image protocol
//   (`images: null`, the real type's own "none of these" value), and true color plus OSC 8
//   hyperlinks, both widely-supported enough, and harmless enough where they are not, that
//   pi's own real detection defaults to them too when a terminal cannot be queried.

export type ImageProtocol = "kitty" | "iterm2" | null;

export interface TerminalCapabilities {
	images: ImageProtocol;
	trueColor: boolean;
	hyperlinks: boolean;
}

export function isImageLine(_line: string): boolean {
	return false;
}

/** Ported unchanged from terminal-image.ts's own `hyperlink`. */
export function hyperlink(text: string, url: string): string {
	return `\x1b]8;;${url}\x1b\\${text}\x1b]8;;\x1b\\`;
}

const CAPABILITIES: TerminalCapabilities = {
	images: null,
	trueColor: true,
	hyperlinks: true,
};

export function getCapabilities(): TerminalCapabilities {
	return CAPABILITIES;
}
