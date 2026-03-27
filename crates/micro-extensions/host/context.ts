// What a handler, tool or command is called with.
//
// pi hands an extension a context describing the session it is running in: where it is, what
// model is answering, what has been said, and the handful of things an extension may do to
// the run itself. micro answers most of these by asking, since the session lives on the
// other side of the wire.
//
// Two things are true at once about that. What is fixed for the run — the working
// directory, whether there is a UI, whether the project is trusted — is told once, when
// the extensions are loaded, and kept here. What changes turn to turn — the model, the
// thinking level, the system prompt, the session — is asked for fresh on every call, in a
// single round trip, so a context built early in a long run still describes the run as it
// is now rather than as it was when extensions were loaded.
//
// Two members pi's `ExtensionContext` offers have no answer here, deliberately:
//
// - `modelRegistry` is a live object whose job is resolving API keys for a model. Handing
//   that across the process boundary would mean an extension could reach a credential
//   through it even if every other path here keeps extensions away from credentials — the
//   boundary is doing useful work by staying closed here, not merely failing to open.
// - `hasPendingMessages()` asks about the interactive editor's own queue of messages typed
//   ahead of a running turn — state `micro-tui`'s `App` keeps and, tried once, turned out
//   not to be free to answer: folding it into `get_context` (the same way `isIdle`/`signal`
//   are answered) means every tool call, command and event asks the interface for it up
//   front, before an extension's own handler runs — and an extension's own asks to the
//   interface, in `ctx.ui`, share that same channel. Doing this unconditionally reordered
//   what the interface heard first on every call that also touched `ctx.ui`, which broke
//   tests elsewhere that were written against the order it had. `isIdle()`, `signal` and
//   `waitForIdle()` looked like the same gap and are not: they answer from state already
//   kept in this file, updated by lifecycle events (`agent_start`/`agent_settled`) that
//   arrive on their own, so answering them costs nothing extra on that channel.
//
// A property that is present but silently wrong is worse than one that is simply missing —
// an extension reading it would make a decision on it.

import { ask, type Json, send } from "./host-wire.ts";

/** Whether a run is in flight, and the controller whose signal is handed out for its
 *  duration. Updated as `agent_start`/`agent_settled` arrive — the same lifecycle events
 *  every extension can already listen for — so `isIdle()` and `signal` answer at once,
 *  with no round trip, from whatever this host last heard. `agent_settled` is sent
 *  whether a turn finished on its own or was abandoned partway (see `SettleGuard` in
 *  `crates/micro-agent/src/lib.rs`), so an interrupt clears this the same as a normal
 *  turn ending does. */
const turn = {
	idle: true,
	controller: undefined as AbortController | undefined,
	waiters: [] as Array<() => void>,
};

/** Resolvers waiting for the next `session_start` with a given `reason` — how
 *  `newSession`/`fork`/`switchSession` know the replacement they asked for actually
 *  happened, not only that the line asking for it was accepted. Keyed by reason since
 *  `session_start` fires for `"new"`, `"resume"` and `"fork"` alike, and a call waiting on
 *  one must not be woken by another. */
const sessionStarted = new Map<string, Array<() => void>>();

/**
 * Track the run's own state from the lifecycle events micro forwards, so `isIdle()`,
 * `signal` and `waitForIdle()` can answer from what this host already knows rather than
 * asking again. Called for every event `host.ts` dispatches, whether or not any extension
 * registered a handler for it — this has to see `agent_start`/`agent_settled`, and
 * `session_start`, regardless of what an extension is listening for.
 */
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

/** How long a session replacement is given to actually happen before giving up on it and
 *  answering `{ cancelled: true }` — long enough for real file I/O, short enough that a
 *  replacement an extension's own `session_before_*` handler refused does not hang the
 *  call that asked for it. There is no direct way to be told a refusal happened instead of
 *  a slow one still pending; giving up is this file's best reading of "it did not happen". */
const SESSION_REPLACEMENT_TIMEOUT_MS = 10_000;

/** Wait for the next `session_start` with this reason, or give up. `true` means it arrived
 *  in time — the replacement genuinely happened, not merely that the line asking for it
 *  was accepted. */
function waitForSessionStart(reason: string): Promise<boolean> {
	return new Promise((resolve) => {
		const waiting = sessionStarted.get(reason) ?? [];
		waiting.push(() => resolve(true));
		sessionStarted.set(reason, waiting);
		setTimeout(() => resolve(false), SESSION_REPLACEMENT_TIMEOUT_MS);
	});
}

/** Where this host is running, and whether the project it is running in is trusted. Told
 *  once when the extensions are loaded, because both are settled before a session starts
 *  and stay that way for the life of the run. */
export const where = { cwd: process.cwd(), hasUI: false, mode: "tui" as string, trusted: false };

/** Fill in what micro says about this run. */
export function located(message: Json): void {
	where.cwd = (message.cwd as string) ?? process.cwd();
	where.hasUI = message.has_ui === true;
	where.mode = (message.mode as string) ?? "tui";
	where.trusted = message.trusted === true;
}

