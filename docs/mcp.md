# MCP servers

micro connects to [Model Context Protocol](https://modelcontextprotocol.io) servers over stdio or streamable HTTP and offers their tools to the model.

## Quick setup

Add a local server, check it, then start a session:

```bash
micro mcp add filesystem -- npx -y @modelcontextprotocol/server-filesystem .
micro mcp list
micro
```

A remote server takes a URL instead of a command:

```bash
micro mcp add docs --url https://example.com/mcp --bearer-token-env-var DOCS_TOKEN
```

These commands write the global file. Add `-l` (`--local`) to write the project's `.micro/mcp.json` instead.

## Configure servers

Servers live in `mcp.json` in micro's configuration directory and, once the project is [trusted](security.md#project-trust), in `.micro/mcp.json`. A project entry replaces a global entry with the same name. The format matches other MCP clients, so an `mcpServers` entry can be copied over as it is:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
    },
    "docs": {
      "url": "https://example.com/mcp",
      "headers": { "Authorization": "Bearer ${DOCS_TOKEN}" },
      "description": "Search and read the product documentation"
    }
  }
}
```

Stdio servers use `command`, `args`, `env`, and `cwd`; a relative `cwd` is taken from the workspace and a leading `~/` names the home directory. HTTP servers use `url`, `headers`, `oauth`, and `auth`. The legacy SSE transport is rejected; such servers usually offer streamable HTTP at `/mcp`. Values in `env`, `headers`, and `oauth.clientSecret` may name environment variables as `${NAME}`, or be the output of a command written as `!command`.

Every server also accepts `enabled: false` to keep the entry without connecting, `timeout` in seconds for each request (60 by default, 120 for tool calls), `description` for the system prompt, `exposure`, and `toolExposure`.

Connecting to an HTTP server is retried twice after a network failure or a transient status (408, 429, or 5xx other than 501). An answer stream that breaks off after the server numbered its events is resumed from the last one, retrying the same failures with backoff. Tool calls are never sent twice, since the server may already have run them.

Server names may use letters, digits, `_`, and `-`. Tools are named `mcp__<server>__<tool>`, with every other character, including `-`, written as `_`. Tools of one server whose names then collide all get a hash suffix, and server names that differ only in `-` and `_` are rejected. An invalid entry is reported and skipped; the other servers still connect.

Extensions can add servers for the session with [`micro.registerMcpServer()`](extensions.md#add-mcp-servers); they appear in `/mcp` with the `extension` scope.

## Control exposure

A server's `exposure` decides how the model reaches its tools. With `codemode`, the default, the tools are callable from [`codemode`](codemode.md) scripts, which find them with `searchTools()` or `describeNamespace()`, but they are neither declared to the model nor listed in the `codemode` description; micro offers `codemode` whenever such a server is enabled. `codemode-deferred` is another name for `codemode`. With `deferred`, `tool_search` finds the tools and the model then calls them by name. With `direct`, the tools are declared like built-in ones, and the first prompt waits for the server. With `hidden`, the server is not connected at all.

Servers with `codemode` or `deferred` exposure connect in the background, so the first prompt does not wait for them; `tool_search` waits for them when it runs, and so does a script that mentions `mcp__` or searches for tools. Their tools are reachable both ways: scripts call them, and `tool_search` finds them. A tool `tool_search` found stays callable by name when the session is resumed: a call made before its server has reconnected waits for it.

Servers whose tools are not declared are listed in the system prompt with one line each, from `description` or, once connected, from the first line of what the server says about itself. The list is checked at the start of each prompt. When it changed, for example after a server connected and said what it offers, the new list is added to the conversation ahead of the prompt, so the system prompt and the prefix the provider cached stay as they were.

```json
{ "mcpServers": { "github": { "url": "https://api.githubcopilot.com/mcp/", "exposure": "deferred" } } }
```

`toolExposure` sets the exposure of single tools over the server's. Keys are the server's names for its tools, or patterns in which `*` stands for any characters; an exact name wins over a pattern, and a longer pattern over a shorter one. A `hidden` server can offer just the tools it names, and a server is waited for at the first prompt when any of its tools is `direct`:

```json
{
  "mcpServers": {
    "github": {
      "url": "https://api.githubcopilot.com/mcp/",
      "exposure": "hidden",
      "toolExposure": { "search_issues": "direct", "get_*": "deferred" }
    }
  }
}
```

Scripts receive the whole `CallToolResult`, `content`, `structuredContent`, and `isError` included, and `describeNamespace("mcp__<server>")` returns the server's instructions and tool names. A result with `isError` resolves inside a script but reaches the model as an error when called directly.

## Authenticate with OAuth

A remote server that uses OAuth needs nothing in `mcp.json` but its URL. When it refuses an unauthenticated connection, micro reports that it needs a sign-in. Run `micro mcp login <server>`, or `/mcp login <server>` in a session. micro opens the authorization page and prints it as a link; when the browser runs on another machine, paste the address it was sent back to into `micro mcp login`.

micro finds the authorization server through the server's protected resource metadata, registers itself, and runs the authorization code flow with PKCE against a loopback callback. It rejects an authorization response whose `iss` names a different authorization server, refreshes tokens before they lapse or when the server refuses them, and, when a server asks for more scope, signs in again for the new scope together with the scope already granted. Credentials are kept in `mcp-auth.json` beside `auth.json`, readable only by you, per server name and URL. `micro mcp logout <server>` deletes them.

OAuth applies to HTTP servers without an `Authorization` header or `auth`. The `oauth` object adjusts it:

```json
{
  "mcpServers": {
    "figma": { "url": "https://mcp.figma.com/mcp", "oauth": { "clientName": "Claude Code" } },
    "corp": {
      "url": "https://mcp.example.com/mcp",
      "oauth": {
        "clientId": "my-client",
        "clientSecret": "${CORP_SECRET}",
        "callbackPort": 8765,
        "authServerMetadataUrl": "https://example.okta.com/.well-known/openid-configuration"
      }
    }
  }
}
```

`clientName` is the name micro registers under, for servers that only accept clients they know. `clientId` and `clientSecret` name a client registered ahead of time; its redirect URI is `http://127.0.0.1:<callbackPort>/callback`, or `callbackUrl` for another loopback URI. `scope` adds scopes to those the server advertises. `authServerMetadataUrl` replaces discovery for servers that advertise the wrong authorization server or none; it is trusted as configured and must use https except on loopback.

