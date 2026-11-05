# CLAUDE.md

Persistent instructions for working on this repository. Read this fully before
doing anything. If a rule here conflicts with a one-off request, ask first.

## Project

**Ferrule** (crates `ferrule-*`, binary `ferrule`, bundle id
`dev.ferrule.ide`) is an open-source IDE for JavaScript/TypeScript web
development. From the user's point of view it behaves
like JetBrains WebStorm: same workflow, tool windows, Search Everywhere
(double Shift) and an IntelliJ-style keymap preset. Internally it is a
from-scratch Rust implementation.

Reference for structure and tone: github.com/l0ng-ai/openidea (Apache-2.0,
Tauri + Rust core). Use it as architectural inspiration only. Never copy its
code.

## Stack

- **Core: Rust (stable).** All logic lives here: text buffer, indexing,
  language intelligence, LSP/DAP clients, Git, search, terminal, run/debug,
  settings, plugin host.
- **Shell: Tauri v2** (`src-tauri/`). IPC commands, window management.
- **Frontend: TypeScript + Vite** (`src/`). Thin view layer only.
- **JS tooling: Bun only.** Package manager and script runner. Do NOT use pnpm,
  npm or yarn. Commit `bun.lock`. Never create `package-lock.json`,
  `pnpm-lock.yaml` or `yarn.lock`.
- **Plugins:** WASM Component Model via `wasmtime`; interfaces in `wit/`.
- **License:** Apache-2.0, plus `NOTICE` and `THIRD_PARTY_LICENSES.md`.

## Repository layout

```
crates/        one crate per domain (buffer, vfs, project, indexer, lsp-client,
               dap-client, search, git, terminal, run-debug, settings,
               plugin-host, lang-js, lang-css, lang-html, ...)
src-tauri/     Tauri shell and IPC command layer (no business logic here)
src/           frontend: editor view, tool windows, popups
languages/     per-language config: tree-sitter grammars/queries, LSP server
               definitions, run configs
plugins/       example WASM plugins
wit/           WIT interface definitions for the plugin API
fixtures/      sample projects used by tests (React+TS, Next, Vue, monorepo)
e2e/           Playwright end-to-end tests
perf/          benchmark scripts
docs/          ARCHITECTURE.md, ROADMAP.md, media
```

## Commands

```bash
# setup and run
bun install
bun tauri dev

# Rust checks (must pass before every commit)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check

# frontend checks (must pass before every commit)
bun run lint
bun run format:check
bun run typecheck
bun run test

# end-to-end
bunx playwright test

# release build
bun tauri build
```

If a script above does not exist yet, create it as part of the milestone that
needs it.

## Architecture rules

1. **The frontend is thin.** No business logic, no language analysis, no file
   IO in TypeScript. The Rust buffer is the single source of truth for document
   text. The frontend sends commands and receives events over typed IPC.
2. **Typed IPC.** Generate TypeScript types from Rust (`specta` or `ts-rs`).
   Never hand-write a type that exists on the Rust side.
3. **Never block the UI.** All IO, indexing, LSP and search work is async on
   `tokio` and cancellable (`CancellationToken`). Never hold a lock across an
   `.await`. No blocking calls inside async functions; use
   `spawn_blocking` when unavoidable.
4. **Virtualize every long list** (file tree, search results, completions,
   problems, Git log).
5. **Incremental everything.** Incremental parsing (tree-sitter), incremental
   indexing with an on-disk cache, debounced LSP notifications.
6. **Never lose user data.** Unsaved edits are autosaved to recovery files.
   A crash must not lose text.
7. **Language servers are external processes** (typescript-language-server or
   vtsls, css/html/json servers, eslint-lsp, tailwindcss-language-server).
   Use `oxc` for fast in-process features (parsing, semantics, import
   resolution, unused imports, quick fixes). Do not reimplement the TypeScript
   type checker.
8. **Cross-platform from day one:** Linux, macOS, Windows. No hardcoded paths
   or separators; use `std::path` and `camino` where useful.

## Rust conventions

- Edition and toolchain pinned in `rust-toolchain.toml`.
- No `unwrap()`/`expect()` in non-test code unless the invariant is documented
  in a comment. Use `thiserror` in libraries, `anyhow` only in the app layer.
- Use `tracing` for logs, never `println!`.
- Public items get doc comments. Keep crate APIs small; prefer private modules.
- Keep crates acyclic and focused. If two crates need each other, extract a
  third.
- Prefer permissive dependencies only (MIT, Apache-2.0, BSD). Check with
  `cargo deny` before adding a crate. Justify every new heavy dependency in
  the commit message.
- No `unsafe` without a `// SAFETY:` comment and a test.

## Frontend conventions

- TypeScript `strict` mode, no `any` without a comment.
- Use Bun for everything: `bun add <pkg>`, `bun run <script>`, `bunx <tool>`.
- Vite is the bundler. Bun is only the package manager/runner.
- Tauri config: `beforeDevCommand: "bun run dev"`,
  `beforeBuildCommand: "bun run build"`.
- Playwright is officially Node-based. If it breaks under Bun, keep Node for
  that single CI step only and document it in `docs/ARCHITECTURE.md`. Do not
  silently change the rest of the toolchain.

## Feature scope (WebStorm parity)

Work in this order. Do not start a phase before the previous one is done.

1. **Editor:** project tree, tabs, split editors, Search Everywhere, Go to
   File/Symbol/Line, Recent Files, Find/Replace in Files (regex), breadcrumbs,
   folding, multi-cursor, bracket matching, auto-indent, EditorConfig,
   highlighting for JS/TS/JSX/TSX/HTML/CSS/SCSS/LESS/JSON/YAML/Markdown/Vue/
   Svelte.
