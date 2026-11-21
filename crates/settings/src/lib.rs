//! Layered settings and keymap presets.
//!
//! Contents: the IntelliJ-style key bindings, EditorConfig resolution
//! ([`resolve_editorconfig`]) and the user settings file
//! ([`load_user_settings`]). Project settings files come later
//! (`docs/ROADMAP.md`).

mod editorconfig;
mod user;

pub use editorconfig::{EditorConfig, EndOfLine, IndentStyle, resolve as resolve_editorconfig};
pub use user::{UserSettings, load_user_settings};

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
    /// Open the current file in a split to the right (no default key).
    SplitRight,
    /// Fold the region at the caret.
    FoldCollapse,
    /// Unfold the region at the caret.
    FoldExpand,
    /// Fold everything.
    FoldCollapseAll,
    /// Unfold everything.
    FoldExpandAll,
    /// Move the caret to the matching bracket.
    GotoMatchingBrace,
    /// Quick Documentation for the symbol at the caret.
    QuickDocumentation,
    /// Go to the declaration of the symbol at the caret.
    GotoDeclaration,
    /// Go to the declaration of the type of the symbol at the caret.
    GotoTypeDeclaration,
    /// Go to implementations of the symbol at the caret.
    GotoImplementation,
    /// Show or hide the Problems tool window.
    ToggleProblems,
    /// Restart the TypeScript language server (no default key).
    RestartTypeScriptServer,
    /// Show code completion.
    Completion,
    /// Show parameter information.
    ParameterInfo,
    /// Find usages of the symbol at the caret.
    FindUsages,
    /// Rename the symbol at the caret.
    Rename,
    /// Show quick fixes and refactorings.
    ShowIntentions,
    /// Optimize imports.
    OptimizeImports,
    /// Sort imports.
    SortImports,
    /// Reformat the current document.
    ReformatCode,
    /// Show or hide the Terminal tool window.
    ToggleTerminal,
    /// Show or hide the Commit tool window.
    ToggleCommit,
    /// Show or hide the Git log tool window.
    ToggleGitLog,
    /// Branches popup (no default key; also the status bar branch).
    GitBranches,
    /// Diff of the current file against HEAD (no default key; Ctrl/Cmd+D
    /// in the commit window).
    ShowDiff,
    /// Show or hide blame in the gutter (no default key).
    ToggleBlame,
    /// Roll back the change at the caret (to the index).
    RollbackHunk,
    /// Run the selected run configuration.
    Run,
    /// Run the current file, or the test at the caret.
    RunContext,
    /// Stop the active run.
    StopRun,
    /// Run the active run's configuration again.
    Rerun,
    /// The run configurations popup.
    ChooseRunConfiguration,
    /// Show or hide the Run tool window.
    ToggleRunWindow,
    /// Debug the selected run configuration (stops on the first line).
    Debug,
    /// Resume a paused program.
    Resume,
    /// Step over the current line.
    StepOver,
    /// Step into the call on the current line.
    StepInto,
    /// Step out of the current function.
    StepOut,
    /// Pause a running program.
    Pause,
    /// Toggle a breakpoint on the caret's line.
    ToggleBreakpoint,
    /// Show or hide the Debug tool window.
    ToggleDebugWindow,
    /// Forget that the project is trusted: the next run asks again.
    RevokeProjectTrust,
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
        Action::SplitRight => "Split Right",
        Action::FoldCollapse => "Collapse",
        Action::FoldExpand => "Expand",
        Action::FoldCollapseAll => "Collapse All",
        Action::FoldExpandAll => "Expand All",
        Action::GotoMatchingBrace => "Move Caret to Matching Brace",
        Action::QuickDocumentation => "Quick Documentation",
        Action::GotoDeclaration => "Go to Declaration",
        Action::GotoTypeDeclaration => "Go to Type Declaration",
        Action::GotoImplementation => "Go to Implementation",
        Action::ToggleProblems => "Problems",
        Action::RestartTypeScriptServer => "Restart TypeScript Server",
        Action::Completion => "Code Completion",
        Action::ParameterInfo => "Parameter Info",
        Action::FindUsages => "Find Usages",
        Action::Rename => "Rename",
        Action::ShowIntentions => "Show Intentions",
        Action::OptimizeImports => "Optimize Imports",
        Action::SortImports => "Sort Imports",
        Action::ReformatCode => "Reformat Code",
        Action::ToggleTerminal => "Terminal",
        Action::ToggleCommit => "Commit",
        Action::ToggleGitLog => "Git Log",
        Action::GitBranches => "Branches…",
        Action::ShowDiff => "Show Diff",
        Action::ToggleBlame => "Annotate with Git Blame",
        Action::RollbackHunk => "Rollback Lines",
        Action::Run => "Run",
        Action::RunContext => "Run Current File",
        Action::StopRun => "Stop",
        Action::Rerun => "Rerun",
        Action::ChooseRunConfiguration => "Run…",
        Action::ToggleRunWindow => "Run Tool Window",
        Action::Debug => "Debug",
        Action::Resume => "Resume Program",
        Action::StepOver => "Step Over",
        Action::StepInto => "Step Into",
        Action::StepOut => "Step Out",
        Action::Pause => "Pause Program",
        Action::ToggleBreakpoint => "Toggle Line Breakpoint",
        Action::ToggleDebugWindow => "Debug Tool Window",
        Action::RevokeProjectTrust => "Revoke Project Trust",
    }
}

