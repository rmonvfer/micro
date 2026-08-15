# Extension examples

These examples exercise micro's extension API and its compatibility with extensions written for pi.

## Run an example

Load one file for a single run:

```bash
micro --extension examples/extensions/hello.ts
```

Or copy it into a trusted project's extension directory:

```bash
mkdir -p .micro/extensions
cp examples/extensions/hello.ts .micro/extensions/
micro
```

Packages with dependencies should be installed through micro:

```bash
micro install --local ./examples/extensions/with-deps
```

See [Extensions](../../docs/extensions.md) for API, installation, capabilities, and compatibility details.

## Useful starting points

| Example | Demonstrates |
| --- | --- |
| `hello.ts` | Minimal registered tool. |
| `commands.ts` | Slash-command registration. |
| `todo.ts` | Tool state and custom rendering. |
| `permission-gate.ts` | Blocking selected tool calls. |
| `protected-paths.ts` | Refusing writes to selected paths. |
| `dynamic-tools.ts` | Registering tools after startup. |
| `structured-output.ts` | Ending a turn through a structured-output tool. |
| `tool-override.ts` | Wrapping built-in tools. |
| `preset.ts` | Flags, commands, model selection, and active tools. |
| `plan-mode/` | A multi-file interactive extension. |
| `status-line.ts` | Footer status updates. |
| `custom-header.ts` | Custom terminal header. |
| `custom-footer.ts` | Custom terminal footer. |
| `modal-editor.ts` | Replacing the input editor. |
| `overlay-test.ts` | Focused overlays and input. |
| `subagent/` | Isolated subagent contexts. |
| `ssh.ts` | Delegating tool operations over SSH. |
| `custom-provider-anthropic/` | Registering a provider. |
| `with-deps/` | Package dependencies. |

The directory also contains examples for session control, compaction, autocomplete, themes, notifications, games, resource discovery, Git workflows, and provider hooks.

## Capability declarations

micro-specific examples declare the host features they use:

```ts
export const capabilities = ["tools", "commands", "ui"];
```

Requests outside the declared set return a capability error and are recorded in the session ledger.

## Compatibility suite

Run the noninteractive sweep with:

```bash
cargo test -p micro-cli --test extension_compatibility -- --nocapture
```

Interactive examples use a separate pseudo-terminal harness:

```bash
cargo test -p micro-cli --test interactive_extension_compatibility -- --nocapture
```

See [Testing extensions](../../docs/extension-testing.md) for the harnesses and fixtures.
