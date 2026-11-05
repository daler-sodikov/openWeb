//! M3b against the real `typescript-language-server`: completion (trigger
//! character, lazy details, auto-import on accept), signature help, Find
//! Usages, Rename (preview, one undo step per open document, closed files
//! rewritten, files changed since the preview skipped), quick fixes run as
//! server commands (`workspace/applyEdit`), Organize Imports, formatting,
//! oxc Sort Imports, and read-only library files (`lib.dom.d.ts`).
//!
//! Skipped, with the reason printed, when the server or Node.js is not on
//! `PATH` (fails instead when `FERRULE_REQUIRE_TS_SERVER` is set, as in CI).

#![allow(clippy::unwrap_used, clippy::expect_used)]
// Skip reasons are printed so a skipped test is visible in the output.
#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    ChangeTransaction, DocId, DocumentReplace, EditKind, EditorSelection, IpcErrorKind, NavKind,
    SelectionRange, ServerStateKind,
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
    if server.is_some() && node.is_some() {
        return true;
    }
    let message = "real-server tests need typescript-language-server and node on PATH; \
                   install with `bun add -g typescript-language-server typescript`";
    assert!(
        std::env::var_os("FERRULE_REQUIRE_TS_SERVER").is_none(),
        "{message}"
    );
    eprintln!("SKIPPED: {message}");
    false
}

const UTIL: &str = "\
export function helper(a: number, b: string): number {
  return a + b.length;
}
export const unusedThing = 1;
";

const USES: &str = "import { helper } from \"./util\";\nexport const used = helper(1, \"x\");\n";

const MAIN: &str = "\
import { unusedThing } from \"./util\";
const total = helper(2, \"y\");
export const doubled = total * 2;
const   messy={a:1};
export { messy };
";

const SIG: &str = "import { helper } from \"./util\";\nhelper(1, );\n";
const COMP: &str = "export const n = hel\nexport const m = Math.\n";
const DOM: &str = "export const t = document.title;\n";
const SORT: &str =
    "import { b } from \"./b\";\nimport { a } from \"a\";\nexport const x = [a, b];\n";

struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    root: PathBuf,
    session: Arc<Session>,
    events: Arc<Events>,
}

async fn fixture() -> Fixture {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let root = project.path().to_path_buf();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "lib": ["ES2022", "DOM"], "noEmit": true }, "include": ["src"] }"#,
    )
    .unwrap();
    for (name, text) in [
        ("util.ts", UTIL),
        ("uses.ts", USES),
        ("main.ts", MAIN),
        ("sig.ts", SIG),
        ("comp.ts", COMP),
        ("dom.ts", DOM),
        ("sort.ts", SORT),
    ] {
        std::fs::write(root.join("src").join(name), text).unwrap();
    }
    let events = Arc::new(Events::default());
    let session = Session::with_config(SessionConfig::new(data.path()), events.clone()).unwrap();
    session
        .open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    Fixture {
        _dirs: (project, data),
        root,
        session,
        events,
    }
}

fn utf16(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap()
}

