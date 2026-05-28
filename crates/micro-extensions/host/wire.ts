

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

/** The same two calls, tagging everything they send with the extension that made it. */
export function wireFor(extension: string): { ask: typeof ask; send: typeof send } {
	return {
		ask: (request: Json) => ask({ ...request, extension }),
		send: (message: Json) => send({ ...message, extension }),
	};
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


declare global {
	// eslint-disable-next-line no-var
	var __MICRO_WIRE__: { ask: typeof ask; send: typeof send } | undefined;
}
globalThis.__MICRO_WIRE__ = { ask, send };
