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
