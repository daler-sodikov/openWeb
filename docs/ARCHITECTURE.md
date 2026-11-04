# Ferrule architecture

Ferrule is an open-source IDE for JavaScript/TypeScript web development with a
WebStorm-like workflow and an IntelliJ-style keymap. Not affiliated with
JetBrains.

This document is the source of truth for how the system is put together. It is
updated before each milestone (see "Milestone roadmap" below and
[ROADMAP.md](ROADMAP.md) for the detailed backlog).

## 1. Process and layer model

```
┌──────────────────────────── one OS process ─────────────────────────────┐
│                                                                          │
│  WebView (src/, TypeScript)          Rust (src-tauri/ + crates/)         │
│  ┌──────────────────────────┐  IPC   ┌──────────────┐   ┌─────────────┐  │
│  │ view layer only:         │◄──────►│ ferrule-app  │──►│ ferrule-core│  │
│  │ CodeMirror 6 view,       │ typed  │ (src-tauri)  │   │ (sessions,  │  │
│  │ tool windows, popups,    │ cmds + │ thin command │   │  services,  │  │
│  │ virtualized lists        │ events │ layer        │   │  event bus) │  │
│  └──────────────────────────┘        └──────────────┘   └──────┬──────┘  │
│                                                                │         │
│                         domain crates: buffer, vfs, project, indexer,    │
│                         search, lang-*, lsp-client, git, terminal, ...   │
└─────────────────────────────────────┬────────────────────────────────────┘
                                      │ stdio (JSON-RPC)
                     external processes: vtsls / typescript-language-server,
                     css/html/json servers, eslint, tailwind, DAP adapters,
                     Node, shells (PTY)
```

- **Frontend (`src/`)** renders state and forwards user intent. It has no
  file IO, no language analysis and no authority over document text.
- **`src-tauri/` (crate `ferrule-app`)** declares `#[tauri::command]`
  functions, window setup and the typed-binding export. Each command is a
  few lines: decode arguments, call `ferrule-core`, return the result.
- **`ferrule-core`** owns the running application state (open projects,
  documents, language servers) and wires the domain crates together.
- **Domain crates** are independent libraries with small public APIs and
  their own tests. None of them depend on Tauri.

## 2. Crate graph

All crates live in `crates/<name>` and are named `ferrule-<name>` inside the
workspace. The graph must stay acyclic; if two crates need each other,
extract a third. Arrows point from a crate to its dependencies.

```
ferrule-app (src-tauri) ──► core ──► ipc
                             │
   ┌──────────┬───────────┬──┴───────┬──────────┬────────────┬────────────┐
   ▼          ▼           ▼          ▼          ▼            ▼            ▼
 search    run-debug   terminal     git    plugin-host   lsp-client   lang-js
   │        │    │                   │          │                     lang-css
   ▼        ▼    ▼                   ▼          ▼                     lang-html
 indexer  dap-  project ──► vfs ◄────┘       settings                    │
   │      client   │         │                                           ▼
   │               ├──► settings                                       syntax
   │               ▼         ▼                                           │
   ├──► lang-*   buffer ◄────┘◄──────────────────────────────────────────┘
   └──► project, vfs
```

