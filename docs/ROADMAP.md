# Roadmap

Milestones run strictly in order; a milestone starts only after the previous
one is done (see "Definition of done" in `CLAUDE.md`). The summary and the
"done when" criteria for M0 to M5 are in
[ARCHITECTURE.md, section 8](ARCHITECTURE.md#8-milestone-roadmap). This file
holds the task-level backlog.

## M0: Foundation (done)

- [x] `docs/ARCHITECTURE.md`: crate graph, IPC contract, data flows, view-layer decision
- [x] Product name chosen: **Ferrule**
- [x] Cargo workspace with all crates, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`
- [x] Tauri v2 shell, Vite + TypeScript frontend, Bun scripts
- [x] Typed IPC with generated bindings and contract test
- [x] Licenses, NOTICE, third-party record, community docs
- [x] CI on Linux/macOS/Windows
- [x] Playwright end-to-end smoke test that launches the app

Follow-ups carried forward from M0:

- [ ] Set the maintainer contact in `CODE_OF_CONDUCT.md` before the first public release.
- [ ] Generate a full transitive third-party license list for release bundles (e.g. `cargo-about` + a JS equivalent).
- [ ] Choose the interactive UI test strategy (see ARCHITECTURE.md, section 7) at the start of M1.
- [ ] Re-check `tauri-specta`/`specta` once they leave release candidate status; they are pinned with `=`.

## M1: Editor core

- [ ] `buffer`: rope storage, change sets, versions, UTF-16 conversions
- [ ] `buffer`: undo/redo with grouping; multi-cursor and selection commands
- [ ] `vfs`: read, atomic save, watcher, recovery files (autosave of dirty buffers)
- [ ] `project`: open folder, ignore rules, lazy directory listing
- [ ] `syntax` + `lang-*`: tree-sitter incremental parsing and highlighting for JS/TS/JSX/TSX/CSS/HTML/JSON; golden tests
- [ ] `core`: `Session`, per-document actors, cancellation
- [ ] Frontend: window layout, virtualized project tree, tabs, split editors
- [ ] Frontend: CodeMirror 6 replica with the authority protocol (ARCHITECTURE.md 4.2); rebase golden tests
- [ ] EditorConfig, bracket matching, auto-indent
- [ ] `fixtures/react-ts`, Playwright tests on it
- [ ] `perf/`: typing latency and startup measurement

## M2: Navigation and search

- [ ] `indexer`: file and symbol index with on-disk cache
- [ ] `search`: fuzzy matcher, Search Everywhere providers and streaming
- [ ] Go to File / Symbol / Line, Recent Files
- [ ] Find/Replace in Files with regex and preview
- [ ] Breadcrumbs, folding
- [ ] Highlighting for SCSS/LESS/YAML/Markdown/Vue/Svelte

## M3: TypeScript intelligence

- [ ] `lsp-client`: JSON-RPC, lifecycle, cancellation, fake-server tests
- [ ] vtsls integration; server definitions in `languages/typescript/`
- [ ] Completion, parameter hints, quick docs
- [ ] Go to definition/type/implementation, find usages
- [ ] Rename, extract variable/function/component, organize imports, auto-import
- [ ] Diagnostics and Alt+Enter quick fixes; `oxc` in-process features
- [ ] ESLint and Prettier integration; Emmet
- [ ] tsconfig / package.json / workspace awareness

## M4: Terminal and Git

- [ ] `terminal`: PTY, VT parsing, terminal tool window
- [ ] `git`: status, diff, blame, log, branches, commit, conflict resolution UI

## M5: Run, debug, tests

- [ ] `run-debug`: run configurations (Node, npm scripts)
- [ ] Test runners (Jest, Vitest, Playwright) with gutter icons
- [ ] `dap-client` + CDP: debugger UI (breakpoints, stepping, variables)

## After M5 (feature scope phases 3 and 4)

- HTTP client (`.http` files), npm dependency panel, local history, TODO,
  bookmarks, problems view
- React/Next/Vue/Angular/Svelte/Tailwind support
- Settings UI, keymap editor, themes
- WASM plugin API (`wit/`, `plugin-host`)
- Auto-update, i18n, accessibility review
