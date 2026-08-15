

import { ask, type Json, wireFor } from "./host-wire.ts";

/** Whether a run is in flight, and the controller whose signal is handed out for its duration. */
const turn = {
	idle: true,
	controller: undefined as AbortController | undefined,
	waiters: [] as Array<() => void>,
};

/** Resolvers waiting for the next `session_start` with a given `reason`. */
const sessionStarted = new Map<string, Array<() => void>>();

/** Track the run's own state from the lifecycle events micro forwards. */
export function noted(event: string, payload: Json): void {
	if (event === "agent_start") {
		turn.idle = false;
		turn.controller = new AbortController();
	} else if (event === "agent_settled") {
		turn.idle = true;
		turn.controller?.abort();
		turn.controller = undefined;
		for (const resolve of turn.waiters.splice(0)) {
			resolve();
		}
	} else if (event === "session_start") {
		const reason = payload.reason as string | undefined;
		if (reason) {
			const waiting = sessionStarted.get(reason) ?? [];
			sessionStarted.delete(reason);
			for (const resolve of waiting) {
				resolve();
			}
		}
	}
}

/**
 * How long a session replacement is given to actually happen before giving up on it and answering
 * `{ cancelled: true }`.
 */
const SESSION_REPLACEMENT_TIMEOUT_MS = 10_000;

/** Wait for the next `session_start` with this reason, or give up. */
function waitForSessionStart(reason: string): Promise<boolean> {
	return new Promise((resolve) => {
		const waiting = sessionStarted.get(reason) ?? [];
		waiting.push(() => resolve(true));
		sessionStarted.set(reason, waiting);
		setTimeout(() => resolve(false), SESSION_REPLACEMENT_TIMEOUT_MS);
	});
}

/** Where this host is running, and whether the project it is running in is trusted. */
export const where = { cwd: process.cwd(), hasUI: false, mode: "tui" as string, trusted: false };

/** Fill in what micro says about this run. */
export function located(message: Json): void {
	where.cwd = (message.cwd as string) ?? process.cwd();
	where.hasUI = message.has_ui === true;
	where.mode = (message.mode as string) ?? "tui";
	where.trusted = message.trusted === true;
}

/** How long `snapshot` waits for micro before giving a call the empty answer instead. */
const SNAPSHOT_TIMEOUT_MS = 2_000;


export async function snapshot(commandContext: boolean): Promise<Json> {
	const asked = ask({ type: "request", request: "get_context", commandContext });
	const gaveUp = new Promise<Json>((resolve) => {
		setTimeout(() => resolve({}), SNAPSHOT_TIMEOUT_MS);
	});
	const answer = await Promise.race([asked, gaveUp]);
	if (Array.isArray(answer.activeTools)) {
		active = answer.activeTools as string[];
	}
	if (typeof answer.thinkingLevel === "string") {
		thinking = answer.thinkingLevel;
	}
	
	if ("sessionName" in answer) {
		named = typeof answer.sessionName === "string" ? answer.sessionName : undefined;
	}
	if (Array.isArray(answer.allTools)) {
		tools = answer.allTools as Json[];
	}
	if (Array.isArray(answer.commands)) {
		slashCommands = answer.commands as Json[];
	}
	return answer;
}

/** Every tool that exists, as the last snapshot described them. */
let tools: Json[] = [];

/** Every command that can be typed, as the last snapshot described them. */
let slashCommands: Json[] = [];

export function allTools(): Json[] {
	return tools;
}

export function commands(): Json[] {
	return slashCommands;
}

/** What the last snapshot said the thinking level was. */
let thinking = "off";

/** What the last snapshot said the session was called, if anything. */
let named: string | undefined;

export function thinkingLevel(): string {
	return thinking;
}

export function sessionName(): string | undefined {
	return named;
}

/** The tools the model is being offered, as of the last snapshot. */
let active: string[] = [];

/** The tool names the last snapshot reported. */
export function activeTools(): string[] {
	return active;
}


export function noteActiveTools(names: string[]): void {
	active = [...names];
}


function branchFrom(byId: Map<string, Json>, fromId: string | undefined): Json[] {
	const path: Json[] = [];
	let current = fromId ? byId.get(fromId) : undefined;
	while (current) {
		path.push(current);
		const parentId = current.parentId as string | null;
		current = parentId ? byId.get(parentId) : undefined;
	}
	path.reverse();
	return path;
}

