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

The sandbox applies to model-requested commands, built-in file tools, and commands that extensions run through micro. The default is `workspace-write`:

- writes are allowed inside the workspace;
- `.git`, `.micro`, and micro's own data directories remain read-only;
- network access is blocked;
- reads are not restricted.

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

Capabilities limit access to micro's host API. They are not an operating-system sandbox for arbitrary code inside the Bun process. Install extensions from sources you are willing to execute.

## What the sandbox does not cover

The command sandbox does not wrap:

- the extension host process itself;
- configured MCP server processes;
- commands entered manually with `!` in the terminal;
- micro's own provider network requests.

Those processes and actions run with the permissions of the user who started micro.

## Recorded decisions

Sandbox refusals and extension capability crossings are recorded in the session ledger. Export it with:

```bash
micro sessions export <SESSION_ID>
```

The ledger is an audit record, not a prevention mechanism. The sandbox and capability broker make the decision; the ledger preserves what they decided.

## Data and privacy

micro does not send telemetry, crash reports, analytics, or installation identifiers. Session logs remain in the local data directory unless you export, copy, or share them.

Model requests are sent to the provider selected for the session. Remote-control messages pass through the configured relay as encrypted payloads. `/share` is an explicit exception: it uploads the conversation to a secret GitHub gist using the token you configured.
