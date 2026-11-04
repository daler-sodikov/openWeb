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
                     external processes: typescript-language-server,
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

| Crate         | Owns                                                                                                                                                                                                                    | Internal deps                       |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| `buffer`      | Rope text storage, edits and change sets, versions, selections and multi-cursor commands, undo/redo history, line index, UTF-8/UTF-16/line-col conversions. Pure, no IO.                                                | none                                |
| `vfs`         | File system access, file ids, path interning, watching, encoding and line-ending detection, atomic save, crash-recovery files (autosave of dirty buffers).                                                              | `buffer`                            |
| `settings`    | Layered settings (defaults, user `settings.json`, project, `.editorconfig`), keymap presets (IntelliJ-style), schema and validation.                                                                                    | none                                |
| `project`     | Project model: roots, ignore rules, `package.json`/`tsconfig.json` discovery, npm/pnpm/yarn workspaces, open-document registry, recent files.                                                                           | `vfs`, `buffer`, `settings`         |
| `syntax`      | Shared tree-sitter machinery: incremental parsing, highlight captures to token spans, folding ranges, bracket pairs, language registry.                                                                                 | `buffer`                            |
| `lang-js`     | JS/TS/JSX/TSX and JSON grammars and queries, the TypeScript language server definition, `oxc` integration (parse, semantics, import resolution, unused imports, quick fixes).                                           | `syntax`                            |
| `lang-css`    | CSS/SCSS/LESS grammars, queries and in-process features.                                                                                                                                                                | `syntax`                            |
| `lang-html`   | HTML (later Vue/Svelte embedding) grammars, queries and in-process features.                                                                                                                                            | `syntax`                            |
| `indexer`     | Incremental project indexing: file list, symbols, on-disk cache, invalidation on file change.                                                                                                                           | `project`, `vfs`, `lang-*`          |
| `search`      | Fuzzy matching (camel humps, path segments) and regex text search/replace. Pure functions over caller-provided text, so unsaved buffers are searched like files.                                                        | none                                |
| `lsp-client`  | Generic LSP client: process lifecycle and restart, JSON-RPC framing, requests with cancellation, notifications, debounced `didChange`, capability negotiation, executable lookup. Knows nothing about specific servers. | none                                |
| `dap-client`  | Generic Debug Adapter Protocol client mirroring `lsp-client`, plus a Chrome DevTools Protocol client for browser/Node debugging.                                                                                        | none                                |
| `git`         | Status, diff, blame, log, branches, commit, conflicts. Read paths via `gix`; write paths may shell out to the `git` binary.                                                                                             | `vfs`                               |
| `terminal`    | PTY processes, VT parsing, terminal grid state and incremental updates for the view.                                                                                                                                    | none                                |
| `run-debug`   | Run configurations (Node, npm scripts, Jest, Vitest, Playwright), process supervision, test-result parsing, debug sessions.                                                                                             | `dap-client`, `project`, `settings` |
| `plugin-host` | `wasmtime` component-model host for plugins built against `wit/`.                                                                                                                                                       | `settings`                          |
| `ipc`         | Serializable DTOs shared with the frontend (`specta::Type` derives), error type, protocol version. No logic.                                                                                                            | none                                |
| `core`        | `Session`: owns open projects and documents, starts language servers, routes events, holds cancellation tokens, maps domain types to `ipc` DTOs. The only crate that knows about all others.                            | all of the above                    |

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

### 3.4 Command surface (M1, M2)

Commands are request/response. Rust → frontend **events** (M2, tauri-specta
typed): `fs-changed` (`FsChangeBatch`: paths created/changed/removed on disk)
and `index-status` (`IndexState`). Every edit to a document is still
initiated by the view through its serialized queue (4.2): an event only
tells the view that something changed, and the view then asks Rust
(`reload_document`, `disk_changed`). Per-request channels are not used yet:
Search Everywhere and Find in Files are request/response, and a newer Find
request cancels an older one in Rust (the headless test bridge has no
channel transport; it serves events through a long-poll endpoint).

