//! IPC contract test: the exported TypeScript bindings must contain every
//! registered command, DTO and constant. The other half of the contract is
//! `tsc` compiling the frontend against the freshly generated file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn ipc_contract() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    ferrule_app::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();

    for expected in [
        // commands (camelCase accessors) and their wire names
        "appInfo",
        "\"app_info\"",
        "reportFrontendReady",
        "\"report_frontend_ready\"",
        "openProject",
        "listDir",
        "openDocument",
        "applyChanges",
        "undo",
        "redo",
        "addCaret",
        "selectNextOccurrence",
        "selectAllOccurrences",
        "highlights",
        "saveDocument",
        "closeDocument",
        "initialProject",
        "keymap",
        "newLine",
        "reloadDocument",
        "diskChanged",
        "searchEverywhere",
        "recentFiles",
        "findInFiles",
        "replaceInFiles",
        // events
        "fsChanged",
        "indexStatus",
        "\"fs-changed\"",
        "\"index-status\"",
        "baseVersion",
        "docId",
        // DTOs
        "OpenedDocument",
        "ChangeTransaction",
        "RemoteUpdate",
        "TokenClass",
        "KeyStroke",
        "AppInfo",
        "FrontendReadyReport",
        "IpcError",
        "IpcErrorKind",
        "Platform",
        // fields keep the camelCase wire names
        "protocolVersion",
        "renderedAppName",
        "documentTitle",
        // constants
        "PROTOCOL_VERSION",
        "Do not edit",
    ] {
        assert!(
            ts.contains(expected),
            "bindings are missing {expected:?}:\n{ts}"
        );
    }
}
