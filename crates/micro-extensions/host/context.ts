// What a handler, tool or command is called with.
//
// pi hands an extension a context describing the session it is running in: where it is, what
// model is answering, what has been said, and the handful of things an extension may do to
// the run itself. micro answers most of these by asking, since the session lives on the
// other side of the wire.

import { ask, type Json, send } from "./host-wire.ts";

/** Where this host is running, told once when the extensions are loaded. */
export const where = { cwd: process.cwd(), hasUI: false, mode: "tui" as string };

/** Fill in what micro says about this run. */
export function located(message: Json): void {
	where.cwd = (message.cwd as string) ?? process.cwd();
	where.hasUI = message.has_ui === true;
	where.mode = (message.mode as string) ?? "tui";
}

/**
 * The context every tool, command and handler is called with.
 *
 * Built fresh per call rather than kept, because most of what it answers is a question for
 * micro and the answer changes between one call and the next.
 */
export function contextFor(ui: Json): Json {
	return {
		cwd: where.cwd,
		hasUI: where.hasUI,
		mode: where.mode,
		ui,
	};
}