| Command                                                                        | Returns                | Purpose                                                                       |
| ------------------------------------------------------------------------------ | ---------------------- | ----------------------------------------------------------------------------- |
| `app_info`                                                                     | `AppInfo`              | Product name/version; protocol check.                                         |
| `report_frontend_ready(report)`                                                | `()`                   | Startup report; used by the smoke test (section 7).                           |
| `initial_project`                                                              | `string \| null`       | Folder passed on the command line (`ferrule <path>`).                         |
| `open_project(path)`                                                           | `ProjectInfo`          | Opens a folder (replaces the current one).                                    |
| `list_dir(path)`                                                               | `DirEntry[]`           | Lazy tree listing; `path` is project-relative with `/`.                       |
| `open_document(path)`                                                          | `OpenedDocument`       | Text, version, language, dirty/restored flags.                                |
| `apply_changes(docId, baseVersion, transactions)`                              | `DocVersion`           | Local edits from the view (4.2).                                              |
| `undo` / `redo(docId, version)`                                                | `RemoteUpdate \| null` | Edits + selection computed by Rust.                                           |
| `add_carets(docId, version, selection, direction)`                             | `Selection`            | Clone carets above/below.                                                     |
| `select_next_occurrence` / `select_all_occurrences(docId, version, selection)` | `Selection`            | Multi-cursor selection commands.                                              |
| `highlights(docId, from, to)`                                                  | `Highlights`           | Token spans for a range, for the current version.                             |
| `save_document(docId)`                                                         | `DocVersion`           | Atomic save; clears the recovery file.                                        |
| `close_document(docId)`                                                        | `()`                   | Closes the tab; unsaved text stays in the recovery file.                      |
| `keymap`                                                                       | `KeyBinding[]`         | IntelliJ-style key bindings resolved in `settings`.                           |
| `new_line(docId, version, selection)`                                          | `RemoteUpdate`         | Enter: newline + language-aware indentation in Rust, recorded as typing (M2). |
| `reload_document(docId, version)`                                              | `DocumentReload`       | Replace the buffer with the file on disk, one undoable step (M2).             |
| `disk_changed(docId)`                                                          | `bool`                 | Whether the file differs from the last loaded/saved text (M2).                |
| `search_everywhere(query, tab)`                                                | `SearchResults`        | Files, symbols, actions (M2).                                                 |
| `recent_files`                                                                 | `string[]`             | Recently opened files, persisted per project (M2).                            |
| `find_in_files(search)`                                                        | `FindResults`          | Text/regex search with optional replacement preview (M2).                     |
| `replace_in_files(search, replacement)`                                        | `ReplaceResult`        | Rewrites closed files; returns edits for open documents (M2).                 |

The native folder picker is a Tauri dialog plugin call made by the view; it
only returns a path, which then goes through `open_project`.

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
   sends `apply_changes(docId, baseVersion, transactions)`. Each transaction
   is one CodeMirror update: a list of `{ from, to, insert }` edits in UTF-16
   offsets relative to the text before that transaction, the user-event kind
   (`input.type`, `delete.backward`, `input.paste`, ...) and the selection
   before and after. Versions count transactions, exactly like
   `@codemirror/collab`, so one transaction = one version step. The view keeps
   at most one request in flight per document and queues everything else
   (local pushes and Rust commands) behind it, so Rust sees edits in order.
3. `core` applies them to the rope, records undo history, bumps the
   version and returns it. Then, off the response path:
   - the syntax tree is edited (`tree.edit`) and marked stale; the next
     `highlights` request reparses incrementally. The view requests
     highlights for its visible range after edits settle (debounced) and on
     scroll, and maps existing decorations through local changes meanwhile;
   - (M3) `lsp-client` sends a debounced `textDocument/didChange`;
   - a recovery file is written after one second without further edits.
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
   rename) with the file's original line endings and BOM, deletes the
   recovery file. (EditorConfig rules: M2.)
7. Text is normalized to `\n` line endings on load, because CodeMirror
   normalizes too and offsets must agree. Closing a tab with unsaved changes
   never discards them: the recovery file stays, and reopening the file (or
   restarting after a crash) restores the text and marks the tab as
   modified ("restored unsaved changes").

### 4.3 Completions from the TypeScript language server

Planned for M3b; the server, sync and cancellation machinery are those of
4.7.

1. The TypeScript server runs as described in 4.7 (`typescript-language-server`).
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

As built in M2:

1. The keymap is resolved in `settings` and sent to the view at startup;
   double Shift is a `DoubleTap(Shift)` stroke. Detecting the tap is input
   handling in the view; global chords (Go to File, Find in Files, ...) are
   matched in the view against the same keymap.
2. Each keystroke calls `search_everywhere(query, tab)`. `core` ranks the
   in-memory index with the `search` fuzzy matcher: files (file name first,
   then path), symbols (from the grammars' tags queries), actions. The All
   tab returns the top items of each kind; an empty query returns recent
   files and all actions. The view drops responses of superseded keystrokes.
3. Choosing an item opens the file (4.2) and moves the caret to the symbol;
   actions run in the view (popups) or as Rust commands (editor actions).

Indexing (`indexer`): after `open_project`, a blocking task walks the
project honouring `.gitignore` (and always skipping `.git`/`node_modules`),
reuses cached entries whose mtime and size are unchanged, parses the rest in
parallel for symbols, saves the cache under the app data directory, and
emits `index-status`. Searches before it finishes report `indexing: true`.

### 4.5 Files changed on disk (M2)

1. After indexing, `vfs::Watcher` watches every indexed directory
   non-recursively (excluded trees never use OS watches) and delivers
   debounced batches of paths.
2. `core` updates the index (new directories get watched), saves the cache
   and emits `fs-changed`.
3. The view re-lists affected tree directories. For an open file: if the tab
   is unmodified it calls `reload_document` (a no-op when the text is
   unchanged, e.g. after our own save); if it is modified it calls
   `disk_changed` and, only if the disk really differs from the last saved
   text, shows **Reload** / **Keep my changes**.

### 4.6 Split editors and structure (M2b)

- **One replica per document, many views.** `DocModel` (view layer) holds
  a single collab hub `EditorState` that talks to Rust through the
  serialized queue (4.2). Each visible `EditorPane` (one per editor group)
  is a mirror: its local edits are forwarded to the hub and to the other
  panes; Rust-originated edits fan out from the hub to every pane. So the
  same file can be open in both groups and Rust still sees exactly one
  writer. Selection commands, undo and Enter act on the pane that invoked
  them.
- **Structure from the syntax tree.** `syntax` computes folding ranges
  (bracketed blocks, multi-line comments, markup elements, YAML/Markdown
  sections), the bracket pair at a position (tokens only, so brackets in
  strings and comments never match) and breadcrumbs (named scopes:
  functions, classes, elements, CSS rules, JSON/YAML keys, Markdown
  sections). The view requests folds after edits settle and caret info
  (breadcrumbs + bracket pair) after the caret moves; both are discarded if
  the document moved on.
- **Injections.** A language may declare injection queries; `SyntaxTree`
  parses the embedded regions (`<script>`/`<style>`, Svelte expressions,
  Markdown inline text and fenced code) with the named language from the
  registry and paints their highlights over the host's. One level deep;
  injected trees are reparsed (not incrementally) after edits.
- **EditorConfig** is resolved in `settings` when a document opens:
  indentation unit and tab width go to the view; trimming, final newline,
  line endings and BOM are applied by Rust on save as one undoable step that
  the view receives like any backend edit.
- **Auto-closing brackets** are view input behaviour (CodeMirror's
  `closeBrackets`) configured per language from Rust (`autoClose` pairs),
  so typing never waits for IPC.

### 4.7 Language servers (M3)

**Server choice.** `typescript-language-server` (Apache-2.0, an external
Node process wrapping the `tsserver` of the project's or a global
TypeScript). It is the most widely packaged server (npm, Bun, Homebrew,
distro packages) and needs no editor-specific setup; vtsls has better
VS Code parity for some features but is less commonly installed and adds a
second layer of VS Code settings. Nothing in `lsp-client` is specific to
it, so vtsls can be added as a second definition later. The definition
(program name, `--stdio`, file extensions → LSP language ids) lives in
`lang-js`.

**Finding the server** (first match wins; `core`, using
`lsp_client::find_program`):

1. `<project>/node_modules/.bin/typescript-language-server` (`.cmd` on
   Windows), so a project-pinned version wins;
2. the user setting `typescript.serverPath` in `settings.json` in the app
   config directory (Linux `~/.config/dev.ferrule.ide/`, macOS
   `~/Library/Application Support/dev.ferrule.ide/`, Windows
   `%APPDATA%\dev.ferrule.ide\`);
3. `PATH` (on Windows with `PATHEXT`). Bun's global bin (`~/.bun/bin`) and
   npm's are found only through `PATH`; no install locations are guessed.
   Apps started from a desktop launcher (macOS Dock in particular) may get
   a shorter `PATH` than a terminal, which is what the setting is for.

If the program found is a Node script (a `#!` line naming `node`, or a
Windows `.cmd` shim), `node` must also be on `PATH`. Missing server or
missing Node is not an error dialog: the server state becomes _not found_
with a status-bar message naming what to install or set, and every other
editor feature keeps working.

**Process lifecycle** (`lsp_client::Supervisor`).

- One server per open project, started lazily when the first document of
  one of its languages opens. Opening another project or closing the app
  stops it.
- Start: spawn with piped stdio (`kill_on_drop`), stderr lines go to
  `tracing` (target `lsp_stderr`), `initialize` with the root URI, the
  workspace folders (root plus detected workspace packages), our process id
  (the server exits if we die) and the client capabilities, then
  `initialized`. Server→client requests are answered
  (`workspace/configuration` → `null`s, `client/registerCapability`,
  `window/workDoneProgress/create` → ok; unknown → `MethodNotFound`).
- States reported to the view: _starting_, _indexing_ (while a
  `$/progress` work-done token is open), _ready_, _restarting_, _error_,
  _not found_, _stopped_.
- Crash (the process exits or stdout closes while running): pending
  requests fail with `Cancelled`, then the supervisor restarts after
  0.5 s, 1 s, 2 s, 4 s, 8 s. A server that stayed up for 60 s resets the
  count; after 5 failed starts in a row the state is _error_ until "Restart
  TypeScript Server" is run.
- Stop: `shutdown` request (2 s timeout), `exit` notification, wait 1 s,
  then kill. Done in the background so quitting stays within budget.

**Document sync.** The Rust buffer is the only source of the text the
server sees.

- Every applied edit (typing, undo/redo, Enter, reload, save edits, Replace
  in Files) yields, besides the tree-sitter byte edits, LSP-shaped changes
  (`buffer::LspChange`: start/end as line + UTF-16 column, computed on the
  rope at the moment each edit is applied, in application order), so they
  form a valid incremental `didChange` content-change list.
- Each server start has a _generation_. A document records the generation
  it was opened in. While holding the document lock, `core` either queues
  the changes (same generation) or sends `didOpen` with the full current
  text (new generation, e.g. after a restart). Restart replays `didOpen`
  for all open documents the same way. Because the check and the send
  happen under the document lock and all messages go through one ordered
  outgoing channel, the server can never see a change before its
  `didOpen` or miss one.
- `didChange` is debounced (100 ms after the last edit, changes merged into
  one notification carrying the latest version). Requests, `didSave` and
  `didClose` flush the document's pending changes first.

**Position encoding.** We offer only `utf-16`
(`general.positionEncodings`), the LSP default and the unit of our IPC
offsets, so no transcoding is needed beyond line/column ↔ offset. `buffer`
converts LSP positions to UTF-16 offsets (clamped to the line, and to the
document) and back. A server answering with another encoding is stopped
with an error state. Tests cover emoji (surrogate pairs) and CJK text.

**Requests and cancellation.**

- Requests are made with a `CancellationToken`. Cancelling sends
  `$/cancelRequest`, drops the pending entry and returns
  `IpcErrorKind::Cancelled`; a late response is ignored. Server errors
  `RequestCancelled` and `ContentModified` also map to `Cancelled`, which
  the view ignores silently.
- Per document, a newer hover (later: completion, signature help) cancels
  the older one. Navigation requests are cancelled by the next navigation.
- The view sends requests with the document version it has synced; `core`
  rejects a mismatch (`Conflict`). Requests do not run inside the view's
  serialized edit queue: the queue only pushes pending edits, then the
  request is awaited outside it so typing never waits for the server.

**Diagnostics.** `publishDiagnostics` is stored per file in `core` (LSP
ranges as received). `core` emits `problems-changed` for the file
(severity, message, source, code, 0-based line and UTF-16 column). An
editor showing that file then asks `doc_diagnostics(docId)`, which converts
the stored ranges to UTF-16 offsets against the current text (tagged with
its version, like highlights). Diagnostics of files that are no longer
open are dropped when the server clears them; the Problems window shows
whatever the server last published.

**Hover rendering.** The server's Markdown (`MarkupContent` or
`MarkedString`s) is rendered to HTML in Rust with `pulldown-cmark`. Raw HTML
in the Markdown is escaped, images become their alt text, links keep only
`http`, `https` and `mailto` URLs, and fenced code is highlighted with our
tree-sitter languages (the same token classes as the editor). The view
inserts that HTML into the tooltip; it never parses Markdown.

