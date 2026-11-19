# Proposal: move the remaining view logic into Rust

Status: proposal, no code changed. Task B1-T08.

CLAUDE.md, architecture rule 1: "No business logic, no language analysis,
no file IO in TypeScript." Rule 2: "Never hand-write a type that exists on
the Rust side." The ROADMAP backlog lists five places that may break these
rules. This document looks at each one, says what should move, and
recommends an order. Moves that are not worth doing are marked **stays**.

## Summary

| Module                                  | Verdict                                     | When        |
| --------------------------------------- | ------------------------------------------- | ----------- |
| `src/app/problems.ts`                   | Move sorting and counts to Rust             | Now         |
| `src/run/format.ts` (`visibleRows`)     | Move per-node status totals to Rust         | After `m5a` |
| `src/run/format.ts` (`summaryText` etc) | **Stays** (pure formatting)                 | n/a         |
| `src/tree/tree-model.ts`                | **Stays** (view state)                      | n/a         |
| `src/git/status.ts` (`isNewer`)         | **Stays**; optional later cleanup           | Not now     |
| `src/ipc/result.ts`                     | Keep the helper, guard the type with a test | Now (tiny)  |

Recommended order: 1. `result.ts` guard, 2. problems, 3. test-tree totals.
The rest stays.

## How this was checked

I read the five modules and the Rust code that feeds them
(`crates/core/src/lsp.rs`, `crates/core/src/git.rs`,
`crates/core/src/session.rs`, `crates/ipc/src/lib.rs`, `src-tauri`).
`src/run/format.ts` and `TestTreeNode` exist only on the unmerged `m5a`
branch, so I read them there (`git show origin/m5a:...`). This branch is
based on `origin/main`, as the task asks. Nothing was built or run. The
payload and cost statements below come from reading the code, not from
measurement, and must not be quoted as numbers (CLAUDE.md, performance
budget).

## 1. `src/app/problems.ts`

**What it computes today.**

- `problemRows`: sorts files by path, sorts problems inside a file by
  severity, then line, then column, and inserts a header row per file.
- `problemCounts`: errors, warnings and "other" across all files, for the
  status bar.
- `ProblemsPanel.render` also derives the title total.

The first two are product rules (what order users see problems in, what
the status bar counts), so they belong in Rust.

**Where it goes.** `ferrule-core`, `crates/core/src/lsp.rs`. It already
owns the diagnostics map and builds `ipc::FileProblems` in
`file_problems`. That function is the natural place to sort.

- Sort each file's problems by severity, line, column in `file_problems`.
  Both `problems()` (full list) and the `ProblemsChanged` event then arrive
  sorted. The view no longer sorts.
- Add `ProblemCounts { errors: u32, warnings: u32, other: u32 }` to
  `ferrule-ipc` and compute it from the diagnostics map. Send it with the
  event: `ProblemsChanged { file: FileProblems, counts: ProblemCounts }`.
  `problems()` returns the same pair for the initial load. The total in
  the panel title is `errors + warnings + other`, shown as-is.
- Counts cover all files, so they cannot be derived from one file's
  event. That is why they must come from Rust and not be re-summed.

**File order stays in the view, deliberately.** An update touches one file;
to insert it in the right place the view needs a path sort. Options:

