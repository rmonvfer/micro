# Security model

micro has separate controls for project-provided configuration, commands, and extensions. They apply at different points in startup and execution.

## Project trust

A project can include a `.micro/` directory with settings, extensions, skills, prompts, themes, and system-prompt overrides. micro asks for a trust decision before loading those resources.

A project without `.micro/` does not require a decision.

For a project that does, micro checks:

1. `--approve` or `--no-approve` for the current run.
2. A decision previously saved for the canonical project path.
3. `default_project_trust` from the user configuration.
4. An interactive prompt.

`--print` and `--rpc` cannot show an approval prompt. If no earlier decision applies, they ignore the project's `.micro/` resources.

Inside an interactive session:

```text
/trust on
/trust off
```

The decision applies on the next run because project resources are loaded during startup.

Trust does not disable the command sandbox. It only decides whether micro loads resources supplied by the project.

## Command confinement

The default command policy is `workspace-write`:

- shell commands may read outside the workspace but may write only inside it;
- built-in file tools keep `.git`, `.micro`, and micro's own data directories read-only; macOS shell commands inherit the same protection, while Linux shell commands are confined to the workspace boundary;
- network access is blocked;
- built-in file tools reject absolute paths, lexical traversal, and symlinks that resolve outside the workspace;

Extension commands sent through micro use the same command policy. The extension host has a separate, stricter process sandbox. Configured MCP servers remain outside the command sandbox.

Select another policy for one run:

```bash
micro --sandbox read-only
micro --sandbox full
```

`full` removes command confinement. [Command sandbox](sandbox.md) documents platform support, custom writable roots, diagnostics, and known gaps.

## Extension capabilities

An extension can declare the host features it uses, such as `tools`, `commands`, `exec`, `context`, or `ui`.

```ts
export const capabilities = ["commands", "ui"];
```

The extension host rejects requests outside that set. The extension receives a named error, the session continues, and the attempt is written to the ledger.

Legacy extensions without a manifest may require a one-time capability decision. A trusted project does not prompt again for capabilities of the extensions it ships because project trust already covers loading that code.

The Bun host runs with an empty inherited environment, no network access, and read-only access to the active workspace and loaded package roots. It is disabled on platforms where micro cannot enforce that sandbox.

Capabilities decide which host operations an extension may request. Direct filesystem writes from the Bun process are denied. A brokered `exec` request enters the active command sandbox, so the extension cannot widen the session policy through `micro.exec`.

## What the sandbox does not cover

The command sandbox does not wrap:

- configured MCP server processes;
- commands entered manually with `!` in the terminal;
- micro's own provider network requests.

Those processes and actions run with the permissions of the user who started micro. The extension host is covered by its own confinement described above.

## Recorded decisions

Sandbox refusals and extension capability crossings are recorded in the session ledger. Export it with:

```bash
micro sessions export <SESSION_ID>
```

The ledger is an audit record, not a prevention mechanism. The sandbox and capability broker make the decision; the ledger preserves what they decided.

## Data and privacy

micro does not send telemetry, crash reports, analytics, or installation identifiers. Session directories use owner-only permissions on Unix; logs, metadata, and retained provider request bodies are owner-readable and owner-writable. The files remain local unless you export, copy, or share them.

Model requests are sent to the provider selected for the session. Remote-control payloads are encrypted, but pairing assumes a relay that does not substitute keys. Relay connections require HTTPS, and authenticated frames cannot be replayed after reconnect. Read the [remote-control threat model](remote-control.md#encryption-and-relay). `/share` uploads the conversation to a secret GitHub gist using the token you configured.
