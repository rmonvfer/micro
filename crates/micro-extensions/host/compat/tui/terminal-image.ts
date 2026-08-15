

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