1. The view keeps `[...keys].sort()` (one line, code-unit order, the same
   as Rust's `BTreeMap`). Recommended: it is display order, not a rule.
2. Rust sends the whole ordered snapshot on every change. Rejected: with
   hundreds of files and diagnostics changing while the user types, the
   payload is far larger than one file.

**What the view keeps.** Header rows (`kind: "file"`), icons, the
"1 problem / n problems" wording, `line + 1` for display. These are
formatting. The `ProblemRow` flattening for the virtual list is a
presentation shape and stays.

**Virtual list and payload.** No change to the rows the list gets, so the
`VirtualList` is untouched. The event grows by three integers. The
initial `problems()` call is as large as today.

**Later, only if needed.** For a project with tens of thousands of
problems, a paged `problem_rows(offset, limit)` command would stop the
view from holding every problem. Do not build it before a measurement
shows a problem.

**Tests.**

- Rust unit tests next to `file_problems` in `lsp.rs`: sort order with
  ties, counts across files, a cleared file lowers the counts.
- The IPC contract test (`src-tauri/tests/ipc_contract.rs`) covers the
  changed event shape.
- Vitest: remove the `problemRows` sorting and `problemCounts` tests, keep
  a test for the row flattening.
- e2e: `e2e/ui/lsp.spec.ts` already checks problems with the real server.
  Add an assertion that the status bar count matches after a fix is
  applied.

## 2. `src/run/format.ts` (on `m5a`)

Four kinds of logic live in this file, and they are not alike.

| Function                                    | Verdict                                                  |
| ------------------------------------------- | -------------------------------------------------------- |
| `statusGlyph`, `formatDuration`, `exitText` | **Stays.** Glyphs and wording are pure formatting.       |
| `summaryText`                               | **Stays.** Rust already sends `counts`; this words them. |
| `nodeKey`                                   | **Stays.** A view key for the collapsed set.             |
| `visibleRows` (the `matches` filter)        | Partly moves, see below.                                 |

**What moves.** `matches` walks the whole subtree for every node, on every
filter or collapse change, to decide whether a suite or file has a test
with a shown status. "Which statuses exist below this node" is a fact about
the results, not about the view.

- Add `subtree: TestCounts { passed, failed, skipped, running }` to
  `TestTreeNode` in `ferrule-ipc`. Compute it once where the tree is built,
  in `crates/run-debug/src/report.rs` (`counts()` already walks the tree
  there). The run-level `counts` tuple can then be derived from the roots.
- `visibleRows` keeps its walk, because the collapsed set and the chosen
  status filter are view state. It replaces the recursive `matches` with a
  check on `node.subtree`.

**Why not move `visibleRows` entirely.** The collapsed set and the filter
are chosen by the user per panel and change on every click. Sending each
toggle to Rust and back for a few hundred rows costs more than it saves,
and Rust would have to hold per-panel UI state.

**Virtual list and payload.** `TestTreeNode` grows by four integers per
node. For a run with several thousand tests that is small next to the
names, locations and failures already sent. No change to the list.

**Timing.** `m5a` is not merged. Changing `TestTreeNode` now would
conflict with it. Do this as a follow-up PR after `m5a` merges.

**Tests.** Rust unit test in `report.rs` using the existing fixtures
(`jest.handwritten.json`, `vitest.json`, `playwright.json`): totals of
nested suites, skipped-only suites, an empty file. Vitest: `visibleRows`
with a node whose `subtree` excludes the shown statuses. The contract test
covers the new field.

## 3. `src/tree/tree-model.ts`

`flattenTree` turns the loaded directories plus the expanded set into
visible rows. `visibleRange` computes the render window from the scroll
position.

**Verdict: stays.**

- The expanded set, the lazily loaded children and the scroll position are
  view state. Rust does not know what is expanded and should not.
- It is not analysis. It does no sorting and applies no rules; `list_dir`
  (`crates/core/src/session.rs`) already returns the entries in order with
  `ignored` set.
- Moving it would make every expand, collapse or scroll an IPC round trip,
  which works against the typing and scroll budgets for no gain.
- `visibleRange` is virtualization arithmetic (rule 4 says to virtualize
  lists; it does not say Rust owns the window).

**One thing to watch.** `TreeView.refreshPaths` has path logic: the parent
directory of a path, and `startsWith(path + "/")` to forget removed
subtrees. It is small and tied to the view's own caches, so it stays. If
the same prefix rules appear in a third place, move them into a shared
Rust helper and ship the result with the `fs-changed` event.

**Tests.** Unchanged: the existing `tree-model.test.ts`.

## 4. `src/git/status.ts`

Three things here.

- `isNewer(shown, next)`: one comparison, `next.generation > shown`.
  **Stays.** The rule that matters is Rust's: `git.rs` numbers every
  status (`next_generation`) and drops a refresh that finished after a
  newer one (`inner.status.generation > generation`). The view only has to
  ignore an answer that arrives out of order, and that order is decided by
  the IPC channel, which Rust cannot see.
- `branchLabel`: formats `branch` or `detached <head>`. **Stays.** It is
  wording over fields Rust already sends, and it will go through i18n.
- `GitStatusModel.classFor`: not in the task list, but worth noting. It
  runs `ignoredDirs.some(...)` with a prefix match per rendered row. With
  many ignored directories that is a loop inside the render path.

**Optional follow-up, not now.** Two changes would remove the need for
`isNewer` and the loop:

1. Stop using the pull command `gitStatus` for the first status and make
   the backend emit the current status when the listener attaches. With
   one channel there is no out-of-order answer.
2. Have Rust send a sorted, deduplicated `ignoredDirs` so the view can do
   a set lookup per ancestor instead of a scan.

Both change the event contract and the open-project flow. Only do them if
a profile shows `classFor` in the tree's render time.

**Tests.** Unchanged for now.

## 5. `src/ipc/result.ts`

Two different things share this file.

- `unwrap`, `BackendError`, `errorMessage` are runtime helpers. Code that
  throws in the view layer has no Rust equivalent. **Stays.**
- `IpcResult<T>` is the type that `tauri-specta` already writes into the
  generated `src/ipc/bindings.ts` as the return type of `typedError`
  (`{ status: "ok"; data: T } | { status: "error"; error: E }`). The
  hand-written copy duplicates that shape.

**What can be done.** The generated runtime does not export a named
`IpcResult`, so there is nothing to import today. I did not check whether
the exporter has a header or extra-type option that could emit one; that
would be the way to generate it and should be tried first. If it does not
work, do not fork the exporter. Add a compile-time guard instead:

```ts
// Fails to compile if the generated result shape drifts from IpcResult.
type Check =
  Awaited<ReturnType<typeof commands.listDir>> extends IpcResult<DirEntry[]> ? true : never;
```

`IpcResult` stays hand-written but cannot silently drift, which is what
rule 2 protects against.

**Tests.** The type check runs as part of `bun run typecheck`.

## Recommended order

1. **`result.ts` guard.** One small PR. No Rust, no contract change.
2. **Problems sorting and counts.** One PR: `ferrule-ipc` type, `lsp.rs`
   sorting and counts, event change, view simplification, tests. The only
   move that is both clearly a rule break and independent of unmerged work.
3. **Test-tree totals**, after `m5a` merges.

Leave `tree-model.ts` and `isNewer` alone. Revisit the Git `classFor` loop
only with a profile.

## Backlog entries to update after this is accepted

In `docs/ROADMAP.md`, replace the two backlog lines (Thin view, Typed IPC)
with the three items above, and note that `tree-model.ts`, `isNewer` and
`branchLabel` were reviewed and kept in the view, with the reason.
