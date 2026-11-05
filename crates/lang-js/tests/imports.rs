//! Golden tests for the in-process import analysis: `tests/imports-golden/
//! <name>.tsx` → `<name>.tsx.sorted` (Sort Imports) and `<name>.tsx.unused`
//! (`line:column names`, one unused import per line). Run with
//! `UPDATE_GOLDEN=1 cargo test` to rewrite the expectations, then review.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use ferrule_lang_js::{sort_imports, unused_imports};

fn line_col(text: &str, byte: usize) -> String {
    let before = &text[..byte];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("{line}:{column}")
}

#[test]
fn golden_imports() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/imports-golden");
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("tsx") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let sorted = match sort_imports(&text, &path) {
            Some(edit) => format!("{}{}{}", &text[..edit.start], edit.text, &text[edit.end..]),
            None => text.clone(),
        };
        let unused: String = unused_imports(&text, &path)
            .iter()
            .map(|u| format!("{} {}\n", line_col(&text, u.start), u.names.join(", ")))
            .collect();
        for (suffix, actual) in [("sorted", &sorted), ("unused", &unused)] {
            let golden = path.with_extension(format!("tsx.{suffix}"));
            if update {
                std::fs::write(&golden, actual).unwrap();
            } else {
                let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
                    panic!("missing {}; run with UPDATE_GOLDEN=1", golden.display())
                });
                assert_eq!(
                    &expected,
                    actual,
                    "{} differs from {}",
                    path.display(),
                    golden.display()
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 0, "no golden inputs in {}", dir.display());
}

#[test]
fn broken_code_reports_nothing() {
    let path = Path::new("broken.ts");
    let text = "import { a } from \"a\";\nconst = ;\n";
    assert!(unused_imports(text, path).is_empty());
    assert!(sort_imports(text, path).is_none());
}

#[test]
fn comments_between_imports_are_left_alone() {
    let path = Path::new("c.ts");
    let text = "import { b } from \"b\";\n// keep me here\nimport { a } from \"a\";\nuse(a, b);\n";
    assert!(sort_imports(text, path).is_none());
}
