import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const capabilities = ["events", "session_write"];

export default function (pi: ExtensionAPI) {
	pi.on("before_provider_request", (event) => {
		pi.appendEntry("provider-payload", {
			phase: "request",
			payload: event.payload,
		});

		// Optional: replace the payload instead of only logging it.
		// return { ...event.payload, temperature: 0 };
	});

	pi.on("after_provider_response", (event) => {
		pi.appendEntry("provider-payload", {
			phase: "response",
			status: event.status,
			headers: event.headers,
		});
	});
}
