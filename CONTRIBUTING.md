# Contributing

Keep each pull request focused, include tests for changed behavior, and update user documentation in the same change. Discuss a subsystem replacement before implementing it.

## Development setup

Rust uses the toolchain pinned in `rust-toolchain.toml`. Build the workspace with:

```bash
cargo build --workspace
```

Bun is required for TypeScript extension work. Use Bun for dependency installation and TypeScript tests.

## Checks

Run the same Rust checks as CI:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

The extension compatibility command requires Bun and fails if an example cannot load or finish its plain test turn:

```bash
cargo test -p micro-cli --test extension_compatibility -- --ignored --nocapture
```

Documentation uses mdBook 0.5.4. Run `mdbook build` after changing `docs/` or `book.toml`, and check links and rendered tables in the generated book.

## Pull requests

Describe the user-visible change, the checks you ran, and any platform-specific coverage that remains. Keep local runtime data and unrelated generated files out of the commit. Add an entry under `Unreleased` in `CHANGELOG.md` for behavior users will notice.
