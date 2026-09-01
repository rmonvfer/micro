/**
 * File Trigger Extension
 *
 * Watches a trigger file and injects its contents into the conversation.
 * Useful for external systems to send messages to the agent.
 *
 * Usage:
 *   echo "Run the tests" > agent-trigger.txt
 */

import * as fs from "node:fs";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const capabilities = ["events", "send_message", "exec", "ui"];

export default function (pi: ExtensionAPI) {
	pi.on("session_start", async (_event, ctx) => {
		const triggerFile = join(ctx.cwd, "agent-trigger.txt");

		fs.watch(triggerFile, async () => {
			try {
				const content = fs.readFileSync(triggerFile, "utf-8").trim();
				if (content) {
					pi.sendMessage(
						{
							customType: "file-trigger",
							content: `External trigger: ${content}`,
							display: true,
						},
						{ triggerTurn: true }, // triggerTurn - get LLM to respond
					);
					await pi.exec("sh", ["-c", ': > "$1"', "sh", triggerFile], { cwd: ctx.cwd });
				}
			} catch {
				// File might not exist yet
			}
		});

		if (ctx.hasUI) {
			ctx.ui.notify(`Watching ${triggerFile}`, "info");
		}
	});
}
