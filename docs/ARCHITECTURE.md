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
`restart_language_server()`. Events: `server-status-changed` (`ServerStatus`:
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

### 4.8 Editing features from the language server (M3b)

**One way to change text.** Every edit computed from the server
(completion, rename, code actions, formatting) is turned in Rust into
UTF-16 `TextEdit`s for one document version. For an **open** document the
view applies them as one local transaction (one undo step) after checking
that its synced version is still that version, exactly like Replace in
Files (M2); the transaction then reaches Rust through the normal queue.
**Closed** files are rewritten by Rust atomically, keeping line endings and
BOM; a file whose content changed since the preview was computed is
skipped and reported, never overwritten. Resource operations (create,
rename, delete files) are reported as unsupported.

**Completion.** `completion(doc, version, offset, trigger)` asks the server
and keeps the raw items in a per-document session in Rust. The view gets a
session id, the replace range and the first page of items, ranked in Rust
with the `search` fuzzy matcher (then `sortText`, then `preselect`). While
the popup is open, each keystroke calls `filter_completion(session,
prefix)`: ranking runs in Rust without asking the server again (unless the
list was incomplete). The selected item's details come lazily from
`completionItem/resolve`. Accepting calls `accept_completion(session,
item, version)`: Rust resolves the item if needed, then returns the main
edit (from the item's range start to the caret), `additionalTextEdits`
(auto-imports) and, for snippets, the text without placeholders plus the
tab stops. Keys pressed meanwhile are deferred and replayed, like Enter.
A newer completion request cancels the older one in the server.

**Signature help, usages.** `signature_help` returns the active signature
with parameter ranges in its label (rendered by the view, documentation as
sanitized HTML). `find_usages` returns `textDocument/references` grouped by
file with line previews for a results tool window.

**Rename.** `prepare_rename` (if the server supports it) gives the range
and placeholder; `rename_preview(name)` stores the server's
`WorkspaceEdit` and returns the files and occurrence counts;
`apply_rename(preview)` returns the edits for open documents and rewrites
closed files as above.

**Code actions.** `code_actions(range)` sends the diagnostics overlapping
the range as context and keeps the actions in Rust. An action with an
`edit` is applied directly. An action with a `command` is executed with
`workspace/executeCommand`; typescript-language-server answers by sending
`workspace/applyEdit` back. `lsp-client` hands such server requests to the
client hooks; `core` captures the edit while its own command runs, answers
`applied: true`, and applies the captured edit through the pipeline above
once the command returns (an `applyEdit` that arrives at any other time is
refused). Organize Imports asks for `source.organizeImports`.

**Formatting** uses `textDocument/formatting` with the indent unit and tab
width of the document (EditorConfig).

**Read-only files.** Navigation and usage targets outside the project or
inside `node_modules` open as read-only documents (`open_external(path)`
for absolute paths): highlighted, synced to the server so hover and
navigation work in them, but edits are refused in Rust and blocked in the
view. They are never autosaved or saved.

**oxc fast path.** `lang-js` parses JS/TS with `oxc` (in process) after
edits settle and reports unused imports, shown faded like the server's
"unnecessary" hints but without waiting for it. "Sort Imports" reorders
the leading import declarations by module specifier in-process.

**Typing-path spans.** With `FERRULE_TYPING_TRACE=<file>` the view records,
per keystroke, the time from the key event to the local-echo frame, to the
Rust buffer confirming the edit and to the frame showing fresh highlights
(User Timing marks), and Rust records how long `apply_changes` and
`highlights` take. Both append to the file; `ferrule-perf` reports their
medians next to the external key-to-glyph latency.

### 4.9 Terminal and Git (M4)

**Terminal: who owns what.** Rust owns each terminal session completely:
the PTY (`portable-pty`, ConPTY on Windows), the shell process tree, the
VT parser and the screen/scrollback grid (`alacritty_terminal`), the
selection and link detection. The view draws rows of styled cells and
sends input; it has no terminal state of its own.

Why not xterm.js as the view: xterm.js is itself a full VT emulator, so
using it would mean either parsing the output twice (Rust and JS, two
states that can disagree) or moving the emulator into TypeScript, against
the thin-view rule. Rows of cells are simple to draw with the DOM (only the
visible rows exist), and keeping the grid in Rust gives link detection,
selection text and later search in one place. Cost we accept: our own
renderer (colors, cursor, wide characters, selection) and no terminal mouse
reporting in M4a.

**PTY lifecycle and cleanup.**

- Spawn: the user's shell (settings `terminal.shellPath` and
  `terminal.shellArgs`, otherwise `$SHELL`, then the account's login shell
  from its passwd entry, then `/bin/sh` on Unix; `pwsh`, then `powershell`, then `%ComSpec%` on Windows),
  as a login shell on macOS (`-l`, so `PATH` matches a terminal), in the
  project root, with `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=Ferrule`.
- Unix: the shell is a session and process-group leader (portable-pty
  `setsid`). Closing sends `SIGHUP` to the whole process group, then
  `SIGKILL` after 2 s. Should Ferrule die, the kernel hangs up the PTY when
  its master closes.
- Windows: the shell is assigned to a Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` right after spawn; closing the
  terminal (or Ferrule exiting for any reason) closes the job handle and
  ends every process in it.
- Terminals belong to the open project: opening another project or quitting
  closes them all. Closing is done off the async workers and never waits on
  the view.

**Bytes to the view (batching, backpressure).** A reader thread per
terminal reads the PTY in 64 KiB chunks and feeds the parser under the
terminal's lock; it never talks to the view. A frame task marks the
terminal dirty and, at most every 16 ms, sends one `terminal-frame` event
with only the rows that changed in the visible viewport (plus cursor,
size, scroll position, title, exit status). Only one frame is in flight per
terminal: the next is sent after the view acknowledges the previous one
(`terminal_ack`), so a slow view gets fewer, larger frames, never a queue.
A program flooding stdout (`yes`) costs at most one frame of visible rows
per 16 ms; the parser keeps up with the PTY, and if it does not, the PTY's
own buffer blocks the writer (the OS backpressure). The view renders a frame
in one animation frame and caps work to the visible rows.

**Input.** Text and paste go to Rust as text (bracketed paste when the
program asks for it); special keys go as key names with modifiers and Rust
encodes them for the current mode (application cursor keys, etc.). While
the terminal has focus, only the terminal's own shortcuts (toggle, copy,
paste, new tab) are handled by Ferrule; every other key reaches the shell.

**Links.** Rust recognizes `path:line[:col]`, `path(line,col)` (tsc) and
`file://` URLs in a row's text; a Ctrl/Cmd+click asks Rust for the link
under the cell, which resolves it against the terminal's starting directory
and the project root and only returns files that exist.

**Git (M4b): the `git` executable, not gix.** Every Git operation runs the
user's `git` (found on `PATH`, setting `git.path`) with machine-readable
output (`status --porcelain=v2 -z`, `blame --porcelain`, `for-each-ref`,
`log` with separator bytes) and is parsed in `ferrule-git`. The M3 plan was
gix for reads and `git` for writes; changed in M4b because:

- exact parity with the command line for everything that shapes status and
  commits: the user's config, `.gitattributes`, filters, LFS, hooks,
  signing, `core.fsmonitor`, sparse checkouts;
- one code path for reads and writes, so what Ferrule shows is what `git`
  will act on;
- gix and its ~60 crates were not available offline during M4 (see
  ROADMAP, "M4 build workflow"), while `git` lets the whole crate be built
  and tested against real repositories.

Costs we accept: a process per request (status of a mid-size repository is
tens of milliseconds; requests are debounced, run off the async workers and
superseded ones are dropped), and `git` (≥ 2.25 for `switch`/`restore`)
must be installed; without it the Git features say so and stay off.
Commands run with `LC_ALL=C` (stable messages to classify errors),
`GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0` for reads, and never with
network-using subcommands: fetch, pull and push are not offered in M4b, and
nothing reads or stores credentials.

**Status model and refresh.** Status is a list of files (project-relative
path, original path for renames, index and worktree change, and the derived
kind: modified, added, deleted, renamed, untracked, ignored, conflicted) plus
the folders containing changes and the ignored folders, and the branch
(name or detached HEAD, upstream, ahead/behind). The project runtime keeps
the latest one; it is recomputed after file-watcher batches, after
Ferrule's own Git writes and saves, and when the window gains focus,
debounced (200 ms), with a generation counter so a slower old run never
overwrites a newer one; then it is pushed as the `git-status` event. The
tree and the commit window read from it.

**Line changes, rollback, diff, blame.** Line diffs are computed in Rust
(Myers on interned lines; past a cap of differences the rest of a region
becomes one hunk) between the open document's current text, unsaved edits
included, and the index version of the file (`git show :path`, cached per
status generation). Gutter markers are hunks (added, modified, deleted);
rollback replaces a hunk with the index text as one edit to the document
(one undo step). The diff viewer compares HEAD with the working tree and
gets rows ready to draw from Rust (unified rows, or aligned side-by-side
rows), with word-level ranges for modified lines. Blame runs
`git blame --porcelain --contents -` on the document's current text, so
lines match the editor.

**Branches and commit.** Branches come from `for-each-ref` (local and
remote). Checkout (`git switch`, `--track` for a remote branch) is refused
while an open document has unsaved changes; `git` itself refuses when local
changes would be overwritten, and that message is shown. Delete uses
`branch -d` (refuses unmerged branches; the error says so). The commit window
stages and unstages with `git add` / `git restore --staged` (`git rm
--cached` before the first commit), saves dirty documents, then runs
`git commit` (hooks run) with optional `--amend`. Missing
`user.name`/`user.email`, failing hooks and empty commits are recognized and
reported in plain words.

**IPC (M4).** Terminal commands: `terminals()` → `TerminalList` (open
terminals, and why none can start in this build), `open_terminal(size)` →
`TerminalInfo` (id, title, shell, cwd), `terminal_input(id, text)`,
`terminal_key(id, key)`, `terminal_paste(id, text)`,
`terminal_resize(id, size)`, `terminal_scroll(id, lines)`,
`terminal_select(id, anchor, head, kind)`, `terminal_clear_selection(id)`,
`terminal_selection_text(id)`, `terminal_link_at(id, cell)` →
`TerminalLink` (target and column range), `rename_terminal(id, title)`,
`close_terminal(id)`, `terminal_ack(id, seq)`. Events:
`terminal-frame-event` (`TerminalFrame`: dirty rows of styled runs; colors
are 0 = default, 1–16 = theme palette, `0x01RRGGBB` = 24-bit, Rust resolves
the other xterm colors; flags from the exported `TERMINAL_FLAGS`) and
`terminal-exited-event`. The view sends input through one ordered queue
(IPC calls may otherwise be handled out of order).
Git commands: `git_status`, `git_refresh`, `git_line_changes(doc)`,
`git_rollback_hunk(doc, version, line, lines)` → `DocumentReplace`,
`git_diff(path, mode)`, `git_blame(doc)`, `git_log(path?)`,
`git_branches`, `git_checkout(name, remote)`,
`git_create_branch(name, checkout)`, `git_delete_branch(name, force)` →
`Deleted | NotMerged`, `git_stage(paths)`, `git_unstage(paths)`,
`git_commit(message, amend, paths)` (saves, restages the checked files,
commits), `git_last_commit_message`. Event: `git-status-event`
(`GitStatus`).

**Test backends.** `ferrule-terminal` has an in-memory backend (`fake`,
feature `testing`): its "shell" echoes input and its screen is plain text.
Core tests use it, and the headless UI-test backend offers it with
`--fake-terminals` (Playwright: `FERRULE_E2E_FAKE_TERMINAL=1`) so the
terminal UI is tested where the native backend is not built. It is never
used by the app.

**What M4 verified, and where (2026-11-06).** M4 was written without
downloading the terminal crates (ROADMAP, "M4 build workflow"); after the
maintainer's fetch everything was compiled and run on Linux:

| Part                                                                                           | Status (Linux)                                                |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `ferrule-terminal` with `native` (portable-pty, alacritty_terminal, process group, Job Object) | compiled first time without errors or warnings; clippy clean  |
| `ferrule-terminal/tests/native.rs` (real PTYs: echo, cwd, resize, exit code, cleanup, flood)   | 5/5 pass (Unix code paths; Windows paths compiled only on CI) |
| Terminal and Git core, IPC, UI                                                                 | Rust, Vitest and Playwright tests pass                        |
| Terminal UI with real shells (pwd, history, word selection, links, `node` flood)               | Playwright 5/5 pass with the native headless backend          |
| App binary and the smoke test (`bun tauri build --debug --no-bundle`)                          | pass                                                          |
| `cargo deny check`                                                                             | pass (advisory database as of 2026-11-05, no fetch)           |
| macOS and Windows                                                                              | **not run yet**: CI after the push                            |
| Alternate screen (`less`, `vim`), 10 000-line scrollback, memory during a long `yes`           | **not tested** (no automated test; check by hand)             |

### 4.10 Run configurations, tests and debugging (M5)

**Who owns what.** Rust owns everything: the configurations, which command
runs, the processes, their output, the parsing of test reports and the
test tree. The view draws the run tool window, the configuration popup, the
test tree and gutter icons, and sends commands. No process, test-parsing or
debugger logic is in TypeScript.

**Crates.** `run-debug` is pure logic and has no processes: the
configuration model and its file, detection from `package.json`, building
the exact command line, reporter parsing (Vitest, Jest, Playwright) and test
filters. It is unit-tested offline with reporter JSON. `core` starts and
supervises runs; tree-sitter discovery of test blocks lives in `lang-js`.
The debugger (M5b) is a new crate, see below.

**Configuration model and storage.** A configuration has an id, a name, a
kind and options: Node (a file), npm script (a script name), Vitest, Jest
or Playwright (a scope: everything, a file, or one test), a working
directory (project-relative), arguments, environment variables and an
optional package manager. They are stored in the project in
`.ferrule/run.json` (never `.idea`), versioned (`"version": 1`), sorted, so
the file can be committed and diffed. Detected configurations (one per
`package.json` script; one per test framework found in `dependencies` or
`devDependencies`) are shown next to the stored ones and become stored when
edited. The package manager comes from `packageManager` in `package.json`,
else from the lockfile (`bun.lock`/`bun.lockb` bun, `pnpm-lock.yaml` pnpm,
`yarn.lock` yarn, `package-lock.json` npm), else npm.

**Commands.** npm scripts run as `<pm> run <script>`. Test frameworks and
Node files run as `node <script>`, where the framework's script is the
`bin` entry of its `package.json`, found like Node finds packages (walking
up `node_modules` from the working directory, so hoisted monorepo installs
work): no `.cmd` shims on Windows, and the same command on every OS. On
Windows a package manager that is a `.cmd`/`.bat` shim is started through
`cmd.exe /d /c <name> run …`; arguments with characters `cmd.exe` would
interpret (`& | < > ^ % ! " ( )`, spaces, empty) are refused with an error
instead of being mangled. Each run gets the reporter flags for a JSON report in
Ferrule's data folder (Vitest `--reporter=default --reporter=json
--outputFile.json=<file>`, Jest `--json --outputFile=<file>`, Playwright
`--reporter=list,json` with `PLAYWRIGHT_JSON_OUTPUT_NAME=<file>`); the
human-readable output still goes to the run's terminal.

**Safety.** Configurations in a project are data: nothing runs when a
project opens, and nothing runs until the user has trusted the project. The
first run in a project shows the exact command line (program, arguments,
working directory, environment changes) and asks "Trust this project";
the answer is stored in Ferrule's data folder (per project, never in the
project itself, so a project cannot trust itself). Configurations changed
on disk are re-read before every run, and the shown command is the one that
runs.

**Process lifecycle and cleanup.** A run is a program on a PTY, using the
M4 terminal session (`ferrule-terminal`): ANSI colors, file links,
scrollback, selection and copy come with it, and so does the cleanup of
the whole process tree (`ferrule-process`: process group on Unix, Job
Object on Windows). Runs live in their own `Terminals` registry in the
project runtime, with ids that do not overlap the terminals', so the
terminal commands (scroll, select, link, copy) work on run output. Stop
ends the tree (SIGHUP then SIGKILL after 2 s on Unix; the job on Windows).
The M4 Windows rules apply: `close_project` (and replacing a project) waits
until every run process, the file watcher and every background task that
uses the project folder as working directory has ended, and reports what
did not.

**Test results, from reporter to view.** When a test run's process exits,
core reads its JSON report, parses it in `run-debug` into a test tree
(file > suites > tests; status passed / failed / skipped; duration; failure
messages; stack frames with file, line and column) and pushes a
`run-finished` event with the exit code, the duration and the tree.
Reporters write their JSON at the end, so while a run is going the tests it
covers are shown as running (from the gutter's test blocks), not live one
by one. The latest result of each test is kept per project for the gutter
until the file's tests change. Re-run failed, run a file or one test become
a new run with the framework's filter: Vitest and Jest `-t <anchored
regex of the full name>` plus the file; Playwright `<file>:<line>`.

**Gutter icons.** `lang-js` finds `describe`/`it`/`test` calls (with
`.only`, `.skip`, `.each`) and their string titles with a tree-sitter
query, giving each block its full name and line. The gutter shows a run
icon, replaced by the last status of that test; clicking runs it.

**Debugger (M5b).** Node is debugged through the Chrome DevTools Protocol
directly: `node --inspect-brk=127.0.0.1:<random port>`, then a WebSocket to
the inspector. Why not DAP for Node: Node speaks CDP natively; the DAP route
needs an adapter (js-debug) that is a large download and a second process
to supervise, while the CDP subset a debugger needs (Debugger._, Runtime._)
is small. The client goes into a new crate `cdp-client` (the protocol is not
DAP, so `dap-client` would be a misleading home); the debugger session model
(breakpoints, frames, variables) is protocol-neutral so a DAP backend can be
added for other languages. The detailed design follows (4.10.1 to 4.10.8);
the slices are in `docs/ROADMAP.md`, M5b.

**IPC (M5a, as built).** Commands: `run_configurations` (stored +
detected, the package manager, trust), `save_run_configuration`,
`delete_run_configuration`, `trust_project`, `revoke_project_trust`,
`start_run(config, size)` →
`Started { run }` or `NeedsTrust { command, cwd }` (the trust question
shows exactly this), `stop_run(id)`, `close_run(id)`, `rerun(id)`,
`rerun_failed(id)`, `rerun_tests(id, targets)` (files, suites or tests from
the tree; paths outside the run's folder are refused), `run_at(doc, line)`
(the innermost test block containing the line, else the test file, else
the file with Node), `test_blocks(doc)` (gutter, with last statuses),
`runs()`. Event: `run-finished-event` (exit code, whether the user stopped
it, duration, test tree, counts, or why there is no report); output uses
the terminal frame events with ids from `RUN_ID_BASE`.

**What M5a verified, and decisions to review.**

- Name filters: Vitest's `-t` matches the full name joined with `" > "`,
  Jest's with spaces (checked against Vitest 5.1.0; Jest from its docs).
  Parameterized titles (`%i`, `$name`) become `.*`.
- Vitest's JSON has no test lines; they are taken from the tree-sitter
  blocks of the file, so a click on a Vitest test opens it.
- `RunFinished.stopped`: npm exits with code 1 after forwarding the stop
  signal, so the exit code alone cannot say "stopped".
- UI tests use the in-memory run backend through the headless server's
  `--fake-runs` (the default in `playwright.config.ts`) while terminals
  stay real; `FERRULE_E2E_REAL_RUNS=1` runs the same spec against real
  Node and Vitest with the repository's `node_modules` linked into the
  project copy. The core tests start real Node, Vitest and Playwright.
- Runs save unsaved documents first (the tools read the files on disk).
- Trust is per project folder, stored as `trusted` in Ferrule's
  `projects/<hash>.json`; the "Revoke Project Trust" action (Go to Action,
  no default shortcut) clears it, and runs already going are not stopped.
- Keys typed while a run's output has the focus go to the program, like in
  the terminal; only the run actions and Alt+F12 are handled by Ferrule.

#### 4.10.1 Crates and the CDP client (`cdp-client`)

**Crate graph (acyclic).** `cdp-client` depends on no Ferrule crate: it is
the protocol and the transport, nothing about breakpoints or the editor.
`run-debug` (pure logic, no processes, no tokio) gets the protocol-neutral
session model, the breakpoint store and the source-map code (4.10.2,
4.10.3). `core` depends on both and owns the glue: a `DebugBackend` trait,
the CDP implementation of it, the session driver and the IPC. `dap-client`
stays the empty placeholder from M0; a DAP backend is a later second
implementation of `DebugBackend`, and its crate description is corrected
when that starts (it says "and Chrome DevTools Protocol" today).

**Public API (small; transport and protocol modules private).**

```rust
/// Finds the inspector's WebSocket URL: GET http://127.0.0.1:<port>/json/list,
/// retried until Node is listening, the deadline passes or `cancel` fires.
pub async fn discover(port: u16, deadline: Duration, cancel: &CancellationToken)
    -> Result<String, CdpError>;

pub struct CdpClient { /* cheap to clone: a handle to the connection task */ }
impl CdpClient {
    /// Connects to a `ws://127.0.0.1:<port>/<uuid>` URL (any other host is refused).
    pub async fn connect(url: &str, cancel: CancellationToken)
        -> Result<(CdpClient, CdpEvents), CdpError>;
    /// One request; resolves with the result or the protocol error.
    pub async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P)
        -> Result<R, CdpError>;
    /// Closes the socket and ends the connection task (idempotent).
    pub async fn close(&self);
}
/// Events and the end of the connection, in arrival order.
pub struct CdpEvents { /* recv() -> Option<CdpEvent> */ }
pub enum CdpEvent { Paused(..), Resumed, ScriptParsed(..), BreakpointResolved(..),
    Console(..), ExceptionThrown(..), Closed { reason } /* ... */ }
