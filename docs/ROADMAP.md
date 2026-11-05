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

M3 runs in two parts: M3a (client, diagnostics, hover, navigation) and M3b
(completion, signature help, usages, rename, code actions, formatting, oxc
fast path). Design: [ARCHITECTURE.md, 4.7](ARCHITECTURE.md#47-language-servers-m3).

Before M3a: the M2b Windows CI job failed (`tree-sitter-scss` passes a GCC
flag to MSVC); fixed by vendoring the crate with the upstream build-script
fix (`third_party/tree-sitter-scss`).

### M3a: LSP client, diagnostics, hover, definition (done)

Plan:

- [x] `lsp-client`: Content-Length framing over any async stream, JSON-RPC
      requests with ids, `$/cancelRequest` driven by a `CancellationToken`,
      server→client requests answered, initialize/initialized/shutdown/exit,
      capability negotiation (UTF-16 positions), stderr to `tracing`,
      supervisor that restarts a crashed server with backoff, debounced
      incremental `didChange`, executable discovery
- [x] `fake-lsp-server` (test-only binary in `lsp-client`): scripted server
      that rebuilds the text from incremental changes, honours cancellation,
      publishes diagnostics and can crash on request
- [x] `buffer`: every applied edit also yields an LSP-shaped change (line +
      UTF-16 column, in application order)
- [x] `project`: tsconfig/jsconfig files and npm/pnpm/yarn workspace packages
- [x] `lang-js`: the TypeScript server definition (program, args, language
      ids per file type)
- [x] `settings`: user settings file with `typescript.serverPath`
- [x] `core`: one TypeScript server per project, started on the first JS/TS
      document, document sync from the Rust buffer, diagnostics store, hover
      (Markdown rendered and sanitized in Rust, code highlighted by
      tree-sitter), definition/type definition/implementation, status events
- [x] Frontend: squiggles with hover messages, Problems tool window, Quick
      Documentation popup and mouse hover, Go to Definition (Ctrl+B,
      Ctrl+Click), Type Definition, Implementation, chooser popup, status
      bar server state and problem counts
- [x] CI installs Node, `typescript-language-server` and `typescript` so the
      real-server tests run on all three OSes

**Done when:**

1. **Discovery.** The server is found in this order: the project's
   `node_modules/.bin/typescript-language-server`, the user setting
   `typescript.serverPath` in `settings.json` (app config directory), then
   `PATH`. A server that is a Node script additionally needs `node` on
   `PATH`. If the server or Node is missing, the status bar says so and how
   to fix it, and the editor stays fully usable: nothing blocks or crashes.
2. **Lifecycle.** Opening the first `.ts/.tsx/.js/.jsx/.mts/.cts/.mjs/.cjs`
   file starts one server for the project; the status bar shows
   _starting → indexing (while the server reports progress) → ready_. If the
   server process dies, it is restarted with backoff (0.5 s, 1 s, 2 s, …,
   at most 5 attempts in a row) and every open document is sent again;
   after that the state is _error_ and the action "Restart TypeScript
   Server" starts it again. Opening another project or quitting shuts the
   server down.
3. **Sync.** The server sees exactly the Rust buffer: `didOpen` on open,
   debounced incremental `didChange` (also for undo/redo, Enter, reload,
   save edits and Replace in Files), `didSave`, `didClose`. Requests flush
   pending changes first. A test proves that the text a server rebuilds from
   our changes equals the buffer after edits with emoji and CJK text.
4. **Diagnostics.** Errors, warnings, info and hints from the server show as
   squiggles (four styles); hovering a squiggle shows the message. The
   Problems tool window (Alt+6, macOS Cmd+6, or a click on the status bar
   counts) lists problems grouped by file with counts; clicking one opens
   the file at the location. The status bar shows error and warning counts.
5. **Quick Documentation.** Ctrl+Q (macOS F1) at the caret, or resting the
   mouse on a symbol, shows the hover as rendered Markdown with highlighted
   code; raw HTML and non-http(s) links in the server's Markdown are not
   rendered. A newer hover cancels the older one in the server.
6. **Navigation.** Go to Definition (Ctrl+B / Cmd+B and Ctrl/Cmd+Click), Go
   to Type Definition (Ctrl+Shift+B / Cmd+Shift+B) and Go to Implementation
   (Ctrl+Alt+B / Cmd+Alt+B) open the target file at the target position;
   several targets show a chooser popup; none shows a message.
7. **Workspace.** The project's `tsconfig.json`/`jsconfig.json` files and
   npm/yarn (`package.json` `workspaces`) or pnpm (`pnpm-workspace.yaml`)
   packages are detected and shown in the server status tooltip; the
   server's workspace folders are the project root plus the workspace
   packages.
8. **Tests.** `lsp-client` unit tests (framing, ids, cancellation, server
   requests, debounce) and tests with `fake-lsp-server` (incremental sync
   with emoji/CJK, crash and restart, shutdown); position conversion tests;
   core integration tests against the real `typescript-language-server`
   (diagnostics, hover, cross-file definition, edits with emoji/CJK) that
   print a reason and skip when it is not installed; Playwright `ui` tests
   on `fixtures/react-ts` for 4, 5 and 6 with the real server and for 1
   (server missing); all checks pass with zero warnings.

Status: points 1–8 verified on 2026-11-05 on Linux, with
typescript-language-server 6.0.3 and TypeScript 6.0.3 (Bun global install),
Node 24. The real-server Rust and Playwright tests ran against that server;
CI installs the same versions. Not yet confirmed on macOS/Windows CI (needs a
push).

Known gaps found in M3a (tracked here):

- [ ] Opening navigation targets outside the project (e.g. `lib.dom.d.ts`)
      (M3b, point 8)
- [ ] Links in hover documentation are shown but do not open the browser
- [ ] `fixtures/react-ts` has no `node_modules`, so React imports show
      "Cannot find module" errors until `bun install` is run in it (M3b,
      point 10)
- [ ] Diagnostics of files that are not open are not requested
      (pull diagnostics / project-wide check)

### M3b: completion, refactoring, code actions (done)

**Done when:**

1. **Completion.** Typing a trigger character (`.`, `"`, `'`, `/`, `@`, `<`)
   or Ctrl+Space opens a virtualized popup; items are fuzzy-filtered and
   ranked in Rust while typing; details and docs load lazily
   (`completionItem/resolve`) for the selected item; accepting inserts
   snippets with tab stops and applies `additionalTextEdits` (auto-import)
   as one undoable step; a newer completion request cancels the older one.
2. **Parameter hints.** Ctrl+P (macOS Cmd+P) and typing `(` or `,` show the
   signature with the active parameter highlighted.
3. **Find Usages.** Alt+F7 (macOS Option+F7) lists references grouped by
   file in a results panel; clicking one opens the location.
4. **Rename.** Shift+F6 renames project-wide after a preview (files and
   occurrence counts); each open document changes as one undo step; closed
   files are rewritten atomically; a file that changed on disk meanwhile is
   not overwritten.
5. **Code actions.** Alt+Enter shows quick fixes and refactorings at the
   caret; Organize Imports (Ctrl+Alt+O, macOS Ctrl+Option+O) and "Add
   missing import" work.
6. **Formatting.** Reformat Code (Ctrl+Alt+L, macOS Cmd+Option+L) uses the
   server's document formatting as one undo step (Prettier is a follow-up).
7. **oxc fast path.** Unused imports are greyed out, and imports can be
   sorted instantly in-process with `oxc`, without waiting for the server.
8. **Read-only library files.** Definitions and usages in files outside
   the project (`lib.dom.d.ts`) or inside `node_modules` open in a read-only
   editor (lock marker in the tab; typing does nothing; hover and navigation
   work there too).
9. **Typing latency.** Timing spans on the typing path (key → local echo
   frame → Rust buffer → highlights → frame) in the view and in Rust,
   collected by `ferrule-perf`. A baseline before M3b and a run with
   completion, signature help and diagnostics active are recorded in
   `perf/RESULTS.md`; M3b must not make the median worse.
10. **Fixture.** `fixtures/react-ts` has `@types/react` and
    `@types/react-dom` as devDependencies with a committed `bun.lock`;
    JSX type-checks without errors; CI installs them.
11. **Tests.** Mock-server tests for completion, cancellation, rename and
    workspace edits; golden tests for refactor edits (snippet expansion,
    workspace edits, import sorting); real-server integration tests;
    Playwright `ui` tests for 1–8; all checks pass with zero warnings.

Status (2026-11-05, Linux, typescript-language-server 6.0.3): **done**,
except the open items below. "Real server" means
`crates/core/tests/lsp_real_m3b.rs`, "mock" means `fake-lsp-server` tests in
`crates/core/tests/lsp_fake.rs`, "UI" means `e2e/ui/lsp.spec.ts`.

- Point 1, completion: trigger characters (`.` real server, mock, UI),
  Ctrl+Space and word prefixes (UI), ranking and filtering in Rust (real
  server, unit), lazy details (real server), auto-import on accept (real
  server, UI: the import is merged into the existing one), a newer request
  cancels the older one (mock). Snippets: parsing and tab-stop positions
  are unit-tested only.
- Point 2, parameter hints: `(` and `,` open them (real server, UI); Ctrl+P,
  macOS Cmd+P.
- Points 3 and 4, Find Usages and Rename: real server, mock (emoji/CJK
  positions across two documents) and UI; closed file changed after the
  preview is skipped and one undo step per open document (real server).
- Point 5, code actions: Alt+Enter adds a missing import (real server and
  UI, through a server command and `workspace/applyEdit`); Organize Imports
  (real server).
- Point 6, formatting: real server and UI (one undo step).
- Point 7, oxc: unused imports shown faded without any server (UI, core
  test) and Sort Imports (golden tests, real session).
- Point 8, read-only library files: `lib.dom.d.ts` (real server) and
  `@types/react` (UI: RO tab, typing ignored).
- Point 9, typing latency: before and after recorded, no measurable
  regression (`perf/RESULTS.md`); the 8 ms budget is still missed on Xvfb.
- Point 10, fixture types: done.
- Point 11, tests: as listed above; all checks pass with zero warnings.

Open M3b items:

- [ ] Snippet tab-stop navigation in the editor verified end to end (the
      TypeScript server rarely returns snippets with the current settings)
- [ ] macOS/Windows CI confirmation (needs a push)

### Later in the TypeScript track

- [ ] Extract variable/function/component
- [ ] ESLint and Prettier integration; Emmet

## M4: Terminal and Git

- [ ] `terminal`: PTY, VT parsing, terminal tool window
- [ ] `git`: status, diff, blame, log, branches, commit, conflict resolution UI

## M5: Run, debug, tests

- [ ] `run-debug`: run configurations (Node, npm scripts)
- [ ] Test runners (Jest, Vitest, Playwright) with gutter icons
- [ ] `dap-client` + CDP: debugger UI (breakpoints, stepping, variables)

## M6: UI design (not started)

Spec: [M6-DESIGN.md](M6-DESIGN.md). Start only when the maintainer says
"start M6"; Part 2 only after "continue with M6 part 2". Until then features
are built functionally, using CSS variables where easy.

**Done when** (from the spec):

1. **Tokens.** All colors and sizes are CSS custom properties in
   `src/styles/tokens.css` (dark values from the spec, a light theme with the
   same structure and WCAG AA contrast); a 4px spacing scale, radii, shadows;
   Inter and a permissive monospace bundled locally with their licenses. No
   hardcoded colors or sizes in components.
2. **Primitives.** Shared button, icon button, tab, tree row, list row,
   popup/menu, tooltip, panel header, status bar item, input and dialog
   (title, content, OK/Cancel/Apply, focus trap, Esc/Enter).
3. **Layout.** Dark frame with rounded panels and gaps; left and bottom tool
   window stripes; resizable panels with persisted sizes; minimum window
   size; no clipped text at the window edge.
4. **Top bar.** Main menu, project name with an initials badge, search and
   settings icons; nothing for features that do not exist.
5. **Project tree.** Root row with muted path, chevrons, folder and
   file-type icons (Lucide, ISC, inline SVG, 16px; our own file-type
   badges), indent guides, selected/hover states, greyed ignored entries,
   "library root" next to `node_modules`.
6. **Editor chrome.** Tabs with file icon, close on hover, modified dot,
   overflow handling, a real split icon; gutter with fold chevrons,
   current-line highlight, indent guides; inspection widget with LSP error and
   warning counts; scrollbar diagnostic markers.
7. **Status bar.** Breadcrumbs on the left (replacing the breadcrumb bar);
   line:col, line separator, encoding, indent and TypeScript server state on
   the right.
8. **Empty editor.** Centered hint list with the real shortcuts.
9. **Menus.** File, Edit, View, Navigate, Code, Refactor, Run, Tools, Git,
   Window, Help with submenus, separators, shortcuts and mnemonics; only real
   actions enabled, others hidden or disabled with a clear label.
10. **Popups.** All popups and panels (Search Everywhere, Go to,
    Find/Replace, hover and diagnostics, completion, signature help, rename,
    usages, Alt+Enter, chooser, reload bar, M4 terminal and Git panels) use
    the shared style.
11. **Theme switch.** System / dark / light.
12. **Performance (Part 3).** Typing latency re-measured after the redesign
    (on a real display if possible), profiled, the biggest costs fixed, and
    `perf/RESULTS.md` updated with before/after.
13. **Verification.** Playwright screenshots of every key screen in dark and
    light (Chromium against the headless backend), compared against the
    references outside the repo (`~/Desktop/design-refs/`, never copied in),
    differences listed and fixed; a Playwright check that fails on clipped or
    overflowing text; Ferrule's own screenshots in `docs/media/`.
14. **Legal.** No JetBrains/WebStorm/IntelliJ/"Islands" names, logos, icons,
    assets or reference color values; every new license recorded in
    `THIRD_PARTY_LICENSES.md`. All checks pass with zero warnings.

Part 2 (settings dialog, notification toasts, welcome screen) is planned in
the spec and gets its own done-when when it starts.

## After M5 (feature scope phases 3 and 4)

- HTTP client (`.http` files), npm dependency panel, local history, TODO,
  bookmarks, problems view
- React/Next/Vue/Angular/Svelte/Tailwind support
- Settings UI, keymap editor, themes
- WASM plugin API (`wit/`, `plugin-host`)
- Auto-update, i18n, accessibility review
