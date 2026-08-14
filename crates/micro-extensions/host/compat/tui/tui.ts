// A stand-in for pi-tui's own tui.ts, carrying only what the rest of this compat layer
// actually needs from it: the `Component`/`Focusable` interfaces components render
// against, `Container`, which composes them, and `compositeTuiLine`, the pure line-
// blending function `HStack` (and any future overlay-drawing code) calls. What is left
// out is the real one's terminal driver — raw mode, cursor placement, focus, overlays —
// and there is no terminal for it to drive here: micro owns the one the process is
// attached to, the same reason `../east-asian-width` exists rather than an extension
// reaching past this layer for its own.
//
// Everything below is transcribed unchanged from pi-tui's `tui.ts`, with one deliberate
// substitution: `compositeTuiLine`'s real first line asks `isImageLine` whether the line
// it is compositing onto already carries a Kitty/iTerm2 image escape sequence, so it can
// leave that line untouched. Nothing produced anywhere in this compat layer ever emits
// one — the image protocol is one of the pieces genuinely left as a terminal-driving
// stub — so answering `false` unconditionally is the correct answer for every line this
// layer ever sees, not an approximation of the real check.

import { extractSegments, sliceByColumn, sliceWithWidth, visibleWidth } from "./utils.ts";

export interface Component {
	render(width: number): string[];
	handleInput?(data: string): void;
	wantsKeyRelease?: boolean;
	invalidate(): void;
}

export interface Focusable {
	focused: boolean;
}

export function isFocusable(component: Component | null): component is Component & Focusable {
	return component !== null && "focused" in component;
}

export const CURSOR_MARKER = "\x1b_pi:c\x07";

const SEGMENT_RESET = "\x1b[0m\x1b]8;;\x07";

/** See the file header: this compat layer never produces an image-protocol escape
 * sequence, so "no line here is an image line" is always the correct answer. */
function isImageLine(_line: string): boolean {
	return false;
}

/** Composite overlay content into a terminal line at a fixed column. Ported unchanged
 * from pi-tui's own `tui.ts`, aside from `isImageLine` above. */
export function compositeTuiLine(
	baseLine: string,
	overlayLine: string,
	startCol: number,
	overlayWidth: number,
	totalWidth: number,
): string {
	if (isImageLine(baseLine)) return baseLine;

	const afterStart = startCol + overlayWidth;
	const base = extractSegments(baseLine, startCol, afterStart, totalWidth - afterStart, true);
	const overlay = sliceWithWidth(overlayLine, 0, overlayWidth, true);
	const beforePad = Math.max(0, startCol - base.beforeWidth);
	const overlayPad = Math.max(0, overlayWidth - overlay.width);
	const actualBeforeWidth = Math.max(startCol, base.beforeWidth);
	const actualOverlayWidth = Math.max(overlayWidth, overlay.width);
	const afterTarget = Math.max(0, totalWidth - actualBeforeWidth - actualOverlayWidth);
	const afterPad = Math.max(0, afterTarget - base.afterWidth);
	const result =
		base.before +
		" ".repeat(beforePad) +
		SEGMENT_RESET +
		overlay.text +
		" ".repeat(overlayPad) +
		SEGMENT_RESET +
		base.after +
		" ".repeat(afterPad);

	return visibleWidth(result) <= totalWidth ? result : sliceByColumn(result, 0, totalWidth, true);
}

export class Container implements Component {
	children: Component[] = [];

	addChild(component: Component): void {
		this.children.push(component);
	}

	removeChild(component: Component): void {
		const index = this.children.indexOf(component);
		if (index !== -1) {
			this.children.splice(index, 1);
		}
	}

	clear(): void {
		this.children = [];
	}

	invalidate(): void {
		for (const child of this.children) {
			child.invalidate?.();
		}
	}

	render(width: number): string[] {
		const lines: string[] = [];
		for (const child of this.children) {
			const childLines = child.render(width);
			for (const line of childLines) {
				lines.push(line);
			}
		}
		return lines;
	}
}