**IPC (M3a).** Commands: `hover(docId, version, offset)` →
`HoverResult | null` (`html`, `from`, `to`); `goto_definition(docId,
version, offset, kind)` (`kind`: `definition`, `typeDefinition`,
`implementation`) → `NavTarget[]` (`path`, `line`, `column`, `preview`);
`doc_diagnostics(docId)` → `DocDiagnostics`; `problems()` →
`FileProblems[]`; `language_servers()` → `ServerStatus[]`;
`restart_language_server()`. Events: `server-status` (`ServerStatus`:
name, state, message, detail) and `problems-changed` (`FileProblems`).
Targets outside the project root are returned but cannot be opened yet.

**Workspace.** `project` detects `tsconfig.json`/`jsconfig.json` files
(skipping ignored directories) and workspace packages from `package.json`
`workspaces` (npm, yarn, Bun) or `pnpm-workspace.yaml` (pnpm), with the
package manager from the lock file. The server uses them as workspace
folders; `tsserver` itself picks the nearest `tsconfig.json` per file.
Files changed on disk (4.5) are forwarded as
`workspace/didChangeWatchedFiles`, so the server notices new, deleted and
externally edited files and `tsconfig.json` changes.

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
- Enter round-trips to Rust (M2) so indentation logic stays out of the
  view. Keys pressed while it is in flight are deferred and replayed after
  the newline is applied, so fast typing keeps its order.