| Crate         | Owns                                                                                                                                                                                         | Internal deps                       |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| `buffer`      | Rope text storage, edits and change sets, versions, selections and multi-cursor commands, undo/redo history, line index, UTF-8/UTF-16/line-col conversions. Pure, no IO.                     | none                                |
| `vfs`         | File system access, file ids, path interning, watching, encoding and line-ending detection, atomic save, crash-recovery files (autosave of dirty buffers).                                   | `buffer`                            |
| `settings`    | Layered settings (defaults, user, project, `.editorconfig`), keymap presets (IntelliJ-style), schema and validation.                                                                         | none                                |
| `project`     | Project model: roots, ignore rules, `package.json`/`tsconfig.json` discovery, npm/pnpm/yarn workspaces, open-document registry, recent files.                                                | `vfs`, `buffer`, `settings`         |
| `syntax`      | Shared tree-sitter machinery: incremental parsing, highlight captures to token spans, folding ranges, bracket pairs, language registry.                                                      | `buffer`                            |
| `lang-js`     | JS/TS/JSX/TSX grammars and queries, `oxc` integration (parse, semantics, import resolution, unused imports, quick fixes).                                                                    | `syntax`                            |
| `lang-css`    | CSS/SCSS/LESS grammars, queries and in-process features.                                                                                                                                     | `syntax`                            |
| `lang-html`   | HTML (later Vue/Svelte embedding) grammars, queries and in-process features.                                                                                                                 | `syntax`                            |
| `indexer`     | Incremental project indexing: file list, symbols, on-disk cache, invalidation on file change.                                                                                                | `project`, `vfs`, `lang-*`          |
| `search`      | Fuzzy matching for files/symbols/actions, regex text search and replace across files, Search Everywhere aggregation and ranking.                                                             | `indexer`, `vfs`                    |
| `lsp-client`  | Generic LSP client: process lifecycle, JSON-RPC framing, requests with cancellation, notifications, debounced `didChange`, capability negotiation. Knows nothing about specific servers.     | none                                |
| `dap-client`  | Generic Debug Adapter Protocol client mirroring `lsp-client`, plus a Chrome DevTools Protocol client for browser/Node debugging.                                                             | none                                |
| `git`         | Status, diff, blame, log, branches, commit, conflicts. Read paths via `gix`; write paths may shell out to the `git` binary.                                                                  | `vfs`                               |
| `terminal`    | PTY processes, VT parsing, terminal grid state and incremental updates for the view.                                                                                                         | none                                |
| `run-debug`   | Run configurations (Node, npm scripts, Jest, Vitest, Playwright), process supervision, test-result parsing, debug sessions.                                                                  | `dap-client`, `project`, `settings` |
| `plugin-host` | `wasmtime` component-model host for plugins built against `wit/`.                                                                                                                            | `settings`                          |
| `ipc`         | Serializable DTOs shared with the frontend (`specta::Type` derives), error type, protocol version. No logic.                                                                                 | none                                |
| `core`        | `Session`: owns open projects and documents, starts language servers, routes events, holds cancellation tokens, maps domain types to `ipc` DTOs. The only crate that knows about all others. | all of the above                    |

Rules that follow from the graph:

- `ferrule-app` depends only on `core` and `ipc` (plus Tauri). Domain types
  are mapped to `ipc` DTOs in `core`, so domain crates carry no UI concerns.
- `lsp-client`, `dap-client` and `terminal` have no internal deps, so they
  are tested against fake servers/processes in isolation.
- Language-specific knowledge lives in `lang-*` and `languages/` (grammars,
  queries, server definitions), never in `core`.

## 3. Typed IPC contract

### 3.1 Mechanism

- **Commands** (frontend → Rust, request/response): Tauri v2 `invoke`.
  Every command is `async`, returns `Result<T, IpcError>`, and delegates to
  the tokio runtime; nothing runs on the main thread.
- **Events** (Rust → frontend, broadcast): typed events for state any view
  may care about (document changed, diagnostics published, file tree
  changed, index progress).
- **Channels** (Rust → frontend, per-request stream):
  `tauri::ipc::Channel<T>` for ordered, high-volume streams tied to one
  request: search results, terminal output, log tails. A superseding
  request cancels the previous stream's `CancellationToken`.

### 3.2 Type generation

- DTOs are Rust types in `ferrule-ipc` deriving `serde` + `specta::Type`.
- Commands (and later events) are registered with `tauri-specta`, which
  generates `src/ipc/bindings.ts` containing TypeScript types **and** typed
  command wrappers (`commands.appInfo()`). No command name or payload type
  is hand-written in TypeScript.
- `bindings.ts` is generated and **not committed** (`.gitignore`).
  `bun run gen:ipc` regenerates it (runs the `export-bindings` binary of
  `ferrule-app`); `dev`, `build`, `typecheck` and `test` run it first.
- Contract test, two halves: the Rust test `ipc_contract` asserts the
  export succeeds and contains every registered command and type; `tsc`
  compiling the frontend against the freshly generated file proves the
  frontend matches.

### 3.3 Conventions

