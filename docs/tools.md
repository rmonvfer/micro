# Tools and integrations

micro gives the model a small built-in tool set. Extensions and MCP servers may add more tools to the same session.

## Built-in tools

| Tool | Purpose |
| --- | --- |
| `read` | Read a file or range of lines. |
| `write` | Create or replace a file. |
| `edit` | Replace one exact text region. |
| `multi_edit` | Apply several edits to one file. |
| `ls` | List a directory. |
| `grep` | Search file contents. |
| `find` | Find paths by name or pattern. |
| `bash` | Run a shell command. |

The file tools resolve paths against the workspace selected by `-C` or the current directory. Absolute paths and `..` traversal cannot escape that workspace, including for reads.

`bash` runs under the selected [command sandbox](sandbox.md). Under the default policy, commands may read outside the workspace but may only write inside it. `.git`, `.micro`, and micro's own data remain read-only.

Tool output longer than 30,000 characters is truncated in the middle before it is returned to the model.

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

Example configuration:

```json
{
  "mcp_servers": {
    "notes": {
      "command": "/usr/local/bin/notes-mcp",
      "args": ["--stdio"],
      "env": { "NOTES_HOME": "/srv/notes" }
    }
  }
}
```

An MCP server that fails to start is reported and skipped. Other tools remain available. Server processes are configured programs and do not run inside the command sandbox.

See [Configuration](configuration.md) for timeouts, working directories, and disabling a server.

## Deferred tool search

Large MCP and extension tool sets increase every provider request because their schemas are included in the prompt.

When the number of non-built-in tools exceeds `tool_search_threshold`, micro leaves those definitions out and adds `tool_search`. The model searches by name or description, receives matching definitions, and then calls the selected tool normally.

The default threshold is `15`. Set it to `0` to include every tool definition on every request.

Built-in tools are never deferred.

## Extension tools

Extensions register tools through the host API. They are filtered by the same `--tools` and `--exclude-tools` options as built-ins and MCP tools.

An extension needs the `tools` capability to register one. See [Extensions](extensions.md).

## Mermaid diagrams

The terminal recognizes Mermaid code blocks in model responses and renders supported diagrams as Unicode art. Unsupported or invalid diagrams fall back to a framed source view.

The renderer supports flowcharts, state, class, entity-relationship, sequence, pie, mind map, timeline, journey, architecture, block, git graph, Kanban, packet, radar, Sankey, treemap, XY, Gantt, quadrant, and requirement diagrams.
