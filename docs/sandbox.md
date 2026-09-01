# Command sandbox

The sandbox restricts commands and file operations performed on behalf of the model. The default policy is `workspace-write`.

## Policies

| Policy            | Command reads | Command writes | Network            |
| ----------------- | ------------- | -------------- | ------------------ |
| `read-only`       | allowed       | blocked        | blocked            |
| `workspace-write` | allowed       | workspace only | blocked by default |
| `full`            | allowed       | unrestricted   | unrestricted       |

Under `workspace-write`, built-in file operations keep `.git`, `.micro`, and micro's own configuration and data directories read-only even when they are inside the workspace. The macOS command sandbox enforces the same protected subpaths for shell commands. Linux Landlock confines shell-command writes to the workspace but cannot subtract protected descendants from a writable parent.

On macOS, agent-run commands cannot commit under the default policy because `.git` is protected. On Linux, use `read-only` when shell commands must not modify repository metadata.

There is no default writable exception for `/tmp` or `$TMPDIR`. Configure a writable root when a toolchain needs one.

## Select a policy

For one run:

```bash
micro --sandbox read-only
micro --sandbox workspace-write
micro --sandbox full
```

Set a default in `config.json`:

```json
{ "sandbox": "workspace-write" }
```

A trusted project may set the same key in `.micro/settings.json`. Command-line selection takes precedence over project configuration, which takes precedence over user configuration.

Add writable roots or network access with a policy object:

```json
{
  "sandbox": {
    "mode": "workspace-write",
    "writable_roots": ["/srv/cache"],
    "allow_network": true
  }
}
```

`--sandbox` also accepts a JSON object.

## Interactive configuration

In the terminal interface, `/sandbox` opens controls for the active session, the user default, and the trusted project policy. Session changes apply immediately. User and project settings apply to new sessions; project settings require `/trust on` and are saved in `.micro/settings.json`.

When a sandbox denial blocks the agent, it may request either network access or a writable temporary directory. The confirmation dialog identifies the capability, reason, and exact command, and offers `Allow once`, `Allow for this session`, or `Deny`. A one-time approval is consumed only by that exact command.

## Platform support

| Platform | Commands             | Built-in file tools    | Network blocking |
| -------- | -------------------- | ---------------------- | ---------------- |
| macOS    | Seatbelt             | in-process path checks | yes              |
| Linux    | Landlock and seccomp | in-process path checks | yes              |
| Windows  | not yet confined     | in-process path checks | not yet          |

When command confinement is unavailable, micro reports that commands are running unconfined. File tools still apply path checks.

Built-in file tools reject absolute paths and lexical `..` traversal outside the workspace. They canonicalize existing path components before opening a file, so a workspace symlink cannot redirect a read or write outside the workspace. The selected policy still decides whether an in-workspace write is allowed.

## What is covered

The policy applies to:

- the built-in command and file tools;
- commands an extension runs through `micro.exec`;
- commands run through micro's other built-in agent tools.

It does not wrap configured extension-host or MCP-server processes, manual `!` commands, or micro's provider connections. See [Security model](security.md).

## Refusals

A denied command returns a normal tool result with a non-zero exit code and an explanation:

```text
denied by policy workspace-write: exit code 1
touch: /etc/hosts: Operation not permitted
```

A denied file operation is rejected before the file is opened:

```text
cannot write /etc/hosts: workspace-write allows writes under /home/you/project only
```

The model receives the refusal and can choose another approach. A `sandbox_decision` event is appended to the session ledger.

Extension calls through `micro.exec` receive `denied: true` and the policy name in addition to the command result.

## Test a command

Use the same policy resolution as an agent session without contacting a model:

```bash
micro sandbox try -- touch ../outside.txt
micro sandbox try --sandbox read-only -- touch inside.txt
```

The output reports the resolved policy, whether OS enforcement is available, the wrapped command, its output, exit status, and whether the result looks like a denial.

## Linux helper

Linux applies Landlock and seccomp in the process that becomes the command. micro starts an internal helper, passes it the resolved policy, applies restrictions, and then replaces the helper process with the requested command.

The parent process resolves paths and policy. The helper does not reinterpret them.

## Known gaps

- Windows command confinement is not implemented.
- Landlock can only apply path rules to entries that already exist. In-process file checks still protect reserved paths that do not yet exist.
- Linux Landlock rules are additive. Under `workspace-write`, shell commands can modify protected paths inside the workspace; built-in file tools still reject those writes.
- A non-zero command result is not always distinguishable from an OS-level denial. micro uses platform signals and known error text to classify it for reporting; the kernel or file-tool check remains the authority on whether the operation was allowed.

Use `micro sandbox try` on the target platform when policy behavior is part of a deployment or CI requirement.
