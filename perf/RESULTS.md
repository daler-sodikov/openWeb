# Benchmark results

Only numbers measured with `ferrule-perf` (see [README.md](README.md)) are
recorded here, with the machine and display they were measured on.

## 2026-11-04 — M2b, Linux, Xvfb (software rendering)

Machine: 13th Gen Intel Core i5-1335U, 12 threads, 15.2 GiB RAM, Linux
7.2.11 (Arch), virtual X server `Xvfb` 1440×900 (no GPU, software
rendering), release build, generated project of 1000 files, `main.ts` open
in the editor.

| Metric                                     | Measured                                 | Budget   |            |
| ------------------------------------------ | ---------------------------------------- | -------- | ---------- |
| Window with project, cold (no index cache) | 453 ms                                   | < 1 s    | met        |
| Window with project, warm                  | 446 ms                                   | < 1 s    | met        |
| Ready (CPU settled), cold and warm         | not reached within 60 s                  | < 2 s    | **missed** |
| Memory, PSS of all processes               | 308 MiB (RSS 526 MiB)                    | < 250 MB | **missed** |
| Idle CPU (10 s average)                    | 21.3 % of a core                         | < 5 %    | **missed** |
| Typing latency (30 keys)                   | median 52.8 ms, p95 55.8 ms, max 59.6 ms | < 8 ms   | **missed** |
| Quit                                       | 16 ms                                    | < 0.5 s  | met        |

Follow-up measurements on the same setup (process-tree CPU over 10 s,
15 s after start, `fixtures/react-ts`):

| State                                      | ferrule | WebKitWebProcess | WebKitNetworkProcess |
| ------------------------------------------ | ------- | ---------------- | -------------------- |
| A file open in the editor (caret blinking) | 6.4 %   | 15.2 %           | 0.0 %                |
| No file open                               | 0.0 %   | 0.1 %            | 0.0 %                |

What this tells us (and what it does not):

- The idle CPU comes from the open editor, not from indexing or the
  watcher: with no editor open the app is idle at ~0 %. The most likely
  cause is the blinking caret repainted by WebKitGTK in software; that is
  not yet proven, nor measured on a GPU-composited desktop.
- "Ready" never settles because the benchmark opens a file at startup, so
  the same editor cost keeps CPU above the 5 % threshold.
- Typing latency on Xvfb includes software rendering and the screen-capture
  polling of the harness. It has not been measured on a real desktop yet.
- Memory: WebKit's processes are included (PSS counts shared pages once).

Next steps are tracked in `docs/ROADMAP.md` (backlog).

## 2026-11-05 — M3b typing latency, before and after

Same machine and Xvfb setup as above. `ferrule-perf --only typing --keys 30`
on the generated 1000-file project with `typescript-language-server` 6.0.3
on `PATH` (so the server runs in both builds). **Before** is a release build
of `ce5a3f7` (typing spans added, M3b not yet written: diagnostics and hover
active, no completion or signature help); **after** is `HEAD` with M3b.
Both builds were measured in alternating order in two rounds.

`comment` types at the end of a comment line; `code` types after
`export const typed = 1 + `, where every key asks for completion in the
M3b build.

Median per run (ms), 30 keys each:

| Stage                                  | Target  | Before, round 1 | Before, round 2 | After, round 1 | After, round 2 |
| -------------------------------------- | ------- | --------------- | --------------- | -------------- | -------------- |
| Key → glyph on screen (external)       | comment | 44.9            | 52.5            | 52.8           | 49.7           |
| Key → glyph on screen (external)       | code    | 42.0            | 36.9            | 43.0           | 43.2           |
| Key event → local echo frame (view)    | comment | 13              | 15              | 14             | 12             |
| Key event → local echo frame (view)    | code    | 11              | 10              | 10             | 10             |
| Key event → Rust confirmed edit (view) | comment | 4               | 5               | 5              | 5              |
| Key event → Rust confirmed edit (view) | code    | 7               | 5               | 5              | 5              |
| Key event → fresh highlights frame     | comment | 56              | 60              | 55             | 55             |
| Key event → fresh highlights frame     | code    | 55              | 58              | 54             | 56             |
| `apply_changes` in Rust                | both    | 0.02–0.03       | 0.02            | 0.02           | 0.02–0.03      |
| `highlights` in Rust                   | both    | 0.06–0.10       | 0.06–0.10       | 0.06–0.09      | 0.06–0.09      |

What this shows:

- No measurable regression from M3b. The same build varies by up to 8 ms
  between rounds in the external measurement (44.9 vs 52.5), which is larger
  than any before/after difference. The view's own stages, which vary less,
  are unchanged.