/** pi's `ReadonlySessionManager`, built from the one snapshot `get_context` already took. */
function sessionManagerFor(session: Json | undefined): Json {
	const entries = (session?.entries as Json[] | undefined) ?? [];
	const labels = (session?.labels as Record<string, string> | undefined) ?? {};
	const byId = new Map<string, Json>();
	for (const entry of entries) {
		byId.set(entry.id as string, entry);
	}
	const leafId = (session?.leafId as string | null | undefined) ?? null;

	return {
		getCwd: (): string => (session?.cwd as string) ?? "",
		getSessionDir: (): string => (session?.dir as string) ?? "",
		getSessionId: (): string => (session?.id as string) ?? "",
		getSessionFile: (): string | undefined => (session?.file as string | undefined) ?? undefined,
		getLeafId: (): string | null => leafId,
		getLeafEntry: (): Json | undefined => (leafId ? byId.get(leafId) : undefined),
		getEntry: (id: string): Json | undefined => byId.get(id),
		getLabel: (id: string): string | undefined => labels[id],
		getBranch: (fromId?: string): Json[] => branchFrom(byId, fromId ?? leafId ?? undefined),
		
		buildContextEntries: (): Json[] => branchFrom(byId, leafId ?? undefined),
		getHeader: (): Json | null => (session?.header as Json | undefined) ?? null,
		getEntries: (): Json[] => entries,
		getTree: (): Json[] => {
			const children = new Map<string | null, Json[]>();
			for (const entry of entries) {
				const parentId = (entry.parentId as string | null) ?? null;
				const siblings = children.get(parentId) ?? [];
				siblings.push(entry);
				children.set(parentId, siblings);
			}
			const node = (entry: Json): Json => ({
				entry,
				children: (children.get(entry.id as string) ?? []).map(node),
				label: labels[entry.id as string],
			});
			return (children.get(null) ?? []).map(node);
		},
		getSessionName: (): string | undefined => (session?.name as string | undefined) ?? undefined,
	};
}


async function replacedSessionContext(ui: Json, extension: string): Promise<Json> {
	const { send } = wireFor(extension);
	const ctx = await contextFor(ui, extension, true);
	ctx.sendMessage = async (message: Json, options?: Json): Promise<void> => {
		send({ type: "action", action: "send_message", message, options: options ?? {} });
	};
	ctx.sendUserMessage = async (content: unknown, options?: Json): Promise<void> => {
		send({ type: "action", action: "send_user_message", content, options: options ?? {} });
	};
	return ctx;
}


async function replaceSession(
	ui: Json,
	extension: string,
	reason: string,
	request: Json,
	withSession: ((ctx: Json) => Promise<void>) | undefined,
): Promise<Json> {
	const arrived = waitForSessionStart(reason);
	const queued = await wireFor(extension).ask(request);
	if (queued.cancelled === true) {
		return queued;
	}
	if (!(await arrived)) {
		return { cancelled: true };
	}
	if (withSession) {
		await withSession(await replacedSessionContext(ui, extension));
	}
	return { cancelled: false };
}

/** The context every tool, command and handler is called with. */
export async function contextFor(ui: Json, extension: string, commandContext = false): Promise<Json> {
	return contextFrom(await snapshot(commandContext), ui, extension, commandContext);
}

/** The same context, built from a snapshot already taken. */
export function contextFrom(now: Json, ui: Json, extension: string, commandContext = false): Json {
	const { ask, send } = wireFor(extension);
	const model = now.model as Json | undefined;

	const context: Json = {
		ui,
		mode: where.mode,
		hasUI: where.hasUI,
		cwd: where.cwd,
		model,
		thinkingLevel: now.thinkingLevel,
		
		scopedModels: (now.scopedModels as Json[] | undefined) ?? [],
		sessionManager: sessionManagerFor(now.session as Json | undefined),
		isProjectTrusted: (): boolean => where.trusted,
		isIdle: (): boolean => turn.idle,
		signal: turn.controller?.signal,
		
		abort: (): void => {
			send({ type: "action", action: "abort" });
		},
		shutdown: (): void => {
			send({ type: "action", action: "shutdown" });
		},
		
		getContextUsage: (): Json | undefined =>
			model ? { tokens: null, contextWindow: model.contextWindow, percent: null } : undefined,
		
		compact: (options?: Json): void => {
			send({ type: "action", action: "compact", customInstructions: options?.customInstructions });
		},
		getSystemPrompt: (): string => (now.systemPrompt as string) ?? "",
	};

	
	if (commandContext) {
		context.waitForIdle = (): Promise<void> => {
			if (turn.idle) {
				return Promise.resolve();
			}
			return new Promise((resolve) => {
				turn.waiters.push(resolve);
			});
		};
		
		const systemPromptOptions = (now.systemPromptOptions as Json | undefined) ?? {};
		context.getSystemPromptOptions = (): Json => ({ cwd: where.cwd, ...systemPromptOptions });
		context.newSession = async (options?: Json): Promise<Json> => {
			
			return replaceSession(
				ui,
				"new",
				{ type: "request", request: "new_session" },
				options?.withSession as ((ctx: Json) => Promise<void>) | undefined,
			);
		};
		context.fork = async (entryId: string, options?: Json): Promise<Json> => {
			return replaceSession(
				ui,
				extension,
				"fork",
				{
					type: "request",
					request: "fork",
					entryId,
					position: options?.position,
				},
				options?.withSession as ((ctx: Json) => Promise<void>) | undefined,
			);
		};
		context.navigateTree = async (targetId: string, options?: Json): Promise<Json> => {
			void options;
			return ask({ type: "request", request: "navigate_tree", targetId });
		};
		context.switchSession = async (sessionPath: string, options?: Json): Promise<Json> => {
			return replaceSession(
				ui,
				extension,
				"resume",
				{ type: "request", request: "switch_session", sessionPath },
				options?.withSession as ((ctx: Json) => Promise<void>) | undefined,
			);
		};
		context.reload = async (): Promise<void> => {
			await ask({ type: "request", request: "reload" });
		};
	}

	return context;
}
