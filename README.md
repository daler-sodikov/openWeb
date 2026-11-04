# Ferrule

Ferrule is an open-source IDE for JavaScript and TypeScript web development.
It aims for a WebStorm-like workflow (project tree, tool windows, Search
Everywhere on double Shift, an IntelliJ-style keymap) built from scratch on a
Rust core with a thin Tauri v2 + TypeScript view layer.

**Not affiliated with JetBrains.**

> Status: **Milestone 0 (foundation)**. The app opens a window and talks to
> its Rust core over typed IPC. There is no editor yet. See
> [docs/ROADMAP.md](docs/ROADMAP.md).

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

A window titled "Ferrule" opens and shows the product name and version that
the frontend fetched from the Rust core.

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

bunx playwright test   # builds the app and runs the end-to-end smoke test
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
