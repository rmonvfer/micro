// Running what an extension registered.
//
// A tool the model called, a command the user typed, a renderer asked for lines: each is a
// function an extension handed over, and each answer has to be turned back into something
// micro can act on.

import { type Json } from "./host-wire.ts";

/** Whatever a renderer returned, as the lines micro will draw. */
export function renderedLines(value: unknown): string[] {
	if (typeof value === "string") {
		return value.split("\n");
	}
	if (Array.isArray(value)) {
		return value.map((line) => String(line));
	}
	return [];
}

/**
 * A tool may answer with a string, or with a shape carrying output and details.
 *
 * What micro needs is the text; anything else an extension attached is carried alongside so
 * a renderer it registered can still reach it.
 */
export function toolAnswer(value: unknown): Json {
	if (typeof value === "string") {
		return { output: value };
	}
	if (value && typeof value === "object") {
		const shape = value as Json;
		const output = shape.output ?? shape.text ?? shape.content;
		return {
			output: typeof output === "string" ? output : JSON.stringify(value),
			details: shape.details,
			isError: shape.isError === true,
		};
	}
	return { output: value === undefined ? "" : String(value) };
}
