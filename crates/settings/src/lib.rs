//! Layered settings and keymap presets.
//!
//! Contents: the IntelliJ-style key bindings, and EditorConfig resolution
//! ([`resolve_editorconfig`]). User/project settings files come later
//! (`docs/ROADMAP.md`).

mod editorconfig;

pub use editorconfig::{EditorConfig, EndOfLine, IndentStyle, resolve as resolve_editorconfig};

/// Editor actions that have key bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Save the current document.
    Save,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Clone the caret one line up.
    AddCaretAbove,
    /// Clone the caret one line down.
    AddCaretBelow,
    /// Add the next occurrence of the selection.
    SelectNextOccurrence,
    /// Select all occurrences of the selection.
    SelectAllOccurrences,
    /// Close the current tab.
    CloseTab,
    /// Search Everywhere.
    SearchEverywhere,
    /// Go to File.
    GotoFile,
    /// Go to Symbol.
    GotoSymbol,
    /// Find Action.
    GotoAction,
    /// Go to Line.
    GotoLine,
    /// Recent Files.
    RecentFiles,
    /// Find in Files.
    FindInFiles,
    /// Replace in Files.
    ReplaceInFiles,
    /// Open Folder (no default key).
    OpenFolder,
}

/// Modifier used in double-tap gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// Ctrl.
    Control,
    /// Alt / Option.
    Alt,
    /// Shift.
    Shift,
}

/// How a binding is triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStroke {
    /// A chord in CodeMirror notation (`Mod` = Cmd on macOS, Ctrl elsewhere).
    Chord(&'static str),
    /// Double-tap a modifier, keep it held, then press a key.
    DoubleModifier(Modifier, &'static str),
    /// Double-tap a modifier (double Shift).
    DoubleTap(Modifier),
}

/// A key binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    /// The action.
    pub action: Action,
    /// The key stroke.
    pub stroke: KeyStroke,
}

/// Which keymap variant to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapOs {
    /// macOS variant.
    Mac,
    /// Windows/Linux variant.
    Other,
}

/// Display name of an action (Search Everywhere's Actions tab).
pub fn action_name(action: Action) -> &'static str {
    match action {
        Action::Save => "Save",
        Action::Undo => "Undo",
        Action::Redo => "Redo",
        Action::AddCaretAbove => "Clone Caret Above",
        Action::AddCaretBelow => "Clone Caret Below",
        Action::SelectNextOccurrence => "Add Selection for Next Occurrence",
        Action::SelectAllOccurrences => "Select All Occurrences",
        Action::CloseTab => "Close Tab",
        Action::SearchEverywhere => "Search Everywhere",
        Action::GotoFile => "Go to File",
        Action::GotoSymbol => "Go to Symbol",
        Action::GotoAction => "Find Action",
        Action::GotoLine => "Go to Line",
        Action::RecentFiles => "Recent Files",
        Action::FindInFiles => "Find in Files",
        Action::ReplaceInFiles => "Replace in Files",
        Action::OpenFolder => "Open Folder",
    }
}

/// Every action, in display order.
pub const ALL_ACTIONS: [Action; 17] = [
    Action::GotoFile,
    Action::GotoSymbol,
    Action::GotoLine,
    Action::RecentFiles,
    Action::FindInFiles,
    Action::ReplaceInFiles,
    Action::SearchEverywhere,
    Action::GotoAction,
    Action::OpenFolder,
    Action::Save,
    Action::Undo,
    Action::Redo,
    Action::CloseTab,
    Action::AddCaretAbove,
    Action::AddCaretBelow,
    Action::SelectNextOccurrence,
    Action::SelectAllOccurrences,
];

/// IntelliJ-style default bindings (from the publicly documented default
/// keymap) for the actions implemented so far.
pub fn intellij_keymap(os: KeymapOs) -> Vec<KeyBinding> {
    use Action as A;
    use KeyStroke::{Chord, DoubleModifier, DoubleTap};
    let mac = os == KeymapOs::Mac;
    let caret_modifier = if mac {
        Modifier::Alt
    } else {
        Modifier::Control
    };
    let bind = |action, stroke| KeyBinding { action, stroke };
    vec![
        bind(A::Save, Chord("Mod-s")),
        bind(A::Undo, Chord("Mod-z")),
        bind(A::Redo, Chord("Mod-Shift-z")),
        bind(A::AddCaretAbove, DoubleModifier(caret_modifier, "ArrowUp")),
        bind(
            A::AddCaretBelow,
            DoubleModifier(caret_modifier, "ArrowDown"),
        ),
        bind(
            A::SelectNextOccurrence,
            Chord(if mac { "Ctrl-g" } else { "Alt-j" }),
        ),
        bind(
            A::SelectAllOccurrences,
            Chord(if mac {
                "Mod-Ctrl-g"
            } else {
                "Ctrl-Alt-Shift-j"
            }),
        ),
        bind(A::CloseTab, Chord(if mac { "Mod-w" } else { "Ctrl-F4" })),
        bind(A::SearchEverywhere, DoubleTap(Modifier::Shift)),
        bind(
            A::GotoFile,
            Chord(if mac { "Mod-Shift-o" } else { "Mod-Shift-n" }),
        ),
        bind(
            A::GotoSymbol,
            Chord(if mac { "Mod-Alt-o" } else { "Mod-Alt-Shift-n" }),
        ),
        bind(A::GotoAction, Chord("Mod-Shift-a")),
        bind(A::GotoLine, Chord(if mac { "Mod-l" } else { "Ctrl-g" })),
        bind(A::RecentFiles, Chord("Mod-e")),
        bind(A::FindInFiles, Chord("Mod-Shift-f")),
        bind(A::ReplaceInFiles, Chord("Mod-Shift-r")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_bound_once_per_os() {
        for os in [KeymapOs::Mac, KeymapOs::Other] {
            let keymap = intellij_keymap(os);
            for action in [
                Action::Save,
                Action::Undo,
                Action::Redo,
                Action::AddCaretAbove,
                Action::AddCaretBelow,
                Action::SelectNextOccurrence,
                Action::SelectAllOccurrences,
                Action::CloseTab,
                Action::SearchEverywhere,
                Action::GotoFile,
                Action::GotoSymbol,
                Action::GotoAction,
                Action::GotoLine,
                Action::RecentFiles,
                Action::FindInFiles,
                Action::ReplaceInFiles,
            ] {
                assert_eq!(
                    keymap.iter().filter(|b| b.action == action).count(),
                    1,
                    "{action:?}"
                );
            }
        }
    }

    #[test]
    fn mac_uses_mac_variants() {
        let mac = intellij_keymap(KeymapOs::Mac);
        assert!(mac.contains(&KeyBinding {
            action: Action::SelectNextOccurrence,
            stroke: KeyStroke::Chord("Ctrl-g")
        }));
        assert!(mac.contains(&KeyBinding {
            action: Action::AddCaretAbove,
            stroke: KeyStroke::DoubleModifier(Modifier::Alt, "ArrowUp")
        }));
    }
}
