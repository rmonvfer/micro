

import { send } from "./host-wire.ts";


export interface Component {
	/** Render the component to lines for the given viewport width. */
	render(width: number): string[];
	/** Handle a keystroke while this component has focus. */
	handleInput?(data: string): { consume?: boolean } | void;
	/** Drop any cached rendering state of its own. */
	invalidate?(): void;
	/** Release anything this component was holding. */
	dispose?(): void;
}

let nextId = 0;
const registry = new Map<string, Component>();

/** Which extension registered each live component. */
const owners = new Map<string, string>();

/** Register a component, and hand back the id it is driven by from here on. */
export function registerComponent(component: Component, owner?: string): { id: string } {
	const id = `component-${nextId++}`;
	registry.set(id, component);
	if (owner) {
		owners.set(id, owner);
	}
	return { id };
}


export function disposeOwnedBy(owner: string): string[] {
	const owned = [...owners.entries()]
		.filter(([, held]) => held === owner)
		.map(([id]) => id);
	for (const id of owned) {
		dispose(id);
	}
	return owned;
}

/** A component's lines at this width, or nothing for an id nobody has registered. */
export function render(id: string, width: number): string[] {
	return registry.get(id)?.render(width) ?? [];
}

/** Offer a component a key, and say whether it consumed it. */
export function input(id: string, data: string): { consume: boolean } {
	const said = registry.get(id)?.handleInput?.(data);
	return { consume: said?.consume === true };
}

/** Tell a component to drop any cached rendering state of its own. */
export function invalidate(id: string): void {
	registry.get(id)?.invalidate?.();
}

/** Retire a component. */
export function dispose(id: string): void {
	const component = registry.get(id);
	if (!component) {
		return;
	}
	registry.delete(id);
	owners.delete(id);
	component.dispose?.();
}

/** Tell micro this component's lines are stale on its own schedule. */
export function pushChanged(id: string): void {
	send({ type: "component_changed", componentId: id });
}
