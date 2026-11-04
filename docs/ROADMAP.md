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

- [x] Set the maintainer contact in `CODE_OF_CONDUCT.md` before the first public release.
- [ ] Generate a full transitive third-party license list for release bundles (e.g. `cargo-about` + a JS equivalent).
- [x] Choose the interactive UI test strategy (see ARCHITECTURE.md, section 7) at the start of M1.
- [ ] Re-check `tauri-specta`/`specta` once they leave release candidate status; they are pinned with `=`.

## M1: Editor core (done)

Plan (decided before implementation; details in ARCHITECTURE.md 3.4, 4.2, 6, 7):

- [x] `buffer`: rope (`ropey` 1.x), transactions of UTF-16 edits, versions
      (one per transaction), line-ending normalization, UTF-16 ↔ char ↔ byte
      conversions, tree-sitter `InputEdit` data per edit
- [x] `buffer`: undo/redo with typing groups (same kind, < 500 ms apart),
      selections restored on undo/redo
- [x] `buffer`: multi-cursor commands: add caret above/below, select next
      occurrence, select all occurrences
- [x] `vfs`: UTF-8 (with/without BOM) reading, CRLF/LF detection, atomic
      save, recovery store (one JSON file per document)
- [x] `project`: open folder, lazy sorted listing (directories first), hide
      `.git`, reject paths escaping the root
- [x] `syntax`: incremental tree-sitter parsing from the rope, highlight
      query → flat token spans for a range
- [x] `lang-js` (TS, TSX, JS/JSX, JSON), `lang-css`, `lang-html`: grammars,
      queries, extension mapping; golden highlight tests
- [x] `settings`: IntelliJ-style key bindings for M1 actions
- [x] `core`: `Session` with project, documents, recovery autosave
- [x] `ferrule-app`: commands; Open Folder dialog; `ferrule <path>`;
      `ferrule-headless` (feature `headless`) for UI tests
- [x] Frontend: layout (toolbar, project tree, tabs, editor, status bar),
      virtualized tree, CodeMirror 6 replica with serialized sync queue,
      highlight decorations, original token theme
- [x] `fixtures/react-ts`; Playwright `ui` tests; smoke test kept
- [x] CI: Chromium for Playwright, clippy for the `headless` feature

**Done when:**

1. `bun tauri dev`, **Open Folder…** on `fixtures/react-ts` (or
   `ferrule fixtures/react-ts`) shows the tree; folders expand lazily; the
   tree only renders visible rows.
2. Clicking files opens them in tabs; switching tabs keeps each file's text,
   cursor and highlighting; closing a tab works.
3. `.ts`, `.tsx`, `.js`, `.jsx`, `.json`, `.css`, `.html` files are
   highlighted by tree-sitter in Rust, and highlighting follows edits.
4. Typing shows a modified marker; Ctrl/Cmd+S saves atomically and keeps the
   file's line endings; the marker clears.
5. Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z undo and redo through Rust, grouping typed
   words and restoring the selection.
6. Multi-cursor: Alt+Shift+Click adds a caret; double-tap-and-hold Ctrl
   (macOS: Option) + Up/Down clones the caret (IntelliJ's gesture; Alt+Shift+Up
   is "move line" there, so it is not used); Alt+J
   selects the next occurrence; Ctrl+Alt+Shift+J (macOS: Cmd+Ctrl+G) selects
   all occurrences; typing edits at every caret.
7. Unsaved text survives a crash: a Rust integration test drops the session
   without saving and a new session restores the text from the recovery
   file; in the UI, closing and reopening a modified tab restores its text.
8. Tests: buffer (edits, conversions, undo/redo, multi-cursor), vfs, project,
   golden highlight tests per language, core session tests, IPC contract,
   Vitest for view helpers, Playwright `ui` tests for each of 1–7 on
   `fixtures/react-ts`, and the smoke test.
9. All checks pass with zero warnings.

Status: all nine points verified on 2026-11-04 (Linux). Points 1–6 were
also checked by hand in the real app (WebKitGTK, driven under Xvfb); this
found a CSP bug the Chromium tests missed, now covered by a test.

Moved out of M1 (each tracked under M2 below):

- split editors; bracket matching; language-aware auto-indent (M1 keeps
  CodeMirror's "copy previous line's indentation" on Enter, see decisions
  in CHANGELOG); EditorConfig; file watching (external changes are not
  picked up in M1); `.gitignore` greying in the tree; typing-latency
  benchmark in `perf/`; non-UTF-8 files and files over 20 MB (refused with
  an error in M1).

## M2: Navigation and search

Carried over from M1:

- [ ] Split editors
- [ ] Bracket matching; language-aware auto-indent in Rust (replace CodeMirror's indentation copy on Enter)
- [ ] EditorConfig
- [ ] File watching (`FileTreeChanged`, reload unmodified documents changed on disk)
- [ ] `.gitignore`-aware tree (greyed entries)
- [ ] `perf/` typing-latency and startup benchmarks
- [ ] Non-UTF-8 encodings; read-only paging for files over 20 MB

- [ ] Undo grouping: Enter currently starts its own undo step (it is not a "typing" event); decide whether to group it with typing like IntelliJ
- [ ] Windows: confirm the CI fix for test binaries (Common Controls manifest) on GitHub Actions

New in M2:

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