pub enum CdpError { Closed, Cancelled, Timeout, Protocol { code: i64, message: String },
    Transport(String), Refused(String) }
```

Typed requests and events cover only the subset the debugger needs
(`Debugger.enable/disable/setBreakpointByUrl/removeBreakpoint/
setBreakpointsActive/setPauseOnExceptions/resume/stepOver/stepInto/stepOut/
pause/evaluateOnCallFrame/getScriptSource`, `Runtime.enable/
runIfWaitingForDebugger/getProperties/evaluate/releaseObjectGroup`, and the
events `Debugger.paused/resumed/scriptParsed/breakpointResolved`,
`Runtime.consoleAPICalled/exceptionThrown`). They are hand-written serde
structs for this subset, not generated from the protocol's JSON: the whole
protocol is large, the subset is about thirty types, and the file stays
reviewable. This is not the "hand-written IPC type" the rules forbid: these
types never cross the IPC boundary (4.10.4 uses generated types).
Unknown fields and unknown events are ignored, so a newer Node does not
break the client.

**Transport.** One tokio task owns the socket. Requests carry increasing
ids; the task keeps `id → oneshot` for the pending calls and forwards
events to the `CdpEvents` channel (bounded; a debuggee that floods
`consoleAPICalled` is throttled by the channel, not buffered without
bound). No lock is held across an `.await`: callers send a command to the
task over an mpsc channel and await their oneshot. Every `call` has a
timeout (default 10 s, `Timeout`), and every wait selects on the
connection's `CancellationToken`; cancelling it closes the socket, fails
all pending calls with `Cancelled` and ends the task. When the socket
closes (the debuggee exited or crashed) pending calls fail with `Closed`
and `CdpEvent::Closed` is the last event.

**Connecting to Node.** Core picks the port by binding `127.0.0.1:0`,
reading the port and releasing it, then starts
`node --inspect-brk=127.0.0.1:<port> <script>` (the pick-then-release gap
is a race with another local process; if Node then fails to listen, it
exits early or `discover` times out, and the start is retried up to three
times with a new port before the error is shown). `discover` reads
`/json/list` with a minimal HTTP/1.1 request over `tokio::net::TcpStream`
(no HTTP client dependency; the answer is one small JSON document) and
returns the `webSocketDebuggerUrl`, which has Node's random UUID in its
path. Parsing Node's "Debugger listening on ws://…" line from the output is
not used: the output is a PTY stream and the line can be split or
colored. `--inspect-brk` makes Node wait for a debugger at the first
line: after `Debugger.enable`, the breakpoints and `Runtime.enable`, the
session sends `Runtime.runIfWaitingForDebugger`, then the program runs
to the first breakpoint (or ends).

**Dependencies and licences.** The only new runtime dependency is
`tokio-tungstenite` (MIT) with `default-features = false` and the
`connect` feature only (no TLS, no `rustls`, no OpenSSL: the socket is
plain `ws://` on loopback, and TLS is not needed and not wanted). It
brings `tungstenite` (MIT or Apache-2.0) and small permissive crates for
the handshake and framing (`httparse`, `sha1`, `base64`, `http`, `bytes`,
`utf-8`, `data-encoding`, `rand`); `futures-util` (MIT or Apache-2.0) and
`httparse` are already in `Cargo.lock`. Writing the WebSocket framing,
masking, fragmentation, ping/pong and the close handshake by hand was
rejected: that is exactly the kind of code that has security-relevant edge
cases, and the crate is widely used and maintained. Nothing is assumed:
slice M5b-1 adds the dependency, runs `cargo deny check` (all four
targets in `deny.toml`), records the new crates in
`THIRD_PARTY_LICENSES.md` and justifies the dependency in the commit
message. If `cargo deny` rejects any transitive crate, the slice stops and
reports instead of adding an exception on its own.

