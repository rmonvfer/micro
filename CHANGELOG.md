# Changelog

## [Unreleased]

## [0.1.12] - 2026-09-01

- Allow manual compaction below the automatic threshold.
- Fork sessions from the persisted conversation branch.
- Exclude Git internals from file completion.
- Add public and authenticated installation bootstrap commands.
- Ignore local generated artifacts.
- Add the MIT license, security policy, and contribution guide.
- Reject filesystem symlink escapes and keep the Bun extension host read-only.
- Apply the session sandbox to RPC shell commands and rebuild the provider runtime on model switches.
- Pair phones through a secret-bearing QR code, require encrypted remote relays, and reject authenticated frame replays after reconnect.
- Store session files with owner-only permissions and make failed deletion retryable.
- Record request pricing for stable historical bills and correct compaction branch totals.
- Preserve run-only trust and refresh the skill command registry during `/reload`.
- Refresh live model listings at startup when `live_models` is enabled.
- Load all maintained extension examples and enforce the compatibility sweep in CI.
- Include MIT and Apache licensing material in release archives.

## [0.1.11] - 2026-08-14

- Authenticate managed update checks and downloads for private GitHub releases.

## [0.1.10] - 2026-08-10

- Reject external entities and DTD declarations while parsing Typst XML.
- Authenticate the dependency-audit workflow for the private repository.

## [0.1.9] - 2026-07-31

- Publish release artifacts from the private repository.

## [0.1.8] - 2026-07-24

- Align Linux sandbox setup and tests with hosted CI runners.

## [0.1.7] - 2026-07-20

- Avoid root propagation remounts during Linux sandbox setup.

## [0.1.6] - 2026-07-13

- Isolate RPC interruption test workspaces.

## [0.1.5] - 2026-07-06

- Map the Linux sandbox child process from its parent namespace.

## [0.1.4] - 2026-07-01

- Map the sandbox identity before entering the Linux namespace.

## [0.1.3] - 2026-06-23

- Preserve permitted root writes under the Linux sandbox.

## [0.1.2] - 2026-06-18

- Pass the Linux sandbox lint gate.

## [0.1.1] - 2026-06-15

- Improve Linux handling for protected sandbox paths. Landlock cannot exclude protected descendants from a writable workspace; see the [known gaps](docs/sandbox.md#known-gaps).

## [0.1.0] - 2026-06-08

The first release includes the terminal agent, provider integrations, append-only sessions, billing, prompt-cache diagnostics, project configuration, command sandboxing, extensions, MCP tools, remote control, and managed updates. The core agent is a Rust binary; TypeScript extensions require Bun.

[Unreleased]: https://github.com/rmonvfer/micro/compare/v0.1.12...HEAD
[0.1.12]: https://github.com/rmonvfer/micro/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/rmonvfer/micro/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/rmonvfer/micro/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/rmonvfer/micro/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/rmonvfer/micro/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/rmonvfer/micro/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/rmonvfer/micro/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/rmonvfer/micro/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/rmonvfer/micro/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/rmonvfer/micro/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/rmonvfer/micro/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/rmonvfer/micro/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rmonvfer/micro/releases/tag/v0.1.0
