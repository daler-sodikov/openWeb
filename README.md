# Ferrule

Ferrule is an open-source IDE for JavaScript and TypeScript web development.
It aims for a WebStorm-like workflow (project tree, tool windows, Search
Everywhere on double Shift, an IntelliJ-style keymap) built from scratch on a
Rust core with a thin Tauri v2 + TypeScript view layer.

**Not affiliated with JetBrains.**

> Status: **Milestones 0 to 4 built on `main`; M5 (run, debug, tests) not on `main` yet.**
> Working: project tree, tabs, split editors, Search Everywhere and Go to
> File/Symbol/Line, Find/Replace in Files, folding, EditorConfig and syntax
> highlighting (M1, M2, M2b); language server-powered diagnostics, hover, Go to
> Definition/Type/Implementation, completion, parameter hints, find usages,
> rename with preview, Alt+Enter quick fixes, formatting and organize imports
> (M3a, M3b); an integrated terminal (M4a) and Git basics: status, gutter
> markers, diff, commit, log, branches and blame (M4b). Verified locally on
> Linux only; CI on GitHub is not passing on every platform yet. M5a (run
> configurations and test runner) exists on the unmerged `m5a` branch and is
> not yet verified in CI. Known gaps include hover links that do not open,
> diagnostics only for open files, and four missed performance budgets. See
> [docs/ROADMAP.md](docs/ROADMAP.md) for the full list of open items.

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