/** How long `snapshot` waits for micro before giving a call the empty answer instead.
 *
 * A local round trip over stdio settles in well under this; the only time it is reached
 * is when nothing is listening for the request at all, which happens in a harness built
 * to test the wire protocol on its own rather than paired with micro's own answering
 * loop. Giving up after a couple of seconds means a tool call there runs with an empty
 * context instead of sitting out the full call timeout waiting on an answer nobody was
 * ever going to send. */
const SNAPSHOT_TIMEOUT_MS = 2_000;

/** What changes turn to turn, fetched in one round trip rather than one per member.
 *
 * `commandContext` asks micro to also assemble `systemPromptOptions` — worth the extra
 * weight only on a command's own snapshot, since `getSystemPromptOptions()` is never
 * offered anywhere else and a tool call or an event dispatch would be paying for skills
 * and context files it has no way to ask for. */
async function snapshot(commandContext: boolean): Promise<Json> {
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
	// Null is an answer here — a session that has not been named — so the field being
	// present at all is what says the snapshot spoke about it.
	if ("sessionName" in answer) {
		named = typeof answer.sessionName === "string" ? answer.sessionName : undefined;
	}
	return answer;
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

/** The tools the model is being offered, as of the last snapshot.
 *
 * pi answers `getActiveTools()` from state it already holds, so it is a plain array rather
 * than a promise. Nothing here can await inside that call, so the names are kept from every
 * snapshot as it arrives — one is taken before each event's handlers run, so a handler
 * reading this sees the list as it stood when its own event was dispatched. */
let active: string[] = [];

/** The tool names the last snapshot reported. */
export function activeTools(): string[] {
	return active;
}

/** Say what `setActiveTools` just asked for, rather than waiting for a snapshot to catch
 * up: the interface applies it on its own time, and an extension that sets the list and
 * reads it back in the same handler should see what it chose. */
export function noteActiveTools(names: string[]): void {
	active = [...names];
}

/** Walk from `fromId` to the root, in root-to-target order — pi's `getBranch`, and the
 *  same walk `buildContextEntries` does. Entries are addressed by id through `byId` rather
 *  than by re-searching the array on every step. */
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

/**
 * pi's `ReadonlySessionManager`, built from the one snapshot `get_context` already took —
 * every method here is a plain, synchronous read over data that arrived with this call,
 * not a fresh question to micro. That is what lets it match pi's own shape: every one of
 * its fourteen methods returns a value, not a promise of one.
 *
 * Built even when nothing asks for it, since the cost is a handful of closures over data
 * that was already fetched — the entries and the labels are the only part of `get_context`
 * this pays extra to send, and only because there is no way to answer `getEntry(id)` or
 * `getBranch(fromId)` synchronously otherwise.
 */
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
		// Not compaction-aware, unlike pi's: this is the raw path from root to the current
		// leaf, the same as `getBranch(getLeafId())`. micro's session log does not expose
		// its compaction records richly enough on this side of the wire to collapse
		// everything before one into the summary that replaced it the way pi's version
		// does, so a session that was compacted shows more here than the model actually
		// read on its next turn — more, never less, so nothing is hidden that was sent.
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

/**
 * pi's `ReplacedSessionContext`: an `ExtensionCommandContext` bound to the session that
 * just replaced the one a `newSession`/`fork`/`switchSession` call started from, plus the
 * two ways pi adds of speaking into it. Built fresh, the same as any other context here —
 * the "replaced" part is entirely in *when* this is built: only once the matching
 * `session_start` has actually arrived (see `replaceSession`), so `ctx.sessionManager` and
 * the rest of it already describe the new session, not the one that was left behind.
 */
async function replacedSessionContext(ui: Json): Promise<Json> {
	const ctx = await contextFor(ui, true);
	ctx.sendMessage = async (message: Json, options?: Json): Promise<void> => {
		send({ type: "action", action: "send_message", message, options: options ?? {} });
	};
	ctx.sendUserMessage = async (content: unknown, options?: Json): Promise<void> => {
		send({ type: "action", action: "send_user_message", content, options: options ?? {} });
	};
	return ctx;
}

/**
 * Type a session-replacement command into the interface, wait for the matching
 * `session_start` to say it actually happened, and run `withSession` against the session
 * that replaced the old one — pi's own order: `AgentSessionRuntime.newSession`/`fork`/
 * `switchSession` each await the replacement before calling `withSession`, and await
 * `withSession` before resolving. `{ cancelled: true }` here means one of two things pi
 * itself cannot always tell apart either: the reader (or an extension's own
 * `session_before_*` handler) refused it, or it simply has not happened within
 * `SESSION_REPLACEMENT_TIMEOUT_MS` — there is no channel back that says which.
 */
async function replaceSession(
	ui: Json,
	reason: string,
	request: Json,
	withSession: ((ctx: Json) => Promise<void>) | undefined,
): Promise<Json> {
	const arrived = waitForSessionStart(reason);
	const queued = await ask(request);
	if (queued.cancelled === true) {
		return queued;
	}
	if (!(await arrived)) {
		return { cancelled: true };
	}
	if (withSession) {
		await withSession(await replacedSessionContext(ui));
	}
	return { cancelled: false };
}

/**
 * The context every tool, command and handler is called with.
 *
 * Built fresh per call rather than kept, because most of what it answers is a question for
 * micro and the answer changes between one call and the next. Asks micro once for the
 * state a call might read, so a member such as `ctx.model` is a plain value by the time an
 * extension sees it rather than something it would have to await itself.
 *
 * `commandContext` adds the handful of members pi only ever hands to a command handler —
 * `waitForIdle`, `getSystemPromptOptions`, `newSession`, `fork`, `navigateTree`,
 * `switchSession`, `reload` — which either move the conversation somewhere else, wait on
 * it, or describe how the prompt itself was assembled, and are not safe or worth offering
 * to a tool or an event handler that did not ask to be run again with the result.
 */
export async function contextFor(ui: Json, commandContext = false): Promise<Json> {
	const now = await snapshot(commandContext);
	const model = now.model as Json | undefined;

	const context: Json = {
		ui,
		mode: where.mode,
		hasUI: where.hasUI,
		cwd: where.cwd,
		model,
		thinkingLevel: now.thinkingLevel,
		// Resolved against the catalog on micro's side; empty when nothing is scoped,
		// same as pi's own "the whole catalog is usable" reading of an empty list.
		scopedModels: (now.scopedModels as Json[] | undefined) ?? [],
		sessionManager: sessionManagerFor(now.session as Json | undefined),
		isProjectTrusted: (): boolean => where.trusted,
		isIdle: (): boolean => turn.idle,
		signal: turn.controller?.signal,
		// Reaches the interface as its own kind of ask, not a typed line: interrupting is
		// a keypress, and `App::ask_question`'s "abort" case calls the same `interrupt()`
		// a Ctrl+C would. With nothing running there is nothing to interrupt, the same as
		// pressing Ctrl+C on an idle prompt does something else entirely.
		abort: (): void => {
			send({ type: "action", action: "abort" });
		},
		shutdown: (): void => {
			send({ type: "action", action: "shutdown" });
		},
		// Unknown rather than estimated: nothing on this side of the wire tracks how many
		// tokens the running conversation holds, so the honest answer is that the token
		// count is not known, the same as pi's own type allows for right after a
		// compaction. What the model's context window holds is known, and is not.
		getContextUsage: (): Json | undefined =>
			model ? { tokens: null, contextWindow: model.contextWindow, percent: null } : undefined,
		// `customInstructions` has nowhere to go — micro's `/compact` takes no argument —
		// and `onComplete`/`onError` are never called: there is no channel back from this
		// action to the specific call that triggered it. An extension that wants to know
		// when a compaction finishes should listen for the `session_compact` event
		// instead, which fires whenever one does, triggered by this or by the reader.
		compact: (options?: Json): void => {
			send({ type: "action", action: "compact", customInstructions: options?.customInstructions });
		},
		getSystemPrompt: (): string => (now.systemPrompt as string) ?? "",
	};

	// Every one of these six is typed into the interface as though the reader had asked
	// for it themselves — `/new`, `/fork <n>`, `/tree <id>`, `/resume <id>`, `/reload` — the
	// same path `setModel`/`setThinkingLevel` already use elsewhere in this host.
	// `newSession`/`fork`/`switchSession` wait for the matching `session_start` before
	// resolving (see `replaceSession`), so `{ cancelled: false }` from those three means
	// the replacement genuinely happened, the same as pi's own promise does — and if
	// `options.withSession` was given, it has already run against the new session by the
	// time the promise resolves, same as pi. `navigateTree` and `reload` have no such
	// wait: pi does not offer `withSession` on `navigateTree`, and `reload` replaces
	// nothing to build a `ReplacedSessionContext` for, so both still only mean "queued",
	// and an extension wanting to know when either lands should listen for the matching
	// session event (`session_tree`, and so on) instead.
	if (commandContext) {
		context.waitForIdle = (): Promise<void> => {
			if (turn.idle) {
				return Promise.resolve();
			}
			return new Promise((resolve) => {
				turn.waiters.push(resolve);
			});
		};
		// `cwd` filled in from `where` rather than sent with the rest: it never changes
		// for the run, and micro already keeps it here. Everything else — whether
		// SYSTEM.md replaced the base prompt, what APPEND_SYSTEM.md added, which tools
		// contributed a snippet or a guideline, the instruction files and the skills that
		// loaded — came from `get_context`'s own answer, assembled once when the session
		// was built and read back rather than recomputed.
		const systemPromptOptions = (now.systemPromptOptions as Json | undefined) ?? {};
		context.getSystemPromptOptions = (): Json => ({ cwd: where.cwd, ...systemPromptOptions });
		context.newSession = async (options?: Json): Promise<Json> => {
			// `parentSession` and `setup` have nowhere to go: `/new` takes neither a
			// parent to fork context from nor a callback to seed the fresh session with,
			// and a callback could not cross this boundary regardless.
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
