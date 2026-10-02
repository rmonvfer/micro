import { ask, type Json, wireFor } from "./host-wire.ts";

/** Every model micro knows, of every type, as micro last described them. */
let catalog: Json[] = [];

/** The refresh in flight, so concurrent callers wait on one request. */
let refreshing: Promise<void> | undefined;

/** Take the catalog micro sends once a run has settled which models exist. */
export function sharedModels(models: unknown): void {
	if (Array.isArray(models)) {
		catalog = models as Json[];
	}
}

/** Read the catalog from micro again: every model, and whether its provider has a credential. */
export function refreshModels(): Promise<void> {
	refreshing ??= ask({ type: "request", request: "model_registry", op: "all" })
		.then((answer) => sharedModels(answer.models))
		.finally(() => {
			refreshing = undefined;
		});
	return refreshing;
}

function typeOf(model: Json): string {
	return (model.type as string | undefined) ?? "chat";
}

function strip(model: Json): Json {
	const { available: _available, ...rest } = model;
	return rest;
}

/** A model, or `{ provider, id }`, as the reference micro looks a model up by. */
function reference(model: Json | undefined): Json {
	if (!model || typeof model !== "object") {
		throw new Error("expected a model, such as one from ctx.modelRegistry.findOfType()");
	}
	return { provider: model.provider, id: model.id, type: typeOf(model) };
}

/** Choice criteria travel as `[key, description]` pairs, so their order survives micro. */
function orderedContext(context: Json): Json {
	const questions = (context?.questions as Record<string, Json> | undefined) ?? {};
	return {
		...context,
		questions: Object.entries(questions).map(([id, question]) => [
			id,
			question?.type === "choice" && question.criteria && !Array.isArray(question.criteria)
				? { ...question, criteria: Object.entries(question.criteria as Json) }
				: question,
		]),
	};
}

/** pi's `ctx.modelRegistry`: lookups answered from the catalog micro last reported, and the
 *  operations each model type accepts, run by micro with the session's credentials. */
export function modelRegistryFor(extension: string): Json {
	const wire = wireFor(extension);
	const ofType = (type: string, onlyAvailable: boolean): Json[] =>
		catalog
			.filter((model) => typeOf(model) === type && (!onlyAvailable || model.available === true))
			.map(strip);

	return {
		find: (provider: string, id: string): Json | undefined => {
			const found = catalog.find(
				(model) => typeOf(model) === "chat" && model.provider === provider && model.id === id,
			);
			return found ? strip(found) : undefined;
		},
		findOfType: (type: string, provider: string, id: string): Json | undefined => {
			const found = catalog.find(
				(model) => typeOf(model) === type && model.provider === provider && model.id === id,
			);
			return found ? strip(found) : undefined;
		},
		getModelOfType(type: string, provider: string, id: string): Json | undefined {
			return this.findOfType(type, provider, id);
		},
		getAll: (): Json[] => ofType("chat", false),
		getAvailable: (): Json[] => ofType("chat", true),
		getModelsOfType: (type: string): Json[] => ofType(type, false),
		getAvailableOfType: (type: string): Json[] => ofType(type, true),
		getAllModels: (): Json[] => catalog.map(strip),
		getAllAvailable: (): Json[] => catalog.filter((model) => model.available === true).map(strip),
		refresh: async (): Promise<void> => {
			await refreshModels();
		},

		/** Generate images with an image model; usage counts toward the session cost. */
		generateImages: async (model: Json, context: Json): Promise<Json> => {
			const answer = await wire.ask({
				type: "request",
				request: "generate_images",
				model: reference(model),
				context: context ?? { input: [] },
			});
			if (typeof answer.error === "string") {
				throw new Error(answer.error);
			}
			return answer.result as Json;
		},

		/** Answer typed questions with a classifier model; usage counts toward the session cost. */
		classify: async (model: Json, context: Json, options?: Json): Promise<Json> => {
			const answer = await wire.ask({
				type: "request",
				request: "classify",
				model: reference(model),
				context: orderedContext(context),
				options: options ?? {},
			});
			if (typeof answer.error === "string") {
				throw new Error(answer.error);
			}
			return answer.result as Json;
		},
	};
}
