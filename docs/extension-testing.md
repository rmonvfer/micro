# Testing extensions

The extension tests cover load compatibility, host capabilities, and terminal rendering.

## Compatibility sweep

Run every example extension in a separate micro process:

```bash
cargo test -p micro-cli --test extension_compatibility -- --nocapture
```

Each process uses a scratch `MICRO_DIR`, workspace, and Bun runtime. Single files are loaded from `.micro/extensions`; packages use micro's normal installation path.

The test reports the `note: <path> was not loaded: <reason>` message produced by the binary. An extension that cannot be exercised automatically is reported separately instead of counted as passing.

## Capability tests

```bash
cargo test -p micro-cli --test extension_capabilities
```

These tests run the real extension host, request allowed and refused operations, and inspect the exported session ledger. They verify that a refusal is returned to the extension, does not terminate the session, and produces an `extension_crossing` event.

The suite also covers manifest-free extensions in trusted and untrusted projects.

## Terminal tests

```bash
cargo test -p micro-cli --test interactive_extension_compatibility -- --nocapture
```

`crates/micro-cli/tests/pty/drive.py` runs micro under a pseudo-terminal, answers terminal queries, sends keystrokes, and captures the result. Use this harness for widgets, overlays, custom editors, and other extensions whose behavior depends on a real terminal.

`crates/micro-cli/tests/inline_mode.rs` replays terminal escape sequences into a screen grid and checks the final screen. This catches rendering problems that cannot be detected by counting text in the raw output stream.

## Test fixtures

The helpers in `crates/micro-cli/tests/support` provide a scratch home, scratch workspace, and a fake provider that requires no network or credentials.

Prefer `--print` for extensions that do not need terminal input. Use the PTY harness only when the extension's behavior depends on rendering or keystrokes.
