//! Layered settings and keymap presets.
//!
//! M1 scope: the IntelliJ-style key bindings for the editor actions that
//! exist so far. Settings files and EditorConfig arrive later
//! (`docs/ROADMAP.md`).

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
}

/// Modifier used in double-tap gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// Ctrl.
    Control,
    /// Alt / Option.
    Alt,
}

/// How a binding is triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStroke {
    /// A chord in CodeMirror notation (`Mod` = Cmd on macOS, Ctrl elsewhere).
    Chord(&'static str),
    /// Double-tap a modifier, keep it held, then press a key.
    DoubleModifier(Modifier, &'static str),
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

/// IntelliJ-style default bindings (from the publicly documented default
/// keymap) for the actions implemented so far.
pub fn intellij_keymap(os: KeymapOs) -> Vec<KeyBinding> {
    use Action as A;
    use KeyStroke::{Chord, DoubleModifier};
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