- Identifiers are newtypes: `ProjectId`, `DocId`, `FileId`, `QueryId`.
- Text positions on the wire are **UTF-16 code-unit offsets**, the unit
  used by JS strings, CodeMirror and LSP (default encoding). `buffer`
  converts to its internal offsets. Line/column pairs are display-only.
- Every document message carries a `DocVersion`; Rust is authoritative
  (4.2).
- Errors: `IpcError { kind: IpcErrorKind, message: String }`, where `kind`
  is a closed enum (`NotFound`, `InvalidArgument`, `Cancelled`, `Io`,
  `Conflict`, `Internal`). The frontend switches on `kind` and never parses
  `message`.
- `ipc::PROTOCOL_VERSION` is returned by `app_info`; the frontend refuses to
  run against a mismatched backend.

### 3.4 Surface in M0

| Command                         | Returns                                                | Purpose                                                              |
| ------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------- |
| `app_info`                      | `AppInfo { name, version, protocolVersion, platform }` | Shell renders product name/version; protocol check.                  |
| `report_frontend_ready(report)` | `()`                                                   | Frontend signals it mounted. Used by the e2e smoke test (section 7). |

## 4. Data flows

### 4.1 Opening a project

1. User picks a folder (native dialog) or passes a path on the command
   line. Frontend calls `open_project(path)`.
2. `core` creates a `ProjectId`; `project` detects roots, ignore rules,
   `package.json` workspaces and `tsconfig.json` files.
3. `vfs` starts a recursive watcher. `core` immediately returns a
   `ProjectSnapshot` (root plus first-level children), so the window shows
   the tree well inside the 1 s budget.
4. In the background (a tokio task whose `CancellationToken` belongs to
   the project), `indexer` loads its on-disk cache, diffs it against the
   file system and re-indexes changed files. Progress streams as
   `IndexProgress` events to the status bar.
5. Tree expansion is lazy: `list_dir(projectId, dirId)` per expanded node.
   Watcher changes arrive as `FileTreeChanged` deltas. The tree view is
   virtualized.
6. Language servers start lazily on the first document of their language
   (4.3).

### 4.2 Editing a file

The Rust `buffer` is the single source of truth. CodeMirror in the webview
is a **replica** that follows the authority model of `@codemirror/collab`
(Rust is the authority, the view is the only peer):

1. `open_document(fileId)`: `core` loads text through `vfs`, creates a
   `buffer::Document` at version 0, runs the initial tree-sitter parse, and
   returns `{ docId, text, version, highlights for the first viewport }`.
2. User types. CodeMirror applies the change locally (instant echo) and
   sends `apply_changes(docId, baseVersion, changes)`, where `changes` is a
   list of `{ from, to, insert }` in UTF-16 offsets.
3. `core` applies them to the rope, records undo history, bumps the
   version and returns it. Then, off the response path:
   - `syntax` reparses incrementally and emits `HighlightsChanged` for the
     affected visible range;
   - `lsp-client` sends a debounced `textDocument/didChange`;
   - `vfs` writes a recovery file after a short idle delay.
4. Edits originating in Rust (undo/redo, multi-cursor commands, format,
   rename, quick fixes, auto-indent, file changed on disk) are pushed as
   `DocumentChanged { docId, version, changes, selections }`. If the view
   has unconfirmed local changes it rebases them over the authority's
   changes (`ChangeSet.map`), exactly as in the collab protocol.
5. Undo/redo and every editing action beyond plain text insertion
   (duplicate line, add caret, select next occurrence, move line, comment,
   auto-indent) are Rust commands. CodeMirror's history, language packages,
   autocomplete, lint and search are not used.
6. Save: `save_document(docId)` → `vfs` writes atomically (temp file +
   rename), applies EditorConfig rules, deletes the recovery file.

### 4.3 Completions from the TypeScript language server

1. When the first JS/TS document opens, `core` starts the configured server
   (`vtsls` by default, `typescript-language-server` as alternative; defined
   in `languages/typescript/`) through `lsp-client`, sends `initialize`
   with the project root, then `didOpen`.