- Rust is not the bottleneck: buffer edit and highlighting take well under
  0.2 ms. Of the ~45–50 ms on screen, ~10–15 ms pass before WebKitGTK paints
  the local echo; the rest is software rendering on Xvfb and the harness
  polling the screen. The fresh-highlights frame (~55 ms) is dominated by the
  40 ms highlight debounce in the view.
- The 8 ms budget is still missed on this setup (backlog). Not measured on a
  GPU desktop, macOS or Windows.
- Not verified: that the completion popup was visible during the `code` run
  (the harness only reads pixels of the first line).

## 2026-11-20 — idle CPU with an open editor: the caret blink (B1-T03)

Same machine and setup as 2026-11-04: 13th Gen Intel Core i5-1335U, 12
threads, 15.2 GiB RAM, Linux 7.3.4 (Arch), `Xvfb` 1440×900×24 (no GPU,
software rendering), release builds (`bun tauri build --no-bundle`), generated
1000-file project, `main.ts` open. Command for every run:
`xvfb-run -a -s "-screen 0 1440x900x24" ferrule-perf --app <build> --files 1000 --keys 30`
(the full run; the numbers below are its idle-CPU, ready and typing rows).
Four builds of the same tree, each measured three times, in alternating order:

- **blink on**: `main` at `727de1c` (CodeMirror's CSS blink animation).
- **blink off**: only `drawSelection({ cursorBlinkRate: 0 })`. A diagnostic,
  not a candidate: the caret does not blink at all.
- **timer, opacity**: the caret blinks from a timer (`caretBlink`,
  `src/editor/theme.ts`), hiding the whole cursor layer with `opacity`.
- **timer, visibility** (the change in this PR): the same timer, hiding only
  the caret boxes with `visibility: hidden`.

| Build                  | Idle CPU, 10 s average (three runs) | Ready (CPU settled), warm (three runs) | Typing latency, median (three runs) |
| ---------------------- | ----------------------------------- | -------------------------------------- | ----------------------------------- |
| blink on (`main`)      | 21.1 %, 25.8 %, 23.2 %              | not reached in 60 s (all three)        | 53.3, 53.3, 52.8 ms                 |
| blink off (diagnostic) | 1.1 %, 1.2 %, 1.1 %                 | 1.2 s, 2.0 s, 1.3 s                    | 17.6, 19.4, 24.0 ms                 |
| timer, opacity         | 1.9 %, 2.1 %, 2.2 %                 | 25.3 s, 6.6 s, 16.3 s                  | 22.4, 21.2, 23.5 ms                 |
| timer, visibility      | 2.3 %, 1.9 %, 2.1 %                 | 4.2 s, 18.4 s, 6.0 s                   | 24.5, 22.1, 21.9 ms                 |

What this shows:

- **The caret blink is the cause of the idle CPU on this setup: confirmed.**
  Turning only the blink off takes the idle CPU from 21–26 % to ~1 % in all
  three rounds, and "Ready" from never to about 1–2 s. Typing latency also
  drops (53 → 18–24 ms; still above the 8 ms budget).
- The cost is not the repaint of a small caret: with the timer the caret
  still toggles twice per 1.2 s, and the idle CPU is ~2 %. The endless CSS
  `steps(1)` animation is what keeps WebKitGTK producing frames; a timer that
  wakes the page twice per cycle removes most of it. This is the likely
  mechanism, inferred from these four builds; the WebKit process was not
  profiled.
- The timer builds meet the idle-CPU budget (< 5 %) in all six runs. They do
  not meet "Ready" (< 2 s): it took 4–25 s, and varies much more between runs
  than the idle average does (probably each toggle is a small burst, and the
  harness waits for a full second below 5 %; not verified). The two timer
  variants cannot be told
  apart from these runs, on idle CPU or on "Ready".
- The caret looks and behaves as before (checked in Chromium with a
  throwaway Playwright test, not kept): hidden and visible for 600 ms each,
  visible right after typing, not left hidden when the editor loses focus,
  CodeMirror's CSS animation duration 0 s.

Limits of these numbers:

- One machine, Xvfb with software rendering. Not measured on a GPU-composited
  desktop, nor on macOS or Windows; the cost there may differ.
- A few short low-priority (`nice -n 19`, two jobs) `cargo` builds for another
  task ran on the same machine during some of the rounds. No timer or
  blink-off run moved visibly, but the 25.8 % "blink on" run is the highest
  of its group and may have been affected.
- Memory was 660–680 MiB PSS in these runs against 308 MiB on 2026-11-04. Not
  investigated here (outside this task); do not compare it with the earlier
  record.
