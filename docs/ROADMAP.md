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

## M2: Search and navigation (done)

Plan (details in ARCHITECTURE.md 3.4, 4.4, 4.5):

- [x] `indexer`: project walk honouring `.gitignore` (plus `node_modules`
      always excluded), file list, TS/JS symbols from the grammars' tags
      queries, on-disk cache keyed by mtime + size, incremental updates
- [x] `search`: IntelliJ-style fuzzy matcher (camel humps, path segments),
      Search Everywhere providers (files, symbols, actions), text search
      (plain/regex, case, whole word) over files and unsaved buffers,
      replace with preview
- [x] `vfs`: file watcher (`notify`), debounced, ignoring excluded folders
- [x] Typed backend → frontend events (tauri-specta) and an event stream in
      the headless test backend
- [x] `buffer`: language-aware Enter indentation in Rust, indent-unit
      detection; Enter grouped with typing in undo; "modified" compares
      text with the saved text
- [x] Frontend: Search Everywhere (double Shift), Go to File / Symbol /
      Line, Recent Files, Find/Replace in Files popups (virtualized lists),
      reload/keep bar for files changed on disk
- [x] Windows: confirm the M1 CI fix (Common Controls manifest) on GitHub
      Actions

**Done when:**

1. Double Shift opens Search Everywhere. Typing `cnt` lists
   `src/components/Counter.tsx` and the symbol `Counter`; Enter opens the
   file at the symbol. Tabs All / Files / Symbols / Actions; actions run.
2. Go to File (Ctrl+Shift+N, macOS Cmd+Shift+O), Go to Symbol
   (Ctrl+Alt+Shift+N, macOS Cmd+Alt+O), Go to Line (Ctrl+G, macOS Cmd+L)
   and Recent Files (Ctrl+E, macOS Cmd+E) open their popups and navigate.
3. The index skips `.gitignore`d paths and `node_modules`, is reused from
   the on-disk cache on reopen (unchanged files are not re-parsed, asserted
   by a Rust test), and follows files changed on disk.
4. Find in Files (Ctrl+Shift+F, macOS Cmd+Shift+F): plain or regex, case
   sensitive, whole word; results show the line with the match; Enter or
   click opens the file at the match; unsaved buffer text is searched, not
   the file on disk.
5. Replace in Files (Ctrl+Shift+R, macOS Cmd+Shift+R): preview shows old →
   new per match; Replace All edits open documents through the buffer
   (undoable) and other files with atomic saves; an invalid regex shows an
   error instead of results.
6. Files changed outside the editor: the tree shows created/deleted files;
   an open file without unsaved changes reloads; an open file with unsaved
   changes shows a bar with **Reload** and **Keep my changes**.
7. Enter is computed in Rust: keeps the line's indentation, indents after
   `{ ( [` and opening markup tags, splits `{|}` onto three lines; Tab
   inserts the file's detected indent unit.
8. Enter and the surrounding typing undo as one step.
9. Undoing back to the saved text clears the modified marker (and redoing
   sets it again).
10. Tests: Rust unit/integration tests for the matcher, indexer and cache,
    text search/replace, watcher-driven updates, indentation, undo grouping
    and the modified state; Playwright `ui` tests for 1, 2, 4, 5, 6, 7, 8
    and 9 on `fixtures/react-ts`; all checks pass with zero warnings.

Status: points 1–10 verified on 2026-11-04 (Linux; Playwright `ui` in
Chromium plus a manual check in the real app). The Windows CI fix from M1
was confirmed on GitHub Actions (test binaries now start); the next Windows
failure (CRLF checkouts breaking golden tests) is fixed by `.gitattributes`
and is confirmed only once pushed.

Follow-ups found in M2:

- [ ] Search Everywhere: stream results per provider over a channel once the
      test bridge supports channels (now request/response).
- [ ] Files changed while the index is being built are only picked up on
      the next change or reopen.
- [ ] Select Next Occurrence is case-sensitive plain matching (no whole-word
      mode yet).

## M2b: Editor polish (done)

Plan:

- [x] `syntax`: language injections (HTML `<script>`/`<style>`, Vue and
      Svelte blocks, Markdown inline text and fenced code), folding ranges,
      bracket pairs and breadcrumbs from the syntax tree
- [x] New languages: SCSS, LESS (`lang-css`), Vue, Svelte (`lang-html`),
      YAML (`lang-yaml`), Markdown (`lang-markdown`); golden tests
