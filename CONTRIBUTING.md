# Contributing to Ferrule

Thanks for helping. This file is the short version; the full rules live in
[CLAUDE.md](CLAUDE.md) (written for both humans and coding agents) and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Ground rules

- **Rust owns the logic.** The TypeScript frontend is a view layer: no file
  IO, no language analysis, no business logic.
- **Typed IPC only.** IPC types are defined in Rust (`crates/ipc`) and
  generated into `src/ipc/bindings.ts` by `bun run gen:ipc`. Never hand-write
  a TypeScript type that exists in Rust. The generated file is not committed.
- **Bun only** for JavaScript tooling. Do not add `package-lock.json`,
  `pnpm-lock.yaml` or `yarn.lock`.
- **Originality.** Do not copy JetBrains code, icons, fonts, themes or
  trademarks, and do not copy code from other projects without checking the
  license and recording it in `NOTICE` and `THIRD_PARTY_LICENSES.md`.
- **No fake functionality.** Unfinished work uses `todo!()` and gets an
  entry in `docs/ROADMAP.md`.

## Before you open a pull request

Every check must pass with zero warnings:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
bun run lint
bun run format:check
bun run typecheck
bun run test
bunx playwright test
```

- Add tests for new behavior. UI features need a Playwright test using a
  project from `fixtures/`.
- Add an entry to `CHANGELOG.md` under "Unreleased".
- Use [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(buffer): ...`, `fix(lsp): ...`, `docs: ...`.
- Keep pull requests small and focused. Propose large rewrites in an issue
  first.
- New dependencies need a permissive license (checked by `cargo deny`) and a
  justification in the commit message.

## Disk usage

`target/` holds everything Cargo builds: compiled dependencies, the
incremental caches of workspace crates, test binaries and the app. It grows
with every build profile and every set of features (the app with and
without `native`, the headless test backend), and can reach many gigabytes.
To keep it smaller, debug builds keep only line tables for our crates and no
debug info for dependencies (see `[profile.dev]` in `Cargo.toml`); raise
`debug` locally if you need full debugger info.

- `cargo clean` deletes all of `target/` (the next build starts from
  scratch).
- [`cargo sweep`](https://crates.io/crates/cargo-sweep) (`cargo install
cargo-sweep`) deletes only stale files, e.g. `cargo sweep --time 14` for
  files not used in 14 days, or `cargo sweep --installed` for builds of
  toolchains you no longer have.
- Do not point `CARGO_TARGET_DIR` at extra copies or build in extra git
  worktrees unless you need to: each one is a full second `target/`.

## Code of conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
