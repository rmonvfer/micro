# Tools and integrations

micro gives the model a small built-in tool set. Extensions and MCP servers may add more tools to the same session.

## Built-in tools

| Tool                     | Purpose                                               |
| ------------------------ | ----------------------------------------------------- |
| `read`                   | Read a file or range of lines.                        |
| `write`                  | Create or replace a file.                             |
| `edit`                   | Replace one exact text region.                        |
| `multi_edit`             | Apply several edits to one file.                      |
| `ls`                     | List a directory.                                     |
| `grep`                   | Search file contents.                                 |
| `find`                   | Find paths by name or pattern.                        |
| `micro_docs`             | Read or search documentation embedded in the binary. |
| `bash`                   | Run a shell command.                                  |
| `codemode`               | Run JavaScript that calls the other tools. Offered on request; see [Codemode](codemode.md). |

The file tools resolve paths against the workspace selected by `-C` or the current directory. They reject absolute paths, lexical `..` traversal, and paths whose existing components resolve outside the workspace.

Reading a PNG, JPEG, GIF, WebP, or BMP hands the picture to the model rather than the bytes, and draws it in the transcript on the terminals that can show one. That is how the agent puts an image in front of you: it reads it.

`bash` runs under the selected [command sandbox](sandbox.md). Under the default policy, commands may read outside the workspace but may only write inside it. Protected paths remain read-only for built-in file tools and macOS shell commands; Linux shell commands enforce the workspace boundary but not protected descendants.

Interactive sessions may also expose `request_sandbox_access`. After a denial, the model can ask for network access or temporary-directory writes for one exact command or the rest of the session. Noninteractive modes do not provide this tool because they cannot show its approval dialog.

Tool output longer than 30,000 characters is truncated in the middle before it is returned to the model. When `bash` output is truncated, its full output is saved to a private `micro-bash-*.log` file in the temporary directory and the result ends with `[Output truncated. Full output: <path>]`, so the model can read or search the omitted part.

## Select tools

Use an allowlist:

```bash
micro --tools read,grep,find
```

Or remove tools from the normal set:

```bash
micro --exclude-tools write,edit,multi_edit,bash
```

Names are matched exactly. The allowlist is applied first; the denylist then removes names from the result.

The `default_tools` setting chooses the built-in tools a session starts with, in `config.json` or in a trusted project's `.micro/settings.json`. Plain names replace the full built-in set; `+name` adds a tool and `-name` removes one. A project list made only of `+name` and `-name` entries applies on top of the user's list, while a project list with plain names replaces it. An empty list turns every built-in tool off. Extension and MCP tools are not affected.

```json
{ "default_tools": ["-bash", "-write"] }
```

`/reload` turns on tools newly added to `default_tools`. Tools removed from it stay on, and a tool turned off during the session stays off unless it is newly added. `--tools` overrides the setting, also on reload, and tools removed with `--exclude-tools` cannot be turned on.

Inside an interactive session, `/tools` may be provided by an extension, but the built-in command-line flags remain the startup control.

## Tool failures

A failed tool call is returned to the model as an error result. It does not end the agent loop by itself.

Sandbox refusals include the active policy and are also written to the session ledger. Unknown tool names and malformed arguments use the same error-result path, so the model can correct the call on a later turn.

Tool calls from a response cut off by the provider's output-token limit are not executed.

## MCP servers

Configured MCP servers add tools named:

```text
mcp__<server>__<tool>
```

Servers are configured in `mcp.json`:

```json
{
  "mcpServers": {
    "notes": { "command": "/usr/local/bin/notes-mcp", "args": ["--stdio"] },
    "docs": { "url": "https://example.com/mcp" }
  }
}
```

A server that fails to connect is reported and skipped. Other tools remain available. Server processes are configured programs and do not run inside the command sandbox.

See [MCP servers](mcp.md) for HTTP servers, OAuth sign-in, exposure, and `micro mcp`.

## Deferred tool search

Large MCP and extension tool sets increase every provider request because their schemas are included in the prompt.

When the number of non-built-in tools exceeds `tool_search_threshold`, micro leaves those definitions out and adds `tool_search`. The model searches by name or description, receives matching definitions, and then calls the selected tool normally.

The default threshold is `15`. Set it to `0` to include every tool definition on every request.

Built-in tools are never deferred, and neither are the tools of an MCP server with `"exposure": "direct"`. The tools of servers with `codemode` or `deferred` exposure are never declared: those servers connect in the background, and `tool_search` waits for them before answering.

## Extension tools

Extensions register tools through the host API. They are filtered by the same `--tools` and `--exclude-tools` options as built-ins and MCP tools, and an extension tool's `exposure` decides whether it is declared, searched for, or only called from other tools.

An extension needs the `tools` capability to register one. See [Extensions](extensions.md).

A call to a tool without its own renderer, MCP tools included, is titled by its arguments. Collapsed, they follow the tool name as `key=value` pairs cut to 100 characters; expanded with `ctrl+o`, each argument gets its own `key: value` line, with strings written as they are.

## Mermaid diagrams

The terminal recognizes Mermaid code blocks in model responses and renders supported diagrams as Unicode art. Unsupported or invalid diagrams fall back to a framed source view.

The renderer supports flowcharts, state, class, entity-relationship, sequence, pie, mind map, timeline, journey, architecture, block, git graph, Kanban, packet, radar, Sankey, treemap, XY, Gantt, quadrant, and requirement diagrams.