2. The view calls `completions(docId, version, offset)` on a trigger
   character or explicit Ctrl+Space.
3. `core` flushes any pending debounced `didChange` for the document so the
   server sees exactly `version`, converts the offset with `buffer`, and
   sends `textDocument/completion`. A previous in-flight completion for the
   same document is cancelled (token + `$/cancelRequest`).
4. Results are merged with in-process items (`lang-js` keywords,
   auto-import candidates from `indexer`), fuzzy-filtered and ranked in
   Rust, truncated to a page, and returned as `CompletionList` with opaque
   item ids. If the document moved past `version`, the result is dropped
   with `Cancelled`.
5. The view renders a virtualized popup. Docs for the selected item load
   lazily (`completion_details(itemId)` → `completionItem/resolve`).
6. Accept: `apply_completion(docId, itemId)`. Rust computes the final edits
   (main edit, `additionalTextEdits` such as auto-imports, snippet
   placeholders) and pushes them as a normal `DocumentChanged` (4.2).

### 4.4 Search Everywhere (double Shift)

1. The keymap is resolved in `settings` and sent to the view at startup as
   an action table; double Shift maps to `search_everywhere`. Detecting the
   double tap is input handling and lives in the view.
2. Each keystroke in the popup calls
   `search_everywhere(query, tab, channel)`. `core` cancels the previous
   query's token, then runs providers in parallel: files (`indexer` file
   list + `search` fuzzy matcher), symbols (`indexer`), actions (action
   registry), settings, and later text matches.
3. Each provider streams ranked batches over the channel as soon as it has
   them (`SearchBatch { queryId, provider, items }`). The view drops
   batches with a stale `queryId`. Files usually arrive within
   milliseconds; slower providers fill in.
4. The result list is virtualized. Choosing an item calls
   `open_location(target)`, which goes through 4.2 and reveals the range.

## 5. Editor view layer: CodeMirror 6 as a view only

**Decision: CodeMirror 6 (MIT) is the input and rendering engine. Rust is
the authority for text and for all editing logic.** A custom renderer
remains possible later behind the same IPC contract.

Why not a custom virtualized renderer now:

- **Text input is the hard part, not drawing.** IME composition (CJK), dead
  keys, bidirectional text, soft wrap, native selection behaviour and
  screen-reader access are years of work to get right. CodeMirror 6 has
  solved them on the same engines Tauri uses (WebKitGTK, WKWebView,
  WebView2). Accessibility and i18n are in scope (phase 4).
- **It is already virtualized.** CodeMirror renders only the viewport.
- **Typing latency.** Local echo means a keystroke never waits for an IPC
  round trip, so the 8 ms budget is spent on DOM work only. A renderer that
  waits for Rust on each keystroke must fit IPC + edit + layout inside 8 ms
  on every platform, which is a real risk on WebKitGTK.
- **A proven sync model exists.** The collab model (versions, rebasing
  unconfirmed changes) keeps one authoritative copy in Rust and a replica
  in the view without inventing a new protocol.
- **The view stays thin.** Only `@codemirror/view`, `@codemirror/state` and
  `@codemirror/collab` are used. Highlighting arrives as decorations from
  tree-sitter in Rust; commands are Rust commands.

Costs we accept:

- Open documents exist twice in memory (rope + view string). Fine for
  source files; files above a threshold (set in M1, about 20 MB) open
  read-only with Rust-driven paging.
- The rebase path needs careful tests (golden tests for interleaved local
  and authority edits in M1).

## 6. Concurrency, cancellation and data safety

- One multi-threaded tokio runtime. Tauri commands are `async` and delegate
  to `core`.
- Every long operation takes a `tokio_util::sync::CancellationToken`;
  superseded requests (new query, new completion, closed project) cancel
  their predecessors.
- No lock is held across `.await`. Each open document is owned by one actor
  task that owns its `buffer`; other code talks to it through messages.
- CPU-heavy work (full parses, indexing, regex over many files) runs on
  `spawn_blocking` or a dedicated rayon pool, never on async workers.
- Dirty buffers are autosaved to recovery files in the platform data
  directory. On start, recovery files newer than the file on disk are
  offered for restore.

