# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added (M3b: completion, refactoring, code actions)

- Completion popup: opened by typing a word or Ctrl+Space (not yet by
  trigger characters such as `.`); fuzzy-filtered and ranked in Rust; lazy detail
  loading (`completionItem/resolve`); snippet expansion with tab stops and
  `additionalTextEdits` (auto-import) as one undoable step.
- Parameter hints (Ctrl+P on every OS): signature popup with the active
  parameter highlighted (not yet opened by typing `(` or `,`).
- Find Usages (Alt+F7): references panel grouped by file; click to navigate.
- Rename (Shift+F6): preview dialog showing affected files and occurrence
  counts; open documents edited as one undo step, closed files rewritten
  atomically; files changed on disk since the preview are skipped.
- Code Actions (Alt+Enter): quick fixes and refactorings at the caret,
  including "Add missing import" and "Organize Imports".
- Reformat Code (Ctrl+Alt+L, macOS Cmd+Option+L) via the language server.
- Optimize Imports (Ctrl+Alt+O, the server's organize imports) and Sort
  Imports (Ctrl+Alt+Shift+O, in-process with `oxc`).
- Typing-path timing spans (`FERRULE_TYPING_TRACE`) and a before/after
  measurement in `perf/RESULTS.md`.
- Read-only tabs for files outside the project (`lib.dom.d.ts`,
  `node_modules`): lock marker in the tab, typing disabled, hover and
  navigation still work.
- LSP infrastructure: token cancellation per feature, completion/refactoring
  and code action stores, `workspace/applyEdit` capture mode for server
  edits during commands, `FileEdits` with snapshot/preview/apply and skipped
  file tracking.

### Fixed (M3b)

- Completion ranking prefers exact label matches over fuzzy matches.
- A completion item whose resolve failed or was answered while the server
  was loading was cached unresolved, so accepting it lost its auto-import.

### Added (M3a: language server, diagnostics, hover, navigation)

- TypeScript/JavaScript intelligence through `typescript-language-server`,
  one per project, started when the first JS/TS file opens. Found in the
  project's `node_modules/.bin`, the user setting `typescript.serverPath`
  (`settings.json` in the app config directory), or `PATH`; a missing server
  or Node.js is shown in the status bar and the editor keeps working.
- `lsp-client`: JSON-RPC over stdio with request ids and cancellation
  (`$/cancelRequest`), initialize/shutdown, UTF-16 positions, restart with
  backoff after a crash, stderr to the log, debounced incremental
  `didChange` driven by the Rust buffer.
- Diagnostics as squiggles (error, warning, info, hint) with the message on
  hover; Problems tool window (Alt+6, macOS Cmd+6) grouped by file; error
  and warning counts in the status bar.
- Quick Documentation (Ctrl+Q, macOS F1) and documentation on mouse hover,
  rendered from Markdown and sanitized in Rust, with highlighted code.
- Go to Declaration (Ctrl/Cmd+B, Ctrl/Cmd+Click), Go to Type Declaration
  (Ctrl/Cmd+Shift+B) and Go to Implementation (Ctrl/Cmd+Alt+B), with a
  chooser for several targets.
- Server status in the status bar (starting, indexing, ready, restarting,
  error, not found) with details in the tooltip (server, TypeScript
  version, tsconfig files, workspace packages); "Restart TypeScript
  Server" action.
- Detection of `tsconfig`/`jsconfig` files and npm, Yarn, pnpm and Bun
  workspace packages.

### Fixed (M3a)

- Windows build: `tree-sitter-scss` passed a GCC-only flag to MSVC; the
  crate is vendored with the upstream fix (`third_party/tree-sitter-scss`).

### Added (M2b: editor polish)

- Split editors: "Split Right" opens the current file in a second group;
  the same file can be edited in both, kept in sync through one replica.
- Breadcrumbs under the editor (named scopes at the caret; click to jump).
- Code folding with gutter markers and Ctrl/Cmd+Minus/Plus (Shift for all);
  ranges come from tree-sitter in Rust.
- Bracket matching from the syntax tree, Move Caret to Matching Brace
  (Ctrl+Shift+M, macOS Cmd+Ctrl+M), auto-closing brackets and quotes.
- EditorConfig: indentation, tab width, trailing whitespace, final newline,
  line endings and UTF-8 BOM.
- `.gitignore`d entries and `node_modules` shown greyed in the tree.
- Highlighting for SCSS, LESS, YAML, Markdown, Vue and Svelte, and for
  embedded `<script>`/`<style>` blocks (HTML, Vue, Svelte) and fenced code
  in Markdown, via injections; golden tests per language.
- `ferrule <folder> [file]` opens a file at startup.
- `ferrule-perf`: external benchmark harness (Linux/X11) for startup, ready
  time, memory, idle CPU, typing latency and quit time; results in
  `perf/RESULTS.md`.

### Fixed (M2b)

- Capitalized function names (React components) were colored as types.
- CSS custom properties (`--gap`) were plain text; SCSS/LESS variables are
  colored too.

### Added (M2: search and navigation)

- Search Everywhere (double Shift) with All / Files / Symbols / Actions tabs;
  Go to File, Go to Symbol and Find Action open it on their tab.
- Go to Line, Recent Files (persisted per project).
- Find in Files and Replace in Files: plain or regex, match case, whole
  word, replacement preview with `$1` expansion; unsaved buffers are
  searched; Replace All edits open documents through the normal (undoable)
  edit path and rewrites other files atomically, keeping line endings.
- `ferrule-indexer`: `.gitignore`-aware project index (always skipping
  `.git` and `node_modules`), TS/JS symbols from the grammars' tags
  queries, on-disk cache keyed by mtime and size, incremental updates.
- `ferrule-search`: IntelliJ-style fuzzy matcher and text search/replace.
- File watching (`notify`): the tree follows created/deleted files,
  unmodified tabs reload, modified tabs offer Reload / Keep my changes.
- Typed backend events (`fs-changed`, `index-status`); the headless test
  backend serves them through a long-poll endpoint.
- Enter is computed in Rust with language-aware indentation (brackets,
  markup tags, `{|}` splitting); Tab inserts the file's detected indent unit.

### Changed (M2)

- Enter and the surrounding typing undo as one step.
- The modified marker compares the text with the last saved text, so
  undoing back to it clears the marker.
- IPC protocol version 3. `deny.toml` allows ISC (for `inotify`).
- Breadcrumbs, folding, split editors and other editor polish moved to the
  new M2b milestone (see `docs/ROADMAP.md`).

### Fixed (M2)

- Windows CI: golden tests failed because Git checked files out with CRLF;
  `.gitattributes` now enforces LF for text files on every OS.

### Added (M1: editor core)

- Project view: Open Folder (native dialog) or `ferrule <folder>`; lazy,
  virtualized project tree (directories first, `.git` hidden) with keyboard
  navigation.
- Editor tabs with one CodeMirror 6 view each, used as a view-only replica of
  the Rust buffer through a collab-style versioned protocol.
- `ferrule-buffer`: rope buffer with UTF-16 transactions, versions, undo/redo
  with typing groups, selection restore and multi-cursor commands (clone
  caret, select next/all occurrences).
- `ferrule-vfs`: UTF-8/BOM/CRLF-aware loading, atomic saves that keep line
  endings and permissions, crash-recovery files (written 1 s after the last
  edit, restored on reopen or after a crash).
- `ferrule-syntax` + `lang-js`/`lang-css`/`lang-html`: incremental
  tree-sitter highlighting for TS, TSX, JS/JSX, JSON, CSS and HTML, with
  golden tests; original token color theme (dark and light).
- `ferrule-settings`: IntelliJ-style key bindings (save, undo/redo, clone
  caret via double-tap Ctrl/Option, Alt+J, Ctrl+Alt+Shift+J, close tab).
- `ferrule-core` `Session` and 14 typed IPC commands.
- Tests: Rust unit/integration tests for every crate above, Vitest for view
  helpers, and Playwright `ui` tests (Chromium + Vite + `ferrule-headless`,
  a mock-runtime build of the real command handler) on `fixtures/react-ts`.

### Fixed

- Windows CI: test binaries linking Tauri failed to start
  (`STATUS_ENTRYPOINT_NOT_FOUND`); the Common Controls manifest is now
  embedded into every binary of `ferrule-app`.
- Production CSP blocked CodeMirror's injected styles (broken editor layout
  in the real app); `style-src` now allows inline styles.

### Changed

- IPC protocol version 2. Repository URL and code-of-conduct contact set.
- CI actions moved off the deprecated Node 20 versions; CI installs
  Chromium for the `ui` tests and lints the `headless` feature.

### Added (M0: foundation)

- M0 foundation: architecture document (`docs/ARCHITECTURE.md`) and roadmap.
- Cargo workspace with all domain crates (empty), `ferrule-ipc` DTOs and
  `ferrule-core` app metadata.
- Tauri v2 shell (`src-tauri/`) with typed IPC via tauri-specta: commands
  `app_info` and `report_frontend_ready`; TypeScript bindings generated by
  `bun run gen:ipc`.
- Vite + TypeScript frontend that renders the product name and version from
  the backend, with an original dark/light theme and app icon.
- Checks: rustfmt, clippy (`-D warnings`), cargo test (including the IPC
  contract test), cargo-deny, ESLint, Prettier, `tsc`, Vitest, and a
  Playwright end-to-end smoke test that launches the real app.
- GitHub Actions CI for Linux, macOS and Windows.
- Apache-2.0 license, NOTICE, third-party license record and community docs.
