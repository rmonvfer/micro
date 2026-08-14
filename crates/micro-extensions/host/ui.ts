// What an extension may do to the interface.
//
// Everything here is a request: the interface belongs to micro, and an extension asks it to
// show something rather than drawing anything itself. That is what keeps a third-party file
// inside the same policy as the rest of the interface.

import { ask, type Json, send } from "./host-wire.ts";

/** The interface as an extension sees it. */
export function uiFor(): Json {
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

		setStatus(statusKey: string, statusText?: string): void {
			send({ type: "ui_request", method: "setStatus", statusKey, statusText });
		},
	};
}
