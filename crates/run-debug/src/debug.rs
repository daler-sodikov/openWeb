//! The protocol-neutral debug session model (`docs/ARCHITECTURE.md`, 4.10.2).
//!
//! Pure data and pure functions: no process, no async, no protocol. A CDP
//! backend fills it today and a DAP backend could later. Lines and columns
//! are zero-based; paths are project-relative with `/` when inside the
//! project, else absolute. Node's own modules have a script name and no path.

use serde::{Deserialize, Serialize};

/// Longest variable preview, in characters (the rest is cut).
pub const MAX_PREVIEW_CHARS: usize = 200;

/// Cuts a value preview to [`MAX_PREVIEW_CHARS`] characters, ending in `…`.
pub fn cut_preview(text: &str) -> String {
    match text.char_indices().nth(MAX_PREVIEW_CHARS) {
        None => text.to_owned(),
        Some((end, _)) => format!("{}…", &text[..end]),
    }
}

/// What the debugger made of a breakpoint. Session data, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BreakpointState {
    /// Not set in a debugger: no session, the script is not loaded yet, or
    /// there is no code at that line.
    Unverified {
        /// Why, for the tooltip.
        reason: String,
    },
    /// Set; `line` is where the debugger put it, possibly later than asked.
    Verified {
        /// The resolved line.
        line: u32,
    },
}

impl BreakpointState {
    /// The state of a breakpoint nobody has tried to set.
    pub fn no_session() -> Self {
        Self::Unverified {
            reason: "No debug session".to_owned(),
        }
    }
}

/// Where the program is in its life. A command is allowed only in the states
/// where it makes sense (see [`DebugState::check`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DebugState {
    /// The process is starting; the debugger is not attached yet.
    Starting,
    /// The program runs.
    Running,
    /// The program is stopped.
    Paused(PauseInfo),
    /// The session is over.
    #[serde(rename_all = "camelCase")]
    Ended {
        /// The program's exit code, when it ended by itself and had one.
        exit_code: Option<i32>,
        /// True when the user stopped it.
        stopped: bool,
    },
}

/// Why the program stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PauseReason {
    /// A breakpoint was hit.
    Breakpoint,
    /// A step finished.
    Step,
    /// The user asked to pause.
    Pause,
    /// An exception was thrown.
    Exception {
        /// The exception's text.
        message: String,
    },
    /// The stop at the first line of a program started with `--inspect-brk`.
    Entry,
}

/// A position in a file or script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based column.
    pub column: u32,
}

/// Where a frame is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// The file (project-relative or absolute), or a script name such as
    /// `node:fs` when there is no file.
    pub path_or_script: String,
    /// Zero-based line, in the original file when a source map applied.
    pub line: u32,
    /// Zero-based column.
    pub column: u32,
    /// The position in the generated code when it was mapped back, for
    /// diagnostics.
    pub generated: Option<Position>,
}

/// The kind of a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopeKind {
    /// Function locals.
    Local,
    /// Variables closed over.
    Closure,
    /// A block.
    Block,
    /// A module's top level.
    Module,
    /// The global object.
    Global,
}

/// A scope of a frame: its name and a handle; the contents are fetched lazily.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    /// The kind.
    pub kind: ScopeKind,
    /// A display name, if the debugger has one.
    pub name: String,
    /// The handle for [`DebugCommand::Variables`].
    pub variables_ref: String,
}

/// One entry of the call stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// An opaque id from the backend.
    pub id: String,
    /// The function name (empty for anonymous).
    pub name: String,
    /// Where it is.
    pub location: Location,
    /// Under `node_modules` or a `node:` module; the view dims it.
    pub is_library: bool,
    /// Names and handles only.
    pub scopes: Vec<Scope>,
}

/// A stop: why, the stack, and which frame is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseInfo {
    /// Why the program stopped.
    pub reason: PauseReason,
    /// The call stack, innermost first.
    pub frames: Vec<Frame>,
    /// Index into `frames`; 0 at a new stop.
    pub selected_frame: usize,
}

impl PauseInfo {
    /// A stop at the innermost frame.
    pub fn new(reason: PauseReason, frames: Vec<Frame>) -> Self {
        Self {
            reason,
            frames,
            selected_frame: 0,
        }
    }

