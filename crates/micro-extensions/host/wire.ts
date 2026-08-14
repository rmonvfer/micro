// Talking to micro.
//
// The host and micro pass one JSON object per line over stdio. Everything an extension asks
// for that micro has to answer goes through here, so the rest of the host never touches the
// wire directly and a request looks the same wherever it was made from.

export type Json = Record<string, unknown>;

/** Requests to micro that are waiting for an answer, by id. */
const waiting = new Map<string, (value: Json) => void>();
let nextRequestId = 0;

export function send(message: Json): void {
	process.stdout.write(`${JSON.stringify(message)}\n`);
}

/** Ask micro for something and wait for the answer. */
export function ask(request: Json): Promise<Json> {
	const id = `host-${nextRequestId++}`;
	return new Promise((resolve) => {
		waiting.set(id, resolve);
		send({ ...request, id });
	});
}

/** Hand micro's answer to whoever asked for it. */
export function answered(id: string, value: Json): boolean {
	const resolve = waiting.get(id);
	if (!resolve) {
		return false;
	}
	waiting.delete(id);
	resolve(value);
	return true;
}

// A compat shim (`crates/micro-extensions/src/compat.rs`) lives in its own
// `node_modules` tree, reached through `NODE_PATH` rather than a relative import from this
// file — that is what lets it resolve for an extension wherever the extension itself lives,
// but it also means the shim cannot `import` this module the ordinary way: nothing on its
// own resolution path leads back here. `globalThis` is the one thing both sides already
// share, being the same process, so `ask` and `send` are published on it here, once, before
// anything dynamically imports an extension — a facade like a `SessionManager` stand-in
// asks micro for something the same way every other part of the host already does.
declare global {
	// eslint-disable-next-line no-var
	var __MICRO_WIRE__: { ask: typeof ask; send: typeof send } | undefined;
}
globalThis.__MICRO_WIRE__ = { ask, send };
