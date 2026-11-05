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