## 6. Concurrency, cancellation and data safety

- One multi-threaded tokio runtime. Tauri commands are `async` and delegate
  to `core`.
- Every long operation takes a `tokio_util::sync::CancellationToken`;
  superseded requests (new query, new completion, closed project) cancel
  their predecessors.
- No lock is held across `.await`. Each open document sits behind its own
  `std::sync::Mutex`, which is only ever locked inside `spawn_blocking`
  closures (edit, parse, highlight, save). This serializes work per document
  without blocking async workers. (The M0 plan said "actor task"; the mutex
  is equivalent here and simpler. Revisit if per-document work needs
  priorities.)
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

**End-to-end and Playwright.** Tauri's webviews cannot be driven by
Playwright on Linux (WebKitGTK) or macOS (WKWebView); only Windows' WebView2
exposes CDP. Options considered:

| Option                                                       | Real Rust logic | Real webview         | Cost                               | Verdict                                                                                                               |
| ------------------------------------------------------------ | --------------- | -------------------- | ---------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| A. Playwright + Vite, IPC mocked in TypeScript               | no              | no                   | low                                | Rejected: buffer, undo and multi-cursor live in Rust, so mocks would re-implement them in TS and test fake behaviour. |
| B. Playwright + Vite, IPC bridged to a headless Rust backend | yes             | no (Chromium)        | medium                             | **Chosen** for UI tests.                                                                                              |
| C. WebdriverIO + `tauri-driver`                              | yes             | yes (Linux, Windows) | high, no macOS, separate toolchain | Not now.                                                                                                              |
| D. Playwright `connectOverCDP` to WebView2                   | yes             | yes (Windows only)   | medium                             | Later addition for Windows-specific checks.                                                                           |
| E. Real-binary smoke test (M0)                               | yes             | yes                  | low                                | **Kept**: proves the real window, bundled assets and IPC.                                                             |

Decision: **B + E.**

