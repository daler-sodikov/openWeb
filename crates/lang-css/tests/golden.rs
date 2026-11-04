//! Golden highlight tests: every `tests/golden/<name>` input is highlighted
//! and compared with `tests/golden/<name>.golden`. Run with
//! `UPDATE_GOLDEN=1 cargo test` to rewrite the expectations, then review the
//! diff by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use std::fs;
use std::path::Path;

#[test]
fn golden_highlights() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden");
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "golden") {
            continue;
        }
        let def = ferrule_lang_css::languages()
            .into_iter()
            .find(|d| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| d.extensions.contains(&e))
            })
            .unwrap_or_else(|| panic!("no language for {}", path.display()));
        let input = fs::read_to_string(&path).unwrap();
        let actual = ferrule_syntax::render_highlights(def, &input).unwrap();
        let golden = path.with_file_name(format!(
            "{}.golden",
            path.file_name().unwrap().to_string_lossy()
        ));
        if update {
            fs::write(&golden, &actual).unwrap();
        } else {
            let expected = fs::read_to_string(&golden).unwrap_or_else(|_| {
                panic!("missing {}; run with UPDATE_GOLDEN=1", golden.display())
            });
            assert_eq!(actual, expected, "golden mismatch for {}", path.display());
        }
        checked += 1;
    }
    assert!(checked > 0, "no golden inputs found");
}