## 7. Testing strategy

| Layer          | Tool                                                             | Notes                                                                                  |
| -------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Domain crates  | `cargo test`                                                     | Unit + integration; golden files for highlighting and refactors; fake LSP/DAP servers. |
| IPC contract   | `cargo test` (`ipc_contract`) + `tsc` against generated bindings | See 3.2.                                                                               |
| Frontend views | Vitest via `bun run test`                                        | View functions and components with IPC mocked.                                         |
| App end-to-end | Playwright via `bunx playwright test`                            | See below.                                                                             |

**End-to-end and Playwright.** Playwright cannot attach to WebKitGTK (Linux)
or WKWebView (macOS). The M0 smoke test therefore uses Playwright as the test
runner and process driver: it builds and launches the real app binary with
`FERRULE_E2E_SMOKE=1`; the frontend mounts, renders the product name it got
over IPC and calls `report_frontend_ready` with what it rendered; the app
checks the report and exits 0 only if the window opened, the bundled frontend
loaded and the IPC round trip worked. On Linux CI it runs under `xvfb-run`.

For interactive UI tests from M1 on, the options are Playwright
`connectOverCDP` against WebView2 on Windows, and/or Playwright driving a
browser against the Vite build with IPC bridged to a headless Rust backend.
The choice is made and recorded here at the start of M1.

Playwright runs under Bun today (`bunx playwright test`). If that breaks,
only that CI step moves to Node and this section records why.

## 8. Milestone roadmap

Detailed task lists live in [ROADMAP.md](ROADMAP.md).

| Milestone                      | Scope                                                                                                                                                                                                             | Done when                                                                                                                                                                                                                                                 |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **M0** Foundation              | This document, workspace skeleton (all crates compile, empty), Tauri v2 + Vite + Bun shell, typed IPC pipeline, licenses and community docs, CI on Linux/macOS/Windows.                                           | All Rust and frontend checks pass with zero warnings; the app opens a window showing the product name and version fetched over typed IPC; the e2e smoke test passes.                                                                                      |
| **M1** Editor core             | Window layout, project tree (virtualized, lazy), tabs, rope buffer with undo/redo and multi-cursor, CodeMirror replica protocol, tree-sitter highlighting for JS/TS/JSX/TSX/CSS/HTML/JSON, save + recovery files. | Open `fixtures/react-ts`, browse the tree, edit several files in tabs with correct highlighting, undo/redo and multi-cursor; kill the process and recover unsaved text; golden highlight tests and buffer tests pass; typing latency measured in `perf/`. |
| **M2** Navigation and search   | File/symbol index with on-disk cache, Search Everywhere, Go to File/Symbol/Line, Recent Files, Find/Replace in Files (regex), breadcrumbs, folding.                                                               | Double Shift finds files and symbols in `fixtures/` in under 50 ms on a warm index; Find in Files with regex and replace preview works; Playwright covers each popup.                                                                                     |
| **M3** TypeScript intelligence | `lsp-client`, vtsls integration, completion, parameter hints, quick docs, go to definition/type/implementation, find usages, rename, diagnostics, Alt+Enter quick fixes, auto-import, ESLint + Prettier.          | On `fixtures/react-ts`: completion, go to definition, cross-file rename and diagnostics work end to end; LSP client tests run against a fake server; refactor edits have golden tests.                                                                    |
| **M4** Terminal and Git        | Integrated terminal (PTY), Git status in tree, commit, diff, blame, branches, log.                                                                                                                                | A shell runs in the terminal tool window on all three OSes; a change can be committed and its diff and log entry viewed from the UI; Git tests use temporary repositories.                                                                                |
| **M5** Run, debug, tests       | Run configurations (Node, npm scripts), Jest/Vitest/Playwright runners with gutter icons, debugger UI over DAP/CDP.                                                                                               | An npm script and a Vitest suite run from the gutter; a breakpoint in a Node script stops and shows variables.                                                                                                                                            |

The rest of the feature scope in `CLAUDE.md` (HTTP client, local history,
frameworks, settings UI, themes, plugins, auto-update, i18n) follows M5 and
is planned in ROADMAP.md.