/// UTF-16 offset of `needle` in `text`, plus `skip` units.
fn at(text: &str, needle: &str, skip: u32) -> u32 {
    utf16(&text[..text.find(needle).unwrap()]) + skip
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

/// Current text and version of an open document.
async fn current(f: &Fixture, path: &str) -> (DocId, u32, String) {
    let doc = f.session.open_document(path.into()).await.unwrap();
    (doc.doc_id, doc.version, doc.text)
}

/// Applies edits for an open document as one transaction, like the view.
async fn apply(f: &Fixture, replace: &DocumentReplace) {
    f.session
        .apply_changes(
            replace.doc_id,
            replace.version,
            vec![ChangeTransaction {
                edits: replace.edits.clone(),
                kind: EditKind::Other,
                selection_before: cursor(0),
                selection_after: cursor(0),
            }],
        )
        .await
        .unwrap();
}

/// Polls `check` every 200 ms for up to 60 s.
async fn poll<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..300 {
        if let Some(value) = check().await {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out after 60 s waiting for {what}");
}

fn path_text(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[tokio::test]
async fn m3b_features_against_typescript_language_server() {
    if !server_available() {
        return;
    }
    let f = fixture().await;
    let s = &f.session;
    let (util, util_v, _) = current(&f, "src/util.ts").await;
    poll("the server to load the project", || async {
        f.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::ServerStatus(st) if st.state == ServerStateKind::Ready))
            .then_some(())
    })
    .await;

    // Completion: a word prefix lists a symbol of another module; details
    // load lazily; accepting adds the import.
    let (comp, comp_v, _) = current(&f, "src/comp.ts").await;
    let caret = at(COMP, "hel\n", 3);
    let list = poll("completion with helper", || async {
        let list = s.completion(comp, comp_v, caret, None).await.ok()??;
        list.items
            .iter()
            .any(|i| i.label == "helper")
            .then_some(list)
    })
    .await;
    assert_eq!(list.from, at(COMP, "hel\n", 0));
    assert_eq!(list.items[0].label, "helper", "ranked first for `hel`");
    let item = list.items.iter().find(|i| i.label == "helper").unwrap();
    let details = s
        .completion_details(list.session, item.index)
        .await
        .unwrap();
    assert!(
        details.detail.as_deref().unwrap_or("").contains("helper"),
        "{details:?}"
    );
    let edit = s
        .accept_completion(comp, comp_v, list.session, item.index, caret)
        .await
        .unwrap();
    assert!(
        edit.edits
            .iter()
            .any(|e| e.insert.contains("import { helper } from \"./util\"")),
        "auto-import edit: {edit:?}"
    );
    assert!(edit.edits.iter().any(|e| e.insert.starts_with("helper")));
    // Filtering ranks again in Rust without asking the server.
    let filtered = s
        .filter_completion(list.session, "help".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(filtered.items[0].label, "helper");
    // A trigger character: members of Math after `.`.
    let dot = at(COMP, "Math.", 5);
    let members = s
        .completion(comp, comp_v, dot, Some(".".into()))
        .await
        .unwrap()
        .unwrap();
    assert!(
        members.items.iter().any(|i| i.label == "max"),
        "Math members"
    );

    // Signature help inside `helper(1, |)`: second parameter active.
    let (sig, sig_v, _) = current(&f, "src/sig.ts").await;
    let help = s
        .signature_help(sig, sig_v, at(SIG, "1, )", 3), Some(",".into()))
        .await
        .unwrap()
        .expect("signature help");
    assert!(
        help.label.contains("helper(a: number, b: string)"),
        "{help:?}"
    );
    assert_eq!(help.active_parameter, Some(1));
    let b = help.parameters[1];
    assert_eq!(&help.label[b.from as usize..b.to as usize], "b: string");

    // Find Usages of `helper` from its declaration: closed files included.
    let usages = s
        .find_usages(util, util_v, at(UTIL, "helper", 1))
        .await
        .unwrap();
    assert_eq!(usages.symbol, "helper");
    let uses = usages
        .groups
        .iter()
        .find(|g| g.path == "src/uses.ts")
        .unwrap();
    assert_eq!(uses.usages.len(), 2, "import and call");
    assert!(uses.usages[1].preview.contains("helper(1, \"x\")"));
    assert!(usages.groups.iter().any(|g| g.path == "src/util.ts"));

    // Quick fix run as a server command (workspace/applyEdit): add the
    // missing import of `helper` to main.ts.
    let (main, main_v, _) = current(&f, "src/main.ts").await;
    let helper_at = at(MAIN, "helper(2", 1);
    let actions = poll("an import quick fix", || async {
        let actions = s
            .code_actions(main, main_v, helper_at, helper_at)
            .await
            .ok()?;
        actions
            .into_iter()
            .find(|a| a.kind == "quickfix" && a.title.contains("import"))
    })
    .await;
    let result = s.apply_code_action(actions.id).await.unwrap();
    assert_eq!(result.open_documents.len(), 1, "{result:?}");
    apply(&f, &result.open_documents[0]).await;
    let (_, main_v, text) = current(&f, "src/main.ts").await;
    assert!(
        text.lines().next().unwrap().contains("helper"),
        "import updated: {text}"
    );

    // Organize Imports removes the unused binding.
    let result = s.organize_imports(main, main_v).await.unwrap();
    apply(&f, &result.open_documents[0]).await;
    let (_, main_v, text) = current(&f, "src/main.ts").await;
    assert!(
        text.starts_with("import { helper } from \"./util\";\n"),
        "{text}"
    );
    assert!(!text.contains("unusedThing"), "{text}");

    // Reformat Code with the server's formatter.
    let result = s.format_document(main, main_v).await.unwrap();
    apply(&f, &result.open_documents[0]).await;
    let (_, _, text) = current(&f, "src/main.ts").await;
    assert!(text.contains("const messy = { a: 1 };"), "{text}");

    // Sort Imports in-process (oxc): packages before relative paths.
    let (sort, sort_v, _) = current(&f, "src/sort.ts").await;
    let result = s.sort_imports(sort, sort_v).await.unwrap();
    apply(&f, &result.open_documents[0]).await;
    let (_, _, text) = current(&f, "src/sort.ts").await;
    assert!(
        text.starts_with("import { a } from \"a\";\nimport { b } from \"./b\";\n"),
        "{text}"
    );

    // Rename: a closed file changed after the preview is not overwritten.
    let (_, util_v, _) = current(&f, "src/util.ts").await;
    let target = s
        .prepare_rename(util, util_v, at(UTIL, "helper", 1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.placeholder, "helper");
    let stale = s
        .rename_preview(util, util_v, at(UTIL, "helper", 1), "assist".into())
        .await
        .unwrap();
    let preview_uses = stale
        .files
        .iter()
        .find(|p| p.path == "src/uses.ts")
        .unwrap();
    assert_eq!(preview_uses.count, 2);
    let changed = format!("{USES}// edited elsewhere\n");
    std::fs::write(f.root.join("src/uses.ts"), &changed).unwrap();
    let result = s.apply_refactoring(stale.id).await.unwrap();
    assert!(
        result.skipped.iter().any(|k| k.path == "src/uses.ts"),
        "{result:?}"
    );
    assert_eq!(
        path_text(&f.root, "src/uses.ts"),
        changed,
        "not overwritten"
    );

    // Rename for real: open documents get one transaction each (one undo
    // step), the closed file is rewritten.
    let preview = s
        .rename_preview(util, util_v, at(UTIL, "helper", 1), "assist".into())
        .await
        .unwrap();
    let result = s.apply_refactoring(preview.id).await.unwrap();
    assert!(result.skipped.is_empty(), "{result:?}");
    assert!(result.files_written.iter().any(|p| p == "src/uses.ts"));
    for replace in &result.open_documents {
        apply(&f, replace).await;
    }
    assert!(path_text(&f.root, "src/uses.ts").contains("assist(1, \"x\")"));
    let (_, util_v, text) = current(&f, "src/util.ts").await;
    assert!(text.starts_with("export function assist("), "{text}");
    let undone = s.undo(util, util_v).await.unwrap().unwrap();
    assert_eq!(undone.updates.len(), 1, "one undo step");
    let (_, _, text) = current(&f, "src/util.ts").await;
    assert_eq!(text, UTIL);

    // Library code: `document` is declared in lib.dom.d.ts outside the
    // project; it opens read-only and refuses edits.
    let (dom, dom_v, _) = current(&f, "src/dom.ts").await;
    let targets = s
        .goto_definition(dom, dom_v, at(DOM, "document", 2), NavKind::Definition)
        .await
        .unwrap();
    let lib = targets
        .iter()
        .find(|t| t.path.ends_with("lib.dom.d.ts"))
        .unwrap_or_else(|| panic!("{targets:?}"));
    assert!(!lib.in_project);
    let opened = s.open_external(lib.path.clone()).await.unwrap();
    assert!(opened.read_only);
    assert_eq!(opened.language.as_deref(), Some("TypeScript"));
    let refused = s
        .apply_changes(
            opened.doc_id,
            opened.version,
            vec![ChangeTransaction {
                edits: vec![ferrule_ipc::TextEdit {
                    from: 0,
                    to: 0,
                    insert: "x".into(),
                }],
                kind: EditKind::Other,
                selection_before: cursor(0),
                selection_after: cursor(0),
            }],
        )
        .await
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Conflict);
    // Navigation works inside the library file too.
    let decl = at(&opened.text, "declare var document: Document", 13);
    let inside = s
        .goto_definition(opened.doc_id, opened.version, decl, NavKind::TypeDefinition)
        .await
        .unwrap();
    assert!(
        !inside.is_empty(),
        "type definition from inside lib.dom.d.ts"
    );
}
