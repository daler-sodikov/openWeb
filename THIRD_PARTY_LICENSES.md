# Third-party licenses

Ferrule is Apache-2.0 licensed. This file records the licenses of the
third-party components it depends on, and every license exception we accept.

The Rust license policy is enforced by `cargo deny check` (see `deny.toml`):
only permissive licenses are allowed, and any exception must be listed here
with a reason. A complete, generated list of all transitive dependencies will
be produced for release bundles (tracked in `docs/ROADMAP.md`).

## Copied code

None. No third-party source code is copied into this repository.

## Rust: direct dependencies

| Crate                                         | License           | Used for                           |
| --------------------------------------------- | ----------------- | ---------------------------------- |
| `tauri`, `tauri-build`                        | Apache-2.0 OR MIT | Desktop shell, webview, IPC        |
| `tauri-specta`, `specta`, `specta-typescript` | MIT               | Generating TypeScript IPC bindings |
| `serde`, `serde_json`                         | MIT OR Apache-2.0 | Serialization                      |
| `thiserror`, `anyhow`                         | MIT OR Apache-2.0 | Error handling                     |
| `tracing`, `tracing-subscriber`               | MIT               | Logging                            |
| `tempfile` (tests only)                       | MIT OR Apache-2.0 | Temporary files in tests           |

## Rust: accepted exceptions

| Crate                                                      | License | Pulled in by                  | Reason                                                                                                                                                                  |
| ---------------------------------------------------------- | ------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` | MPL-2.0 | `tauri-utils` → `dom_query`   | Required by Tauri v2 for HTML processing at build time and runtime. Used unmodified; source is available on crates.io, which satisfies MPL-2.0 for binary distribution. |
| `option-ext`                                               | MPL-2.0 | `tauri` → `dirs` → `dirs-sys` | Required by Tauri v2. Used unmodified; same reasoning.                                                                                                                  |

## Rust: accepted advisories

| Advisory          | Crate                             | Reason                                                                   |
| ----------------- | --------------------------------- | ------------------------------------------------------------------------ |
| RUSTSEC-2024-0370 | `proc-macro-error` (unmaintained) | Compile-time only, via gtk3-rs `glib-macros`, which Tauri uses on Linux. |
| RUSTSEC-2024-0436 | `paste` (unmaintained)            | Compile-time only proc macro, via `specta`.                              |

## JavaScript: runtime dependencies (bundled into the app)

| Package           | License           |
| ----------------- | ----------------- |
| `@tauri-apps/api` | Apache-2.0 OR MIT |

## JavaScript: development tools (not distributed)

| Package                                                                          | License           |
| -------------------------------------------------------------------------------- | ----------------- |
| `@tauri-apps/cli`                                                                | Apache-2.0 OR MIT |
| `vite`                                                                           | MIT               |
| `typescript`                                                                     | Apache-2.0        |
| `eslint`, `@eslint/js`, `typescript-eslint`, `eslint-config-prettier`, `globals` | MIT               |
| `prettier`                                                                       | MIT               |
| `vitest`, `happy-dom`                                                            | MIT               |
| `@playwright/test`                                                               | Apache-2.0        |
| `@types/node`                                                                    | MIT               |

## Artwork

The application icon (`docs/media/app-icon.svg` and the generated files in
`src-tauri/icons/`) is original artwork made for this project, licensed with
the project under Apache-2.0.
