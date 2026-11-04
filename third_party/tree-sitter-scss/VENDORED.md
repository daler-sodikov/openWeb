# Vendored: tree-sitter-scss 1.0.0

- Source: the `tree-sitter-scss` 1.0.0 package on crates.io
  (<https://github.com/tree-sitter-grammars/tree-sitter-scss>), MIT license
  (`LICENSE`, copied from the upstream repository because the package does
  not ship one).
- Why: the published build script always passes the GCC flag
  `-Wno-unused-parameter`, which MSVC rejects, so Windows builds fail.
  Upstream fixed this on its main branch, but no fixed version has been
  released.
- Change: `bindings/rust/build.rs` passes `/wd4100` on MSVC instead (the
  upstream fix). All other files are the published package unmodified
  (`grammar.js` and `src/grammar.json` are omitted; they are not used to
  build).
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`. Remove this
  directory and the patch once a fixed version is on crates.io.
