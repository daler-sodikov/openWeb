# Ferrule

Ferrule is an open-source IDE for JavaScript and TypeScript web development.
It aims for a WebStorm-like workflow (project tree, tool windows, Search
Everywhere on double Shift, an IntelliJ-style keymap) built from scratch on a
Rust core with a thin Tauri v2 + TypeScript view layer.

**Not affiliated with JetBrains.**

> Status: **Milestone 1 (editor core)**. Open a folder, browse it, edit
> files in tabs with tree-sitter highlighting, undo/redo and multi-cursor,
> save, and recover unsaved text after a crash. No code intelligence yet.
> See [docs/ROADMAP.md](docs/ROADMAP.md).

## Requirements

- Rust (installed through [rustup](https://rustup.rs); the exact toolchain is
  pinned in `rust-toolchain.toml` and installed automatically)
- [Bun](https://bun.sh) 1.3 or newer (the only JavaScript package manager
  used here)
- Tauri v2 system dependencies for your OS:
  <https://v2.tauri.app/start/prerequisites/>

## Run

```bash
bun install
bun tauri dev
```

A window titled "Ferrule" opens. Click **Open Folder…** and pick a project
(for example `fixtures/react-ts`), or pass a folder on the command line:
`bun tauri dev -- -- /absolute/path/to/project`.

## Check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check

bun run lint
bun run format:check
bun run typecheck
bun run test

bunx playwright install chromium   # once, for the UI tests
bunx playwright test               # smoke test (real app) + UI tests (Chromium)
```

On Linux without a display, run the end-to-end test under `xvfb-run -a`.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Roadmap](docs/ROADMAP.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## License

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE) and
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