/// Every action, in display order.
pub const ALL_ACTIONS: [Action; 59] = [
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
    Action::SplitRight,
    Action::FoldCollapse,
    Action::FoldExpand,
    Action::FoldCollapseAll,
    Action::FoldExpandAll,
    Action::GotoMatchingBrace,
    Action::QuickDocumentation,
    Action::GotoDeclaration,
    Action::GotoTypeDeclaration,
    Action::GotoImplementation,
    Action::ToggleProblems,
    Action::RestartTypeScriptServer,
    Action::Completion,
    Action::ParameterInfo,
    Action::FindUsages,
    Action::Rename,
    Action::ShowIntentions,
    Action::OptimizeImports,
    Action::SortImports,
    Action::ReformatCode,
    Action::ToggleTerminal,
    Action::ToggleCommit,
    Action::ToggleGitLog,
    Action::GitBranches,
    Action::ShowDiff,
    Action::ToggleBlame,
    Action::RollbackHunk,
    Action::Run,
    Action::RunContext,
    Action::StopRun,
    Action::Rerun,
    Action::ChooseRunConfiguration,
    Action::ToggleRunWindow,
    Action::Debug,
    Action::Resume,
    Action::StepOver,
    Action::StepInto,
    Action::StepOut,
    Action::Pause,
    Action::ToggleBreakpoint,
    Action::ToggleDebugWindow,
    Action::RevokeProjectTrust,
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
        bind(A::FoldCollapse, Chord("Mod--")),
        bind(A::FoldExpand, Chord("Mod-=")),
        bind(A::FoldCollapseAll, Chord("Mod-Shift--")),
        bind(A::FoldExpandAll, Chord("Mod-Shift-=")),
        bind(
            A::GotoMatchingBrace,
            Chord(if mac { "Mod-Ctrl-m" } else { "Mod-Shift-m" }),
        ),
        bind(
            A::QuickDocumentation,
            Chord(if mac { "F1" } else { "Ctrl-q" }),
        ),
        bind(A::GotoDeclaration, Chord("Mod-b")),
        bind(A::GotoTypeDeclaration, Chord("Mod-Shift-b")),
        bind(A::GotoImplementation, Chord("Mod-Alt-b")),
        bind(A::Completion, Chord("Ctrl-Space")),
        bind(A::ParameterInfo, Chord("Mod-p")),
        bind(A::FindUsages, Chord("Alt-F7")),
        bind(A::Rename, Chord("Shift-F6")),
        bind(A::ShowIntentions, Chord("Alt-Enter")),
        bind(A::OptimizeImports, Chord("Ctrl-Alt-o")),
        bind(A::SortImports, Chord("Ctrl-Alt-Shift-o")),
        bind(A::ReformatCode, Chord("Mod-Alt-l")),
        bind(
            A::ToggleProblems,
            Chord(if mac { "Mod-6" } else { "Alt-6" }),
        ),
        bind(A::ToggleTerminal, Chord("Alt-F12")),
        bind(A::ToggleCommit, Chord(if mac { "Mod-0" } else { "Alt-0" })),
        bind(A::ToggleGitLog, Chord(if mac { "Mod-9" } else { "Alt-9" })),
        bind(A::RollbackHunk, Chord("Mod-Alt-z")),
        bind(A::Run, Chord(if mac { "Ctrl-r" } else { "Shift-F10" })),
        bind(
            A::RunContext,
            Chord(if mac {
                "Ctrl-Shift-r"
            } else {
                "Ctrl-Shift-F10"
            }),
        ),
        bind(A::StopRun, Chord("Mod-F2")),
        bind(A::Rerun, Chord(if mac { "Mod-r" } else { "Ctrl-F5" })),
        bind(
            A::ChooseRunConfiguration,
            Chord(if mac { "Ctrl-Alt-r" } else { "Alt-Shift-F10" }),
        ),
        bind(
            A::ToggleRunWindow,
            Chord(if mac { "Mod-4" } else { "Alt-4" }),
        ),
        bind(A::Debug, Chord(if mac { "Ctrl-d" } else { "Shift-F9" })),
        bind(A::Resume, Chord(if mac { "Mod-Alt-r" } else { "F9" })),
        bind(A::StepOver, Chord("F8")),
        bind(A::StepInto, Chord("F7")),
        bind(A::StepOut, Chord("Shift-F8")),
        // IntelliJ's Pause Program has no default key; this one is free.
        bind(A::Pause, Chord("Mod-Alt-p")),
        bind(A::ToggleBreakpoint, Chord("Mod-F8")),
        bind(
            A::ToggleDebugWindow,
            Chord(if mac { "Mod-5" } else { "Alt-5" }),
        ),
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
                Action::Completion,
                Action::ParameterInfo,
                Action::FindUsages,
                Action::Rename,
                Action::ShowIntentions,
                Action::OptimizeImports,
                Action::SortImports,
                Action::ReformatCode,
                Action::ToggleTerminal,
                Action::ToggleCommit,
                Action::ToggleGitLog,
                Action::RollbackHunk,
                Action::Run,
                Action::RunContext,
                Action::StopRun,
                Action::Rerun,
                Action::ChooseRunConfiguration,
                Action::ToggleRunWindow,
                Action::Debug,
                Action::Resume,
                Action::StepOver,
                Action::StepInto,
                Action::StepOut,
                Action::Pause,
                Action::ToggleBreakpoint,
                Action::ToggleDebugWindow,
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
    fn no_two_actions_share_a_stroke() {
        for os in [KeymapOs::Mac, KeymapOs::Other] {
            let keymap = intellij_keymap(os);
            for (i, a) in keymap.iter().enumerate() {
                for b in &keymap[i + 1..] {
                    assert_ne!(
                        a.stroke, b.stroke,
                        "{:?} and {:?} share a key",
                        a.action, b.action
                    );
                }
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