    /// The selected frame, if the index is valid.
    pub fn selected(&self) -> Option<&Frame> {
        self.frames.get(self.selected_frame)
    }
}

/// A variable, shown as a short preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    /// The name or index.
    pub name: String,
    /// A preview made by the backend (`"abc"`, `Array(3)`, `{a: 1, …}`),
    /// cut with [`cut_preview`].
    pub value: String,
    /// The type, for display.
    pub type_name: String,
    /// Present when the value can be expanded.
    pub variables_ref: Option<String>,
    /// An expression that reads it again, for "add to watches".
    pub evaluate_name: Option<String>,
}

/// A page of children: the backend sends at most [`PAGE_SIZE`] at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesPage {
    /// The children in the asked range.
    pub variables: Vec<Variable>,
    /// How many children there are in all, so the view can say "N more".
    pub total: usize,
}

/// The most children fetched in one request.
pub const PAGE_SIZE: usize = 100;

/// A command to a debug session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugCommand {
    /// Continue.
    Resume,
    /// Step over.
    StepOver,
    /// Step into.
    StepInto,
    /// Step out.
    StepOut,
    /// Stop a running program.
    Pause,
    /// End the session.
    Stop,
    /// Read children of a scope or variable.
    Variables,
    /// Evaluate an expression in a frame.
    Evaluate,
}

/// What a session tells its listeners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DebugEvent {
    /// The debugger is attached; the program runs.
    Started,
    /// The program stopped.
    Paused(PauseInfo),
    /// The program runs again (also after a step).
    Resumed,
    /// A breakpoint was set, moved or refused.
    #[serde(rename_all = "camelCase")]
    BreakpointState {
        /// Ferrule's id of the breakpoint.
        id: u64,
        /// The new state.
        state: BreakpointState,
    },
    /// A line from the program's console.
    Output {
        /// The text.
        text: String,
    },
    /// The session is over.
    #[serde(rename_all = "camelCase")]
    Ended {
        /// The exit code, if the program ended by itself.
        exit_code: Option<i32>,
        /// True when the user stopped it.
        stopped: bool,
    },
}

/// A refused command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DebugError {
    /// The command does not make sense in the current state.
    #[error("{command:?} is not possible while the session is {state}")]
    NotAllowed {
        /// The refused command.
        command: DebugCommand,
        /// A word for the state: starting, running, paused or ended.
        state: &'static str,
    },
    /// The backend could not do it.
    #[error("{0}")]
    Backend(String),
}

