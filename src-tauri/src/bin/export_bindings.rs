//! Writes the TypeScript IPC bindings to `src/ipc/bindings.ts`, or to the
//! path given as the first argument. Run through `bun run gen:ipc`.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os().nth(1).map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("src")
                .join("ipc")
                .join("bindings.ts")
        },
        PathBuf::from,
    );
    ferrule_app::export_bindings(&path)
}