- [x] Highlighting fixes: capitalized function names use the function
      color; CSS custom properties (`--gap`) get a variable color
- [x] `settings`: EditorConfig (`indent_style`, `indent_size`, `tab_width`,
      `end_of_line`, `trim_trailing_whitespace`, `insert_final_newline`,
      `charset`), nearest file wins, `root = true` stops the search
- [x] `project`: listings mark `.gitignore`d entries (and `node_modules`)
- [x] Frontend: split editors (two groups, the same file in both), code
      folding, bracket matching, auto-closing brackets and quotes,
      breadcrumbs, greyed ignored entries
- [x] `perf/`: external benchmark harness on a generated ~1000-file project
      (Linux/X11 first)

**Done when:**

1. **Split editors.** "Split Right" (tab button, Search Everywhere action)
   opens the current file in a second group; both groups have their own
   tabs; the same file can be open in both and edits in one appear in the
   other immediately; closing a group's last tab removes the group.
2. **Breadcrumbs.** A bar under the editor shows the structural path at the
   caret (e.g. `App › main › Counter` in a TSX file, `.card › h1`
   in CSS, `scripts › build` in JSON/YAML); clicking a crumb moves the caret
   there.
3. **Folding.** Gutter markers on multi-line blocks (braces, brackets,
   multi-line comments, markup elements, YAML/Markdown sections); click or
   Ctrl+Minus / Ctrl+Plus (macOS Cmd) folds/unfolds; Ctrl+Shift+Minus /
   Ctrl+Shift+Plus folds/unfolds all. Ranges come from Rust.
4. **Brackets.** The bracket pair at the caret is highlighted (from the
   syntax tree, so brackets in strings and comments are ignored);
   Ctrl+Shift+M (macOS Cmd+Ctrl+M) jumps to the matching bracket. Typing
   `(`, `[`, `{`, quotes (and backticks in JS/TS) inserts the closing
   character; typing it again steps over; Backspace in an empty pair
   deletes both.
5. **EditorConfig.** With a `.editorconfig`, Tab and Enter use its
   indentation, tab width follows `tab_width`, and saving applies
   `trim_trailing_whitespace`, `insert_final_newline` and `end_of_line`
   (as one undoable step, visible in the editor).
6. **Ignored files.** `.gitignore`d entries and `node_modules` are shown
   greyed in the tree (still openable).
7. **Highlighting.** `.scss`, `.less`, `.yaml`/`.yml`, `.md`, `.vue` and
   `.svelte` are highlighted, including `<script>`/`<style>` blocks in Vue,
   Svelte and HTML and fenced code in Markdown; the two quirks are fixed;
   golden tests cover every language.
8. **Benchmarks.** `cargo run -p ferrule-perf --release -- …` generates a
   ~1000-file project, launches the release app as a separate process, and
   measures from the outside: time to a window showing the project (screen
   pixels), ready time (process-tree CPU settled), memory (PSS of the whole
   process tree), idle CPU, typing latency (key injected with XTest until
   the glyph appears on screen) and quit time. Results with machine
   details are recorded in `perf/RESULTS.md`; only measured numbers.
9. **Tests.** Rust tests for injections, folding, brackets, breadcrumbs,
   EditorConfig, ignored entries and the new languages (golden); Playwright
   `ui` tests for 1–7; all checks pass with zero warnings.

Status: points 1–9 verified on 2026-11-04 (Linux; Playwright `ui` in
Chromium). The benchmark harness works; its first measurements miss four
budgets (see `perf/RESULTS.md` and the backlog).

Not in M2b (moved to the backlog below): non-UTF-8 encodings and
read-only paging for files over 20 MB.

## Backlog (unscheduled)

- [ ] Non-UTF-8 encodings; read-only paging for files over 20 MB (from M1)
- [ ] `perf/` on macOS and Windows; comparison runs against other IDEs with
      the same method
- [ ] Search-latency benchmark
- [ ] Performance (from `perf/RESULTS.md`, M2b run on Xvfb): idle CPU with an
      open editor (21 % of a core; ~0 % without one), typing latency
      (53 ms median), memory (308 MiB PSS) and "ready" miss the budget.
      Measure on a GPU desktop, then reduce the editor's idle repaint cost
      (caret blink), typing-path work and memory.

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