impl DebugState {
    /// A word for the state, for messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Paused(_) => "paused",
            Self::Ended { .. } => "ended",
        }
    }

    /// Whether `command` is allowed now: Resume, the steps, Variables and
    /// Evaluate only while paused; Pause only while running; Stop until the
    /// session has ended.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] otherwise; a command is never ignored.
    pub fn check(&self, command: DebugCommand) -> Result<(), DebugError> {
        use DebugCommand::{Evaluate, Pause, Resume, StepInto, StepOut, StepOver, Stop, Variables};
        let allowed = match command {
            Resume | StepOver | StepInto | StepOut | Variables | Evaluate => {
                matches!(self, Self::Paused(_))
            }
            Pause => matches!(self, Self::Running),
            Stop => !matches!(self, Self::Ended { .. }),
        };
        if allowed {
            Ok(())
        } else {
            Err(DebugError::NotAllowed {
                command,
                state: self.name(),
            })
        }
    }

    /// The state after `event`. Events that do not change the state
    /// (breakpoint states, output) and events that cannot happen in this
    /// state (a stop after the end) leave it as it is.
    #[must_use]
    pub fn after(&self, event: &DebugEvent) -> DebugState {
        if matches!(self, Self::Ended { .. }) {
            return self.clone();
        }
        match event {
            DebugEvent::Started | DebugEvent::Resumed => Self::Running,
            DebugEvent::Paused(info) => Self::Paused(info.clone()),
            DebugEvent::Ended { exit_code, stopped } => Self::Ended {
                exit_code: *exit_code,
                stopped: *stopped,
            },
            DebugEvent::BreakpointState { .. } | DebugEvent::Output { .. } => self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [DebugCommand; 8] = [
        DebugCommand::Resume,
        DebugCommand::StepOver,
        DebugCommand::StepInto,
        DebugCommand::StepOut,
        DebugCommand::Pause,
        DebugCommand::Stop,
        DebugCommand::Variables,
        DebugCommand::Evaluate,
    ];

    fn frame(name: &str) -> Frame {
        Frame {
            id: format!("f-{name}"),
            name: name.to_owned(),
            location: Location {
                path_or_script: "src/a.ts".to_owned(),
                line: 3,
                column: 0,
                generated: None,
            },
            is_library: false,
            scopes: Vec::new(),
        }
    }

    fn paused() -> DebugState {
        DebugState::Paused(PauseInfo::new(PauseReason::Step, vec![frame("a")]))
    }

    fn allowed(state: &DebugState) -> Vec<DebugCommand> {
        ALL.into_iter()
            .filter(|c| state.check(*c).is_ok())
            .collect()
    }

    #[test]
    fn commands_are_allowed_only_where_they_make_sense() {
        use DebugCommand::{Evaluate, Pause, Resume, StepInto, StepOut, StepOver, Stop, Variables};
        assert_eq!(allowed(&DebugState::Starting), vec![Stop]);
        assert_eq!(allowed(&DebugState::Running), vec![Pause, Stop]);
        assert_eq!(
            allowed(&paused()),
            vec![
                Resume, StepOver, StepInto, StepOut, Stop, Variables, Evaluate
            ]
        );
        let ended = DebugState::Ended {
            exit_code: Some(0),
            stopped: false,
        };
        assert!(allowed(&ended).is_empty());
    }

    #[test]
    fn a_refused_command_says_which_and_why() {
        let error = DebugState::Running.check(DebugCommand::Resume).unwrap_err();
        assert_eq!(
            error,
            DebugError::NotAllowed {
                command: DebugCommand::Resume,
                state: "running"
            }
        );
        assert!(error.to_string().contains("Resume"));
        assert!(error.to_string().contains("running"));
    }

    #[test]
    fn events_move_the_state() {
        let stop = PauseInfo::new(PauseReason::Entry, vec![frame("main")]);
        let mut state = DebugState::Starting;
        state = state.after(&DebugEvent::Started);
        assert_eq!(state, DebugState::Running);
        state = state.after(&DebugEvent::Paused(stop.clone()));
        assert_eq!(state, DebugState::Paused(stop));
        state = state.after(&DebugEvent::Output {
            text: "x".to_owned(),
        });
        assert_eq!(state.name(), "paused");
        state = state.after(&DebugEvent::Resumed);
        assert_eq!(state, DebugState::Running);
        state = state.after(&DebugEvent::Ended {
            exit_code: Some(1),
            stopped: false,
        });
        let ended = DebugState::Ended {
            exit_code: Some(1),
            stopped: false,
        };
        assert_eq!(state, ended);
        // Nothing revives an ended session.
        assert_eq!(state.after(&DebugEvent::Resumed), ended);
    }

    #[test]
    fn the_selected_frame_is_the_first_unless_changed() {
        let mut info = PauseInfo::new(PauseReason::Pause, vec![frame("a"), frame("b")]);
        assert_eq!(info.selected().map(|f| f.name.as_str()), Some("a"));
        info.selected_frame = 1;
        assert_eq!(info.selected().map(|f| f.name.as_str()), Some("b"));
        info.selected_frame = 5;
        assert!(info.selected().is_none());
    }

    #[test]
    fn previews_are_cut_on_a_character_boundary() {
        assert_eq!(cut_preview("short"), "short");
        let exact = "é".repeat(MAX_PREVIEW_CHARS);
        assert_eq!(cut_preview(&exact), exact);
        let long = "é".repeat(MAX_PREVIEW_CHARS + 50);
        let cut = cut_preview(&long);
        assert_eq!(cut.chars().count(), MAX_PREVIEW_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn events_round_trip_through_json() {
        let event = DebugEvent::Paused(PauseInfo::new(
            PauseReason::Exception {
                message: "boom".to_owned(),
            },
            vec![frame("a")],
        ));
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DebugEvent>(&json).unwrap(), event);
    }
}