2. **Code insight:** completion, parameter hints, quick docs, go to
   definition/type/implementation, find usages, project-wide rename, extract
   variable/function/component, auto-import, organize imports, diagnostics,
   Alt+Enter quick fixes, ESLint + Prettier, Emmet, tsconfig/package.json
   awareness, npm/pnpm/yarn workspaces.
3. **Tooling:** integrated terminal, run/debug configurations (Node, npm
   scripts, Jest, Vitest, Playwright), test runner with gutter icons, debugger
   UI (DAP + Chrome DevTools Protocol), HTTP client (`.http` files), npm
   dependency panel, Git UI (commit, diff, blame, branches, log, conflicts),
   local history, TODO, bookmarks, problems.
4. **Polish:** React/Next/Vue/Angular/Svelte/Tailwind support, settings UI,
   keymap editor, themes, WASM plugin API, auto-update, i18n, accessibility.

## Performance budget

Measured on a mid-size project (~1000 files), warm cache, release build:

| Metric                 | Target        |
| ---------------------- | ------------- |
| Window with project    | < 1 s         |
| Ready (CPU settled)    | < 2 s         |
| Memory (all processes) | < 250 MB      |
| Idle CPU               | < 5 % of core |
| Quit                   | < 0.5 s       |
| Typing latency         | < 8 ms        |

Benchmarks live in `perf/`. Compare against WebStorm only by external
measurement, using the same method for both. Never publish a number you did
not measure.

## Legal and originality (strict)

- Do NOT copy any JetBrains source code, icons, logos, fonts, themes or
  trademarks. Re-implement behavior from public documentation and open
  protocols (LSP, DAP, Chrome DevTools Protocol).
- Do NOT use "WebStorm", "JetBrains" or "IntelliJ" in the product name, UI,
  window titles, or binaries. Docs may say "IntelliJ-style keymap". The README
  must state: "Not affiliated with JetBrains."
- Original icon set and original dark/light themes.
- Do not copy code from the OpenIDEA reference repo or any other project
  without checking its license and recording it in `NOTICE` and
  `THIRD_PARTY_LICENSES.md`.

## Workflow

1. **Plan before code.** Before each milestone, update `docs/ARCHITECTURE.md`
   or `docs/ROADMAP.md` with the plan. Ask the user only when truly blocked.
2. **One milestone at a time.** Finish, verify, then stop and report.
3. **Definition of done for a milestone:**
   - all Rust and frontend checks above pass with zero warnings,
   - new behavior has unit/integration tests, and UI features have a
     Playwright test using `fixtures/`,
   - `CHANGELOG.md` entry added,
   - a short demo instruction: "run X, you should see Y".
4. **No fake functionality.** Unfinished work uses `todo!()` and gets an entry
   in `docs/ROADMAP.md`. Never ship stubs that pretend to work. Never claim a
   feature works without running it.
5. **Tests to always have:** text buffer operations, undo/redo, multi-cursor,
   LSP message handling, search, refactor edits, highlighting (golden tests),
   IPC contract.
6. **Commits:** small, reviewable, Conventional Commits
   (`feat(buffer): ...`, `fix(lsp): ...`). Do not commit generated files or
   build output.
7. **CI (GitHub Actions):** build and test on Linux/macOS/Windows, clippy,
   fmt, eslint, tsc, cargo-deny, release bundles. Use `oven-sh/setup-bun` and
   `bun install --frozen-lockfile`.
8. **Docs to maintain:** README, CONTRIBUTING, CODE_OF_CONDUCT, LICENSE,
   NOTICE, THIRD_PARTY_LICENSES.md, docs/ARCHITECTURE.md, docs/ROADMAP.md.

## Things not to do

- Do not add Electron, Java/Kotlin, or a JS-based analysis engine.
- Do not put logic in the frontend that belongs in Rust.
- Do not add a dependency without checking its license and maintenance state.
- Do not use pnpm, npm or yarn.
- Do not rewrite large parts of the codebase unprompted. Propose it first.
- Do not leave `dbg!`, `println!`, `console.log` or commented-out code behind.

## Current status

Update this section at the end of every milestone.

- [x] M0: docs/ARCHITECTURE.md, workspace skeleton, CI, licenses
      (2026-11-04; all checks pass locally on Linux, CI not yet run on GitHub;
      open follow-ups in docs/ROADMAP.md)
- [x] M1: window, project tree, tabs, rope buffer, highlighting
      (2026-11-04; checks pass locally on Linux; see docs/ROADMAP.md M1)
- [x] M2: search (files, symbols, text), Search Everywhere
      (2026-11-04; checks pass locally on Linux; see docs/ROADMAP.md M2;
      M2b editor polish comes before M3)
- [x] M2b: split editors, breadcrumbs, folding, brackets, EditorConfig,
      more languages, perf harness (2026-11-04; see docs/ROADMAP.md M2b and
      perf/RESULTS.md: four budgets missed on Xvfb)
- [x] M3a: LSP client, diagnostics, hover, go to definition
      (2026-11-05; checks pass locally on Linux with the real
      typescript-language-server; see docs/ROADMAP.md M3a)
- [ ] M3b: completion, signature help, usages, rename, code actions,
      formatting, oxc fast path (2026-11-05: mostly done and verified on
      Linux with the real server; open items in docs/ROADMAP.md M3b:
      trigger characters, parameter hints on `(`, oxc unused imports in the
      editor, some UI tests)
- [ ] M4: terminal, Git basics
- [ ] M5: run/debug, test runner