## Use a provider credential

`"auth": { "provider": "<name>" }` sends that provider's micro credential, the one `micro auth login` stores, as the bearer token on every request, so a refreshed token applies at once:

```json
{ "mcpServers": { "hosted": { "url": "https://mcp.example.com/mcp", "auth": { "provider": "openai" } } } }
```

Only the global `mcp.json` may use it, so a repository cannot choose where your credential goes, and the URL must use https except on loopback.

## Manage servers

`micro mcp list` connects every server and prints its state and tools; it exits with status 1 when an entry is invalid or an enabled server does not connect, and `--json` prints the same as JSON. `micro mcp add` writes an entry, with `--env`, `--cwd`, `--header`, `--description`, `--exposure`, `--oauth-client-id`, `--oauth-client-secret`, `--oauth-callback-port`, `--oauth-client-name`, and `--auth-provider`. `micro mcp remove` deletes one.

In a session, `/mcp` lists the servers with their state, servers that need attention first. Selecting one offers to sign in, sign out, or reconnect. A server signed in or reconnected during a session offers its tools through `tool_search`, so the tools declared to the model, and the cached prompt, stay as they were.

Log messages servers send, and what stdio servers write to their standard error, are appended to `mcp.log` in micro's data directory as `<time> [<server>] <level> <logger>: <message>`, with `stderr` as the level for standard error. The file moves to `mcp.log.1` once it grows past 5 MB. A server that fails to start also shows the end of its standard error in the error.

Tool calls are shown as `server/tool`. MCP servers are programs you configured and do not run inside the command sandbox.
