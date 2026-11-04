//! Tauri build script: generates the application context and platform
//! resources (icons, Windows manifest).

use std::env;
use std::path::PathBuf;

fn main() {
    let mut attributes = tauri_build::Attributes::new();

    // On Windows MSVC, tauri-build normally embeds the application manifest
    // into the app executable only. Test binaries that link Tauri then fail to
    // start with STATUS_ENTRYPOINT_NOT_FOUND because they miss the Common
    // Controls v6 dependency. Embedding the manifest through linker arguments
    // applies it to every binary of this crate, tests included.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
        let manifest = manifest_dir.join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        println!("cargo:rustc-link-arg=/WX");
    }

    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {err:#}");
    }
}
