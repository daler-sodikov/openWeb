# Performance benchmarks

`ferrule-perf` measures the budgets in `CLAUDE.md` from **outside** the app:
it launches the release binary as a separate process and only observes the
screen (X11 `GetImage`), injects input (XTest) and reads the process tree's
CPU and memory from `/proc`. Linux/X11 only for now; macOS and Windows are
in the backlog (`docs/ROADMAP.md`).

## Run

```bash
bun tauri build --no-bundle                      # release build of the app
cargo run -p ferrule-perf --release -- --files 1000 --keys 30
```

`DISPLAY` must point to an X server: your desktop (on Wayland the app runs
through XWayland, `GDK_BACKEND=x11` is set by the harness) or a virtual one
(`xvfb-run -a cargo run -p ferrule-perf --release`). The app window appears
while it runs; do not touch mouse or keyboard meanwhile.

## What is measured

| Metric              | Method                                                                                                                                                     |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Window with project | Spawn → the project tree's pixels appear in the window. Cold = empty app data (no index cache), warm = second launch.                                      |
| Ready               | Spawn → CPU of the whole process tree (app + WebKit processes) stays below 5 % of one core for 1 s (that second is not counted).                           |
| Memory              | Sum of PSS (proportional set size, shared pages split fairly) of the process tree after ready; RSS sum for reference.                                      |
| Idle CPU            | Process-tree CPU averaged over 10 s after ready.                                                                                                           |
| Typing latency      | XTest key press → the glyph is on screen (pixels differ from every baseline frame, so caret blinking is ignored). In a TypeScript file, 1000-file project. |
| Quit                | `WM_DELETE_WINDOW` sent → process exited.                                                                                                                  |

The project (~1000 TS/TSX/CSS/JSON files) is generated deterministically in a
temp directory each run. Results go to stdout as a Markdown table; recorded
runs, with machine details, are in [RESULTS.md](RESULTS.md). Only numbers
measured with this tool are published there.