#### 4.10.2 The protocol-neutral session model (`run-debug`)

Pure data and pure functions, so it is unit-tested offline and a DAP
backend can fill it later. Lines and columns are zero-based (like
`run_at`); paths are project-relative with `/` when inside the project,
else absolute (a library in `node_modules` is inside the project; Node's
own `node:` modules have a `script` name and no path).

- **`Breakpoint`**: `id` (Ferrule's, stable across sessions), `path`,
  `line`, `condition: Option<String>`, `log_message: Option<String>`,
  `enabled`, and a `state` that is session data, not stored:
  `Unverified { reason }` (no session, script not loaded yet, no code at
  that line) or `Verified { line }` (the line the debugger actually
  resolved, possibly later than asked, as in every debugger).
- **`BreakpointStore`**: the project's breakpoints, sorted for stable
  files, persisted (4.10.5), with `toggle(path, line)`, `update`, `remove`,
  `shift_for_edit` (4.10.5) and `for_path`. Pure; the file IO is in `core`.
- **`DebugState`**: `Starting`, `Running`, `Paused(PauseInfo)`,
  `Ended { exit_code: Option<i32>, stopped: bool }`.
- **`PauseInfo`**: `reason` (`Breakpoint`, `Step`, `Pause`, `Exception {
message }`, `Entry` for the `--inspect-brk` stop), the call stack
  (`Vec<Frame>`) and the selected frame.
- **`Frame`**: `id` (opaque string from the backend), `name`, a
  `Location { path_or_script, line, column }` (already mapped back to the
  original file when a source map applies, with `generated` kept for
  diagnostics), `is_library` (frames under `node_modules` or `node:` are
  dimmed in the view), and `scopes: Vec<Scope>` (names and expandable
  references only; contents are fetched lazily).
- **`Scope`**: `kind` (`Local`, `Closure`, `Block`, `Module`, `Global`),
  `name`, `variables_ref`.
- **`Variable`**: `name`, `value` (a short preview string made by the
  backend: `"abc"`, `Array(3)`, `{a: 1, …}`, `ƒ foo()`), `type_name`,
  `variables_ref: Option<String>` (present when expandable) and
  `evaluate_name` (for "add to watches"). Children are fetched by
  `variables_ref`, 100 at a time (`start`, `count`), so a huge array never
  crosses IPC whole.
- **`DebugBackend`** (a trait in `core`, async; the model above is its
  vocabulary): `start`, `set_breakpoints(path, &[Breakpoint]) ->
Vec<BreakpointState>`, `resume`, `step_over`, `step_into`, `step_out`,
  `pause`, `variables(ref, range)`, `evaluate(frame, expression)`,
  `stop`, and an event stream of `DebugEvent` (`Paused`, `Resumed`,
  `BreakpointState`, `Output`, `Ended`). The CDP backend is the first
  implementation; the scripted **fake backend** (`run-debug::fake`,
  `testing` feature, next to the run fake) is the second and is what UI
  tests and core tests without Node use.
- **Log points.** `logpoint_condition(message) -> String` turns IntelliJ
  style `text {expr} text` into a condition expression that prints and
  never pauses: `(()=>{try{console.log(`…${expr}…`)}catch(e){}return false})()`.
  The message's backticks, `${` and backslashes are escaped; a `{expr}`
  is the only way to evaluate anything. Pure, unit-tested including
  hostile input. The output goes to the debuggee's stdout, so it appears
  in the Run tab, in order with the program's own output.

#### 4.10.3 Source maps (`run-debug`, no new dependency)

TypeScript breakpoints are set on the generated JavaScript, so a map is
needed in both directions: original → generated to set a breakpoint, and
generated → original to show frames and the execution line.

**Where.** A module `run-debug::sourcemap`: decode a Source Map v3 JSON
(`version`, `sources`, `sourceRoot`, `names` unused, `mappings`), keep
decoded segments sorted for binary search, and answer `original_for(line,
col)` and `generated_for(source, line, col)` (for a line with no mapping,
the next line that has one, which is what makes a breakpoint on a blank
line move down). Pure functions over a string; the reading of `.map`
files is in `core`.

**Why no dependency.** The format is small and specified (base64 VLQ
segments of five fields), the decoder is about 150 lines with tests from
real `tsc` output, and the crates that do it either pull a larger tree
(`sourcemap`, BSD-3-Clause, with `debugid`, `scroll`, `rustc_version`) or
sit in the oxc release train we pin by exact version (`oxc_sourcemap`, MIT).
Writing it ourselves keeps `cargo deny` surface unchanged. If M5b-4 finds
the decoder is not small after all, it stops and proposes
`oxc_sourcemap` instead of growing it.

**Which maps.** For each `Debugger.scriptParsed` whose `sourceMapURL` is
set, core loads the map once: an inline `data:application/json;base64,…`
URL (what `tsx`, `ts-node` and `--enable-source-maps` pipelines produce)
or a file next to the script (`file:` URL or relative path). Limits: a map
file larger than 50 MB or outside the project folder and the script's own
directory is not read. `sources` are resolved against the map's URL and
`sourceRoot` and normalized; an original path that resolves outside the
project folder is shown as a frame but never opened by the editor (the
`open_location` rule for run output links). `sourcesContent` is ignored:
Ferrule shows the file on disk, and warns "source differs from the running
code" when its mtime is newer than the session start.

**Breakpoints in `.ts` files.** A breakpoint on `src/foo.ts:12` is
resolved against every loaded script whose map lists `src/foo.ts`; the
generated position goes to `Debugger.setBreakpointByUrl` using the
script's URL (escaped as a regular expression, so a module loaded
later under the same URL is also covered). Modules load lazily, so a
breakpoint stays `Unverified { "not loaded yet" }` until a matching
`scriptParsed` arrives, and is then resolved and verified without the
user doing anything. `.js` files with no map use their own URL directly.
Paths are compared after `file://` URL decoding with drive-letter and
case rules per OS (Windows: case-insensitive and `\` ≡ `/`; macOS:
case-insensitive; Linux: exact).

**Limits, stated honestly.** Variable names are not mapped back
(`names` is not used): a minifier's renamed locals show their generated
names. A map that lists one source in several generated files resolves
the breakpoint in each. Debugging code run by a bundler's dev server
(Vite, webpack) through other transports is out of M5b.

#### 4.10.4 IPC: commands, events and generated types

All types live in `crates/ipc` with specta derives, and the TypeScript
comes from `gen:ipc`; nothing is hand-written in `src/`. Commands (all
async, cancellable, returning `IpcError` as the other commands do):

| Command                                                  | Result                                                 |
| -------------------------------------------------------- | ------------------------------------------------------ |
| `start_debug(config_id, size)`                           | `StartDebug::Started { session, run }` or `NeedsTrust` |
| `debug_resume/step_over/step_into/step_out/pause(sess)`  | `()` (the new state arrives as an event)               |
| `stop_debug(session)`                                    | `()` (ends the debuggee tree; the run tab stays)       |
| `breakpoints()`                                          | all breakpoints with their current state               |
| `toggle_breakpoint(path, line)`                          | the breakpoint, or `None` if it was removed            |
| `update_breakpoint(id, condition, log_message, enabled)` | the breakpoint                                         |
| `remove_breakpoint(id)`, `remove_all_breakpoints()`      | `()`                                                   |
| `debug_select_frame(session, frame)`                     | the frame's `Scope`s                                   |
| `debug_variables(session, variables_ref, start, count)`  | `Vec<Variable>`                                        |
| `debug_evaluate(session, frame, expression)`             | `Variable` or an error message (from the debuggee)     |
| `watches()`, `add_watch(expr)`, `remove_watch(i)`        | watch expressions with their last values               |

`start_debug` takes the same path as `start_run`: it saves unsaved
documents, re-reads the configurations, applies trust (the dialog shows the
command with `--inspect-brk=127.0.0.1:<random port>`; the port is the only
part that differs from what runs, and the dialog says so) and then spawns
the PTY run, so the output, Stop and the process-tree cleanup are the M5a
ones. Events: one tagged `debug-event` carrying `session` and a variant
`Started`, `Paused { reason, frames, selected }`, `Resumed`,
`BreakpointsChanged { breakpoints }` (verification results, also when no
session is running), `Console { kind, text }` (evaluate results and
`console.*` from `Runtime.consoleAPICalled`, to the Debug window's
console; the program's stdout stays in the Run tab) and `Ended { exit_code,
stopped }`. Watch values are part of `Paused`, evaluated by Rust on each
stop. A `debug-event` stream with a monotonically increasing `seq` lets
the view drop stale events after a quick Resume/Pause. A new `ToggleDebugWindow`
and the debug actions are added to `KeyAction` in `crates/settings` and
reach the view through the generated bindings, like `toggleRunWindow`.

#### 4.10.5 Breakpoint persistence and editing

Breakpoints are stored per project, in Ferrule's data folder next to trust
(`projects/<hash>.json`, a `breakpoints` array, versioned, sorted by path
then line), not in the project: they are personal and machine-local, and a
committed breakpoint is a noise diff. Watches go in the same file. Paths
are project-relative. A path that no longer exists is kept but shown
unverified (a branch switch should not erase them), and is dropped by
"Remove all breakpoints".

**Following edits.** The buffer in Rust is the authority, so the lines of a
document's breakpoints are shifted in Rust on every applied change (insert
or delete lines above shift it; deleting the line removes the breakpoint),
through `BreakpointStore::shift_for_edit(path, start_line, removed_lines,
inserted_lines)` called from the edit path in `core`. The view receives
`BreakpointsChanged` and redraws the gutter; it keeps no line numbers of
its own. External changes on disk (a Git checkout) reload the document
through the existing path and keep the lines as they are. While a session
runs, a changed line is pushed to the debugger (remove + set); because
runs save documents first, the debugger always sees the saved text.

#### 4.10.6 Frontend (thin)

Functional only, new panels in the bottom tool window; M6 does the visuals.
No debugger logic in TypeScript: the view sends commands and draws events.

- **Gutter.** A breakpoint marker column in the editor view (a
  CodeMirror gutter, fed by `BreakpointsChanged`, next to the test-block
  markers): click toggles through `toggle_breakpoint`; the marker shows
  enabled, disabled, unverified (hollow) and conditional/log-point
  variants; right-click opens a small popup form for the condition and log message
  (it posts `update_breakpoint`).
- **Execution line.** On `Paused` the view opens the selected frame's file
  (an `open_location` that refuses paths outside the project) and sets a
  line decoration; `Resumed`/`Ended` clears it. Selecting another frame
  moves it (with a lighter style for non-top frames).
- **Debug tool window** (Alt+5, macOS Cmd+5), tabs: Frames (the call
  stack, virtualized, library frames dimmed, click selects and jumps),
  Variables (a lazy tree: scopes first, children on expand via
  `debug_variables`, virtualized), Watches (add, remove, values from the
  last stop) and Console (evaluate input, output lines). The toolbar has
  Resume, Step Over, Step Into, Step Out, Pause and Stop. Inline values
  are not done in M5b unless M5b-9 finds them cheap (they would need the
  scopes of the top frame and a tree-sitter pass for identifiers); the
  roadmap item says "if cheap".
- **Keymap** (IntelliJ preset, `crates/settings`; macOS in parentheses
  where it differs): Debug (the selected configuration) Shift+F9 (Ctrl+D);
  Resume F9 (Cmd+Alt+R); Step Over F8; Step Into F7; Step Out Shift+F8;
  Toggle Breakpoint Ctrl+F8 (Cmd+F8); Stop Ctrl+F2 (Cmd+F2), as in M5a;
  Pause has no default key, only Go to Action. Keys typed while a run's
  output has the focus go to the program except the run actions; the debug
  actions join that exception list (`DEBUG_ACTIONS` beside `RUN_ACTIONS`),
  so F8 works while the Run tab is focused. Defaults are tested against
  the existing "every action has exactly one binding" test.

#### 4.10.7 Safety and cleanup

- **Loopback only.** The inspector is started with an explicit
  `127.0.0.1:<port>`, never the bare `--inspect-brk` (which listens on the
  fixed port 9229) and never `0.0.0.0`. The client refuses any other host
  (`Refused`), so a hostile `webSocketDebuggerUrl` in `/json/list` cannot
  redirect it. The port is random per session; the WebSocket path is
  Node's own random UUID. Other local users of the machine can still
  connect to a loopback port; that is the same exposure as every
  `--inspect` and is documented, not hidden.
- **No inspector flags from the project.** The configuration's `env` must
  not contain `NODE_OPTIONS` with `--inspect*`; the arguments must not
  contain `--inspect*` for a debug run. Both are an error shown before
  anything starts, so a configuration cannot open a second inspector
  or move the port.
- **Trust applies.** Debugging executes project code, so it needs the same
  per-project trust as running (4.10, Safety).
- **Evaluate runs project code.** `debug_evaluate` and watches execute in
  the paused frame (`Debugger.evaluateOnCallFrame` with
  `throwOnSideEffect` off, like every debugger); they exist only while
  the debuggee is paused in a trusted project, and never on hover without
  the user typing (no automatic hover evaluation in M5b).
- **No debuggee left running.** Stop and every error path end the session
  in this order: cancel the session's `CancellationToken`, send
  `Debugger.disable` (best effort, 1 s), close the socket, then end the
  process tree through the M5a Stop (SIGHUP then SIGKILL after 2 s on
  Unix; the Job Object on Windows). A debuggee paused at a breakpoint is
  killed like any other, never resumed first. If the socket closes first
  (the program ended), the session ends `Ended` with the run's exit code.
  A failure to connect within 10 s ends the session and the run with an
  error message, so a start that never attaches does not leave a Node
  process that waits for a debugger forever (`--inspect-brk` blocks until
  one attaches).
- **`close_project`** (and replacing the project) ends debug sessions
  first, then runs, then the rest, with the M4 Windows rule: it waits until
  the debug tasks, the sockets and the processes have ended and reports
  what did not. `Ended` events are sent before the runtime is dropped.
- **Test runners and child processes.** Only the process Ferrule starts is
  attached. Test frameworks that run tests in worker processes need
  options that keep the test in the debugged process (for Vitest a
  single-process pool, for Jest `--runInBand`, for Playwright `--debug`
  is not used: `--workers=1` plus the inspector on the runner); M5b-10
  checks each against the installed tool and says which are supported.
  Auto-attaching to child processes (`NodeWorker`/`Target.setAutoAttach`)
  is a follow-up, not M5b.

#### 4.10.8 Test plan

1. **`cdp-client`, offline.** A scripted fake CDP server (a tokio WebSocket
   listener on `127.0.0.1:0`, in the crate's `testing` support module,
   also used by core tests) answers expected requests and pushes events.
   Tests: id correlation with out-of-order replies, protocol errors,
   unknown events ignored, call timeout, cancellation while connecting and
   while a call is pending, peer close fails pending calls and ends the
   event stream, a flood of events with a slow reader, `discover` against a
   fake `/json/list` (and against a hostile one: non-loopback URL
   refused), refusal of non-loopback `connect`.
2. **`run-debug`, unit.** Breakpoint store (toggle, update, sorting, file
   round trip, `shift_for_edit` cases), `logpoint_condition` (escaping,
   hostile input), the session model, the source-map decoder against real
   `tsc` output and the specification's examples (multi-line, no mapping,
   `sourceRoot`, inline data URL, malformed input never panics), and URL ↔
   path conversion for POSIX and Windows paths (pure, on every OS).
3. **`core`, with the fake backend and the fake CDP server.** Session
   lifecycle, pause → event with frames, stepping, breakpoint verification
   on a late `scriptParsed`, edit shifting, persistence, `close_project`
   ends a paused session, and failure paths (connect refused, socket
   dropped, Stop while paused) leave no task and no process.
4. **Real Node** (skipped where `node` is missing, like the M5a tests):
   debug a small script: the `--inspect-brk` stop, a breakpoint hit with a
   condition, step over/into/out, variables with expansion, evaluate, Stop
   leaves no process (`kill -0` on Unix, the job on Windows), and a `.ts`
   breakpoint through a committed source-map pair in
   `fixtures/debug-app/`: a few lines of TypeScript with the `.js` and
   `.js.map` that `tsc` made once (kept as test input, with a README
   saying how they were made, so no build step is needed in tests).
5. **UI (Playwright) with a fake debug backend.** `--fake-debug` in the
   headless server (implies `--fake-runs`) scripts a session: gutter click
   toggles and shows verified/unverified, Debug shows the Debug window, a
   pause draws the execution line and the stack, F8/F7/Shift+F8/F9 move
   through the script, variables expand lazily, a watch updates, Stop
   clears everything. `FERRULE_E2E_REAL_RUNS=1` runs a smaller spec
   against real Node.
6. **Checks.** Every slice passes the commands in `CLAUDE.md` with zero
   warnings; the macOS and Windows CI results are recorded in the roadmap
   like M5a's (never claimed before they have run).

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
| **M5** Run, debug, tests       | Run configurations (Node, npm scripts), Jest/Vitest/Playwright runners with gutter icons, debugger UI over CDP (Node). In two parts, M5a and M5b.                                                                                       | See [ROADMAP.md, M5](ROADMAP.md#m5-run-debug-tests).                                                                                                                       |

The rest of the feature scope in `CLAUDE.md` (HTTP client, local history,
frameworks, settings UI, themes, plugins, auto-update, i18n) follows M5 and
is planned in ROADMAP.md.
