//! The session against the real `typescript-language-server` (found on
//! `PATH`, like the app does): diagnostics at the right UTF-16 positions
//! after edits with emoji and CJK text, hover, cross-file definition, type
//! definition and implementation.
//!
//! Skipped, with the reason printed, when the server or Node.js is not
//! installed. Install: `bun add -g typescript-language-server typescript`
//! (or npm), and Node.js 22 or newer.

#![allow(clippy::unwrap_used, clippy::expect_used)]
// Skip reasons are printed so a skipped test is visible in the output.
#![allow(clippy::print_stderr)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    ChangeTransaction, DocId, EditKind, EditorSelection, NavKind, SelectionRange, ServerStateKind,
    Severity, TextEdit,
};

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn server_available() -> bool {
    let server = ferrule_lsp_client::find_on_path("typescript-language-server");
    let node = ferrule_lsp_client::find_on_path("node");
    match (server, node) {
        (Some(_), Some(_)) => true,
        (server, node) => {
            let message = format!(
                "real-server tests need typescript-language-server ({}) and node ({}) on PATH; \
                 install with `bun add -g typescript-language-server typescript`",
                if server.is_some() { "found" } else { "missing" },
                if node.is_some() { "found" } else { "missing" },
            );
            // CI installs both; a missing server there is a setup error.
            assert!(
                std::env::var_os("FERRULE_REQUIRE_TS_SERVER").is_none(),
                "{message}"
            );
            eprintln!("SKIPPED: {message}");
            false
        }
    }
}

const UTIL: &str = "\
export function greet(name: string): string {
  return `hi ${name}`;
}

export interface Shape {
  area(): number;
}

export class Square implements Shape {
  area(): number {
    return 1;
  }
}
";

const MAIN: &str = "\
import { greet, type Shape } from \"./util\";

const 名前 = \"😀\"; const total: number = greet(名前);
let shape: Shape | undefined;
";

struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    session: Arc<Session>,
    events: Arc<Events>,
}

async fn fixture() -> Fixture {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let root = project.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "noEmit": true }, "include": ["src"] }"#,
    )
    .unwrap();
    std::fs::write(root.join("src/util.ts"), UTIL).unwrap();
    std::fs::write(root.join("src/main.ts"), MAIN).unwrap();
    let events = Arc::new(Events::default());
    let session = Session::with_config(SessionConfig::new(data.path()), events.clone()).unwrap();
    session
        .open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    Fixture {
        _dirs: (project, data),
        session,
        events,
    }
}

async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..600 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out after 60 s waiting for {what}");
}

fn utf16(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap()
}

/// UTF-16 offset of the first occurrence of `needle` (plus `skip` units).
fn at(text: &str, needle: &str, skip: u32) -> u32 {
    utf16(&text[..text.find(needle).unwrap()]) + skip
}

/// The text of the error diagnostics, once the server reported `expected`.
async fn error_texts(f: &Fixture, doc: DocId, text: &str, expected: usize) -> Vec<String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    for _ in 0..600 {
        let diagnostics = f.session.doc_diagnostics(doc).await.unwrap();
        let errors: Vec<String> = diagnostics
            .items
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| String::from_utf16_lossy(&units[d.from as usize..d.to as usize]))
            .collect();
        if errors.len() == expected {
            return errors;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out after 60 s waiting for {expected} error diagnostics");
}

#[tokio::test]
async fn typescript_language_server_end_to_end() {
    if !server_available() {
        return;
    }
    let f = fixture().await;
    let doc = f.session.open_document("src/main.ts".into()).await.unwrap();
    eventually("the server to be ready", || async {
        f.events.0.lock().unwrap().iter().any(
            |e| matches!(e, SessionEvent::ServerStatus(s) if s.state == ServerStateKind::Ready),
        )
    })
    .await;

    // The type error is reported on `total`, after an emoji on the same line.
    let errors = error_texts(&f, doc.doc_id, MAIN, 1).await;
    assert_eq!(errors, ["total"]);
    let problems = f.session.problems().await.unwrap();
    let main = problems.iter().find(|p| p.path == "src/main.ts").unwrap();
    assert_eq!(main.problems[0].line, 2);
    assert_eq!(
        main.problems[0].column,
        utf16("const 名前 = \"😀\"; const ")
    );
    assert_eq!(main.problems[0].source.as_deref(), Some("typescript"));

    // Insert CJK and emoji before the error: the diagnostic follows.
    let insert = "/* 漢字🎉 */ ";
    let from = at(MAIN, "const 名前", 0);
    let status = f
        .session
        .apply_changes(
            doc.doc_id,
            doc.version,
            vec![ChangeTransaction {
                edits: vec![TextEdit {
                    from,
                    to: from,
                    insert: insert.into(),
                }],
                kind: EditKind::Other,
                selection_before: cursor(from),
                selection_after: cursor(from),
            }],
        )
        .await
        .unwrap();
    let edited = MAIN.replacen("const 名前", &format!("{insert}const 名前"), 1);
    eventually("the diagnostic to move with the edit", || async {
        let diagnostics = f.session.doc_diagnostics(doc.doc_id).await.unwrap();
        diagnostics
            .items
            .iter()
            .any(|d| d.severity == Severity::Error && d.from == at(&edited, "total", 0))
    })
    .await;

    // Hover on `greet` (Markdown with highlighted TypeScript).
    let hover = f
        .session
        .hover(doc.doc_id, status.version, at(&edited, "greet(名前)", 1))
        .await
        .unwrap()
        .expect("hover for greet");
    assert!(hover.html.contains("greet"), "{}", hover.html);
    assert!(hover.html.contains("tok-"), "{}", hover.html);
    assert_eq!(hover.to - hover.from, utf16("greet"));

    // Cross-file definition.
    let definition = f
        .session
        .goto_definition(
            doc.doc_id,
            status.version,
            at(&edited, "greet(名前)", 2),
            NavKind::Definition,
        )
        .await
        .unwrap();
    assert_eq!(definition.len(), 1, "{definition:?}");
    assert_eq!(definition[0].path, "src/util.ts");
    assert_eq!((definition[0].line, definition[0].column), (0, 16));
    assert!(definition[0].preview.starts_with("export function greet"));

    // Type definition of `shape` → the Shape interface.
    let types = f
        .session
        .goto_definition(
            doc.doc_id,
            status.version,
            at(&edited, "shape:", 1),
            NavKind::TypeDefinition,
        )
        .await
        .unwrap();
    assert!(
        types.iter().any(|t| t.path == "src/util.ts" && t.line == 4),
        "{types:?}"
    );

    // Implementations of Shape → Square.
    let util = f.session.open_document("src/util.ts".into()).await.unwrap();
    let implementations = f
        .session
        .goto_definition(
            util.doc_id,
            util.version,
            at(UTIL, "Shape {", 1),
            NavKind::Implementation,
        )
        .await
        .unwrap();
    assert!(
        implementations
            .iter()
            .any(|t| t.path == "src/util.ts" && t.line == 8),
        "{implementations:?}"
    );
}

fn cursor(at: u32) -> EditorSelection {
    EditorSelection {
        ranges: vec![SelectionRange {
            anchor: at,
            head: at,
        }],
        main: 0,
    }
}