- **UI tests (Playwright project `ui`).** Playwright drives Chromium against
  the Vite dev server. An init script installs a `window.__TAURI_INTERNALS__`
  shim that forwards every `invoke(cmd, args)` over HTTP to
  `ferrule-headless`, a binary of `ferrule-app` built with the `headless`
  feature. It builds the app with Tauri's mock runtime (`tauri::test`) and
  the **same** tauri-specta invoke handler, managed state and command
  functions as the real app, and passes each request through
  `tauri::test::get_ipc_response`. So argument decoding, commands and all
  Rust logic are real; only the transport and the browser engine differ.
  Each test copies a project from `fixtures/` into a temp directory and opens
  it via the `?project=<path>` URL parameter (the browser cannot show the
  native folder dialog).
- **Smoke test (Playwright project `smoke`).** Unchanged from M0: the real
  binary, real webview, bundled frontend, `FERRULE_E2E_SMOKE=1`.
- **Production CSP in `ui` tests.** Vite serves the pages with the CSP from
  `tauri.conf.json` (plus `connect-src` for the test backend) during UI
  tests, so a CSP that breaks the UI fails the tests. Added after a manual
  check found CodeMirror unstyled in the real app: its base theme is a
  runtime `<style>` element, so `style-src` must include `'unsafe-inline'`
  (scripts stay `'self'` only).
- **Local browser override.** `FERRULE_E2E_CHROMIUM=<path to chrome>` makes
  the `ui` project use an already installed Chromium instead of the one
  `bunx playwright install chromium` downloads (for slow or offline
  machines). CI does not set it.
- **Known gaps:** engine differences (Chromium vs WebKitGTK/WKWebView/
  WebView2) and the dialog plugin are not covered by `ui`. Manual checks on
  each OS cover them until option D is added.

Playwright runs under Bun today (`bunx playwright test`). If that breaks,
only that CI step moves to Node and this section records why.

## 8. Milestone roadmap

Detailed task lists live in [ROADMAP.md](ROADMAP.md).

| Milestone                      | Scope                                                                                                                                                                                                                                   | Done when                                                                                                                                                                  |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **M0** Foundation              | This document, workspace skeleton (all crates compile, empty), Tauri v2 + Vite + Bun shell, typed IPC pipeline, licenses and community docs, CI on Linux/macOS/Windows.                                                                 | All Rust and frontend checks pass with zero warnings; the app opens a window showing the product name and version fetched over typed IPC; the e2e smoke test passes.       |
| **M1** Editor core             | Window layout, project tree (virtualized, lazy), tabs, rope buffer with undo/redo and multi-cursor, CodeMirror replica protocol, tree-sitter highlighting for JS/TS/JSX/TSX/CSS/HTML/JSON, save + recovery files.                       | See the concrete checklist in [ROADMAP.md, M1](ROADMAP.md#m1-editor-core).                                                                                                 |
| **M2** Search and navigation   | File/symbol index with on-disk cache, Search Everywhere, Go to File/Symbol/Line, Recent Files, Find/Replace in Files (regex), file watching, Rust indentation.                                                                          | See [ROADMAP.md, M2](ROADMAP.md#m2-search-and-navigation). Breadcrumbs, folding and the rest moved to M2b.                                                                 |
| **M3** TypeScript intelligence | `lsp-client`, `typescript-language-server` integration, completion, parameter hints, quick docs, go to definition/type/implementation, find usages, rename, diagnostics, Alt+Enter quick fixes, auto-import. In two parts, M3a and M3b. | See [ROADMAP.md, M3](ROADMAP.md#m3-typescript-intelligence).                                                                                                               |
| **M4** Terminal and Git        | Integrated terminal (PTY), Git status in tree, commit, diff, blame, branches, log.                                                                                                                                                      | A shell runs in the terminal tool window on all three OSes; a change can be committed and its diff and log entry viewed from the UI; Git tests use temporary repositories. |
| **M5** Run, debug, tests       | Run configurations (Node, npm scripts), Jest/Vitest/Playwright runners with gutter icons, debugger UI over DAP/CDP.                                                                                                                     | An npm script and a Vitest suite run from the gutter; a breakpoint in a Node script stops and shows variables.                                                             |

The rest of the feature scope in `CLAUDE.md` (HTTP client, local history,
frameworks, settings UI, themes, plugins, auto-update, i18n) follows M5 and
is planned in ROADMAP.md.
