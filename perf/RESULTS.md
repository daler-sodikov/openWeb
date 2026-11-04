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
