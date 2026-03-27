// The registry every live component is driven through, by id, from the other side of the
// wire.
//
// A Component never crosses the wire — it is three or four methods, and a method cannot be
// written down as JSON — but a call to it by id can, the same shape `execute()` already uses
// for a tool that stays in this process and is driven by id rather than shipped over. This
// file is the map from an id back to the object it names, and the four operations micro
// drives one through: asked for its lines, offered a key, told to invalidate, retired.
//
// See `host.ts`'s `case "component"` for the messages this answers, and
// `crates/micro-extensions/src/host.rs`'s `Host::render_component` and the methods beside it
// for what asks.

import { send } from "./host-wire.ts";

/**
 * pi's own `Component` (`pi/packages/tui/src/tui.ts`), adapted for a caller on the other
 * side of a pipe rather than in the same process: `handleInput` answers whether it consumed
 * the key instead of returning nothing, since nothing here is close enough to the keyboard
 * to assume a key reaching a component was necessarily its to keep.
 */
export interface Component {
	/** Render the component to lines for the given viewport width. */
	render(width: number): string[];
	/** Handle a keystroke while this component has focus. */
	handleInput?(data: string): { consume?: boolean } | void;
	/** Drop any cached rendering state of its own — called when the theme changes, or when
	 * whatever is drawing this component otherwise decides it should recompute from
	 * scratch rather than reuse what it drew last. */
	invalidate?(): void;
	/** Release anything this component was holding — a timer, a subscription — now that
	 * nothing will ask it to render or handle input again. */
	dispose?(): void;
}

let nextId = 0;
const registry = new Map<string, Component>();

/** Register a component, and hand back the id it is driven by from here on. An object
 * rather than a bare string, matching pi's own convention of a result namely being able to
 * grow a field without every caller's destructuring breaking. */
export function registerComponent(component: Component): { id: string } {
	const id = `component-${nextId++}`;
	registry.set(id, component);
	return { id };
}

/** A component's lines at this width, or nothing for an id nobody has registered — the same
 * answer a disposed component gives, since by the time anything asks again there is no way
 * to tell "never existed" apart from "existed and was let go of". */
export function render(id: string, width: number): string[] {
	return registry.get(id)?.render(width) ?? [];
}

/** Offer a component a key, and say whether it consumed it. An id nobody has registered
 * answers as not consumed, the same as a component that simply declined it. */
export function input(id: string, data: string): { consume: boolean } {
	const said = registry.get(id)?.handleInput?.(data);
	return { consume: said?.consume === true };
}

/** Tell a component to drop any cached rendering state of its own. */
export function invalidate(id: string): void {
	registry.get(id)?.invalidate?.();
}

/** Retire a component. Idempotent — disposing an id twice, or one nobody registered,
 * changes nothing the second time. */
export function dispose(id: string): void {
	const component = registry.get(id);
	if (!component) {
		return;
	}
	registry.delete(id);
	component.dispose?.();
}

/** Tell micro this component's lines are stale on its own schedule — a timer, something
 * arriving asynchronously — rather than in answer to a render this side was asked for.
 * `ToolRenderContext.invalidate` in `host-tools.ts` sends this same message directly rather
 * than calling through here, since by the time it fires the registry has already done its
 * part; this export exists for `host-ui.ts`'s components, which have no context object of
 * their own to carry the id in. */
export function pushChanged(id: string): void {
	send({ type: "component_changed", componentId: id });
}
