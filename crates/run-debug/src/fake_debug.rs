//! A scripted debug backend for UI and core tests without Node (feature
//! `testing`; never in the app). It is synchronous and has no process: a
//! script lists what the "program" does, commands move it along, and every
//! call returns the events it caused, in order.
//!
//! The program is `Starting` until [`FakeDebug::start`]. It then `Running`
//! and plays its script on [`FakeDebug::advance`]: output lines are emitted
//! until the next stop (which pauses it) or the exit (which ends it).
//! Resume and the steps put it back to `Running`; the test calls `advance`
//! to let it continue, so every state in between can be observed.

use std::collections::{BTreeMap, VecDeque};

use crate::breakpoints::Breakpoint;
use crate::debug::Variable;
use crate::debug::{
    BreakpointState, DebugCommand, DebugError, DebugEvent, DebugState, PauseInfo, PauseReason,
    VariablesPage,
};

/// One thing the scripted program does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptItem {
    /// Prints a line.
    Output(String),
    /// Stops (a breakpoint, a step end, an exception ...).
    Stop(PauseInfo),
    /// Ends by itself with this exit code.
    Exit(Option<i32>),
}

/// The scripted backend.
#[derive(Debug)]
pub struct FakeDebug {
    state: DebugState,
    script: VecDeque<ScriptItem>,
    /// `(path, asked line)` -> the line the "debugger" resolves it to.
    resolved: BTreeMap<(String, u32), u32>,
    /// `variables_ref` -> children.
    children: BTreeMap<String, Vec<Variable>>,
}

impl FakeDebug {
    /// A backend that will play `script`. A script that ends without an
    /// `Exit` ends the program with exit code 0.
    pub fn new(script: Vec<ScriptItem>) -> Self {
        Self {
            state: DebugState::Starting,
            script: script.into(),
            resolved: BTreeMap::new(),
            children: BTreeMap::new(),
        }
    }

    /// Makes `path:asked` resolve to `resolved` (a line with no code moves
    /// down, as in a real debugger).
    #[must_use]
    pub fn resolving(mut self, path: &str, asked: u32, resolved: u32) -> Self {
        self.resolved.insert((path.to_owned(), asked), resolved);
        self
    }

    /// Serves `children` for `variables_ref`.
    #[must_use]
    pub fn with_children(mut self, variables_ref: &str, children: Vec<Variable>) -> Self {
        self.children.insert(variables_ref.to_owned(), children);
        self
    }

    /// The current state.
    pub fn state(&self) -> &DebugState {
        &self.state
    }

    fn emit(&mut self, event: DebugEvent, out: &mut Vec<DebugEvent>) {
        self.state = self.state.after(&event);
        out.push(event);
    }

    /// Attaches: `Starting` to `Running`.
    ///
    /// # Errors
    /// If the session already started.
    pub fn start(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        if !matches!(self.state, DebugState::Starting) {
            return Err(DebugError::Backend(format!(
                "the session is already {}",
                self.state.name()
            )));
        }
        let mut out = Vec::new();
        self.emit(DebugEvent::Started, &mut out);
        Ok(out)
    }

    /// Lets a running program play its script up to the next stop or the end.
    /// Does nothing (no events) when the program is not running.
    pub fn advance(&mut self) -> Vec<DebugEvent> {
        let mut out = Vec::new();
        if !matches!(self.state, DebugState::Running) {
            return out;
        }
        loop {
            match self.script.pop_front() {
                Some(ScriptItem::Output(text)) => {
                    self.emit(DebugEvent::Output { text }, &mut out);
                }
                Some(ScriptItem::Stop(info)) => {
                    self.emit(DebugEvent::Paused(info), &mut out);
                    break;
                }
                Some(ScriptItem::Exit(exit_code)) => {
                    self.emit(
                        DebugEvent::Ended {
                            exit_code,
                            stopped: false,
                        },
                        &mut out,
                    );
                    break;
                }
                None => {
                    self.emit(
                        DebugEvent::Ended {
                            exit_code: Some(0),
                            stopped: false,
                        },
                        &mut out,
                    );
                    break;
                }
            }
        }
        out
    }

    fn resume_as(&mut self, command: DebugCommand) -> Result<Vec<DebugEvent>, DebugError> {
        self.state.check(command)?;
        let mut out = Vec::new();
        self.emit(DebugEvent::Resumed, &mut out);
        Ok(out)
    }

    /// Continues a paused program.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless paused.
    pub fn resume(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.resume_as(DebugCommand::Resume)
    }

    /// Steps over; the next scripted stop is the end of the step.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless paused.
    pub fn step_over(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.resume_as(DebugCommand::StepOver)
    }

    /// Steps into.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless paused.
    pub fn step_into(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.resume_as(DebugCommand::StepInto)
    }

    /// Steps out.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless paused.
    pub fn step_out(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.resume_as(DebugCommand::StepOut)
    }

    /// Stops a running program at its next scripted stop, reported as a
    /// user pause.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless running; [`DebugError::Backend`]
    /// if the script has no stop left to show.
    pub fn pause(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.state.check(DebugCommand::Pause)?;
        let at = self
            .script
            .iter()
            .position(|item| matches!(item, ScriptItem::Stop(_)))
            .ok_or_else(|| DebugError::Backend("the script has no stop left".to_owned()))?;
        let mut out = Vec::new();
        for item in self.script.drain(..at) {
            if let ScriptItem::Output(text) = item {
                self.state = self.state.after(&DebugEvent::Output { text: text.clone() });
                out.push(DebugEvent::Output { text });
            }
        }
        if let Some(ScriptItem::Stop(mut info)) = self.script.pop_front() {
            info.reason = PauseReason::Pause;
            self.emit(DebugEvent::Paused(info), &mut out);
        }
        Ok(out)
    }

    /// Ends the session at the user's request, from any state but the end.
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] once ended.
    pub fn stop(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        self.state.check(DebugCommand::Stop)?;
        self.script.clear();
        let mut out = Vec::new();
        self.emit(
            DebugEvent::Ended {
                exit_code: None,
                stopped: true,
            },
            &mut out,
        );
        Ok(out)
    }

    /// Sets the breakpoints of one file, and answers the state of each:
    /// disabled ones are unverified; the others are verified on the line
    /// given by [`FakeDebug::resolving`], else on the line asked.
    pub fn set_breakpoints(&self, path: &str, breakpoints: &[Breakpoint]) -> Vec<BreakpointState> {
        let running = !matches!(self.state, DebugState::Starting | DebugState::Ended { .. });
        breakpoints
            .iter()
            .map(|b| {
                if !running {
                    BreakpointState::no_session()
                } else if !b.enabled {
                    BreakpointState::Unverified {
                        reason: "Disabled".to_owned(),
                    }
                } else {
                    let line = self
                        .resolved
                        .get(&(path.to_owned(), b.line))
                        .copied()
                        .unwrap_or(b.line);
                    BreakpointState::Verified { line }
                }
            })
            .collect()
    }

    /// Children of a scope or variable, `count` from `start`, at most
    /// [`crate::debug::PAGE_SIZE`].
    ///
    /// # Errors
    /// [`DebugError::NotAllowed`] unless paused; [`DebugError::Backend`] for
    /// an unknown reference.
    pub fn variables(
        &self,
        variables_ref: &str,
        start: usize,
        count: usize,
    ) -> Result<VariablesPage, DebugError> {
        self.state.check(DebugCommand::Variables)?;
        let all = self
            .children
            .get(variables_ref)
            .ok_or_else(|| DebugError::Backend(format!("unknown reference {variables_ref}")))?;
        let count = count.min(crate::debug::PAGE_SIZE);
        Ok(VariablesPage {
            variables: all.iter().skip(start).take(count).cloned().collect(),
            total: all.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breakpoints::BreakpointStore;
    use crate::debug::{Frame, Location, PAGE_SIZE};

    fn stop(reason: PauseReason, line: u32) -> PauseInfo {
        PauseInfo::new(
            reason,
            vec![Frame {
                id: format!("f{line}"),
                name: "main".to_owned(),
                location: Location {
                    path_or_script: "src/app.ts".to_owned(),
                    line,
                    column: 0,
                    generated: None,
                },
                is_library: false,
                scopes: Vec::new(),
            }],
        )
    }

    fn session() -> FakeDebug {
        FakeDebug::new(vec![
            ScriptItem::Stop(stop(PauseReason::Entry, 0)),
            ScriptItem::Output("hello".to_owned()),
            ScriptItem::Stop(stop(PauseReason::Breakpoint, 4)),
            ScriptItem::Stop(stop(PauseReason::Step, 5)),
            ScriptItem::Output("bye".to_owned()),
            ScriptItem::Exit(Some(3)),
        ])
    }

    fn kinds(events: &[DebugEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                DebugEvent::Started => "started",
                DebugEvent::Paused(_) => "paused",
                DebugEvent::Resumed => "resumed",
                DebugEvent::BreakpointState { .. } => "breakpoint",
                DebugEvent::Output { .. } => "output",
                DebugEvent::Ended { .. } => "ended",
            })
            .collect()
    }

    #[test]
    fn plays_a_session_start_pause_step_end() {
        let mut fake = session();
        assert_eq!(fake.state().name(), "starting");
        assert_eq!(kinds(&fake.start().unwrap()), ["started"]);
        assert_eq!(fake.state().name(), "running");

        let events = fake.advance();
        assert_eq!(kinds(&events), ["paused"]);
        let DebugState::Paused(entry) = fake.state() else {
            panic!("expected paused");
        };
        assert_eq!(entry.reason, PauseReason::Entry);

        assert_eq!(kinds(&fake.resume().unwrap()), ["resumed"]);
        let events = fake.advance();
        assert_eq!(kinds(&events), ["output", "paused"]);
        assert_eq!(
            events[0],
            DebugEvent::Output {
                text: "hello".to_owned()
            }
        );

        assert_eq!(kinds(&fake.step_over().unwrap()), ["resumed"]);
        let events = fake.advance();
        let DebugEvent::Paused(step) = &events[0] else {
            panic!("expected a stop");
        };
        assert_eq!(step.reason, PauseReason::Step);
        assert_eq!(step.frames[0].location.line, 5);

        fake.step_out().unwrap();
        let events = fake.advance();
        assert_eq!(kinds(&events), ["output", "ended"]);
        assert_eq!(
            fake.state(),
            &DebugState::Ended {
                exit_code: Some(3),
                stopped: false
            }
        );
    }

    #[test]
    fn a_script_without_exit_ends_with_code_zero() {
        let mut fake = FakeDebug::new(vec![]);
        fake.start().unwrap();
        fake.advance();
        assert_eq!(
            fake.state(),
            &DebugState::Ended {
                exit_code: Some(0),
                stopped: false
            }
        );
    }

    #[test]
    fn commands_in_the_wrong_state_are_refused() {
        let mut fake = session();
        assert!(matches!(
            fake.resume(),
            Err(DebugError::NotAllowed {
                state: "starting",
                ..
            })
        ));
        fake.start().unwrap();
        assert!(matches!(
            fake.step_into(),
            Err(DebugError::NotAllowed {
                state: "running",
                ..
            })
        ));
        assert!(fake.start().is_err(), "a session starts once");
        fake.advance();
        assert!(matches!(
            fake.pause(),
            Err(DebugError::NotAllowed {
                state: "paused",
                ..
            })
        ));
    }

    #[test]
    fn pause_stops_a_running_program_as_a_user_pause() {
        let mut fake = session();
        fake.start().unwrap();
        fake.advance(); // entry stop
        fake.resume().unwrap();
        let events = fake.pause().unwrap();
        assert_eq!(kinds(&events), ["output", "paused"]);
        let DebugState::Paused(info) = fake.state() else {
            panic!("expected paused");
        };
        assert_eq!(info.reason, PauseReason::Pause);

        let mut empty = FakeDebug::new(vec![ScriptItem::Exit(None)]);
        empty.start().unwrap();
        assert!(matches!(empty.pause(), Err(DebugError::Backend(_))));
    }

    #[test]
    fn stop_ends_from_running_and_from_paused_and_only_once() {
        let mut fake = session();
        fake.start().unwrap();
        fake.advance();
        let events = fake.stop().unwrap();
        assert_eq!(
            events,
            [DebugEvent::Ended {
                exit_code: None,
                stopped: true
            }]
        );
        assert!(fake.stop().is_err());
        assert!(fake.advance().is_empty(), "nothing plays after the end");

        let mut starting = session();
        assert!(starting.stop().is_ok(), "also before it attached");
    }

    #[test]
    fn breakpoints_verify_on_the_resolved_line() {
        let mut store = BreakpointStore::new();
        store.toggle("src/app.ts", 3);
        store.toggle("src/app.ts", 8);
        let id = store.all()[1].id;
        store
            .update(
                id,
                crate::breakpoints::BreakpointEdit {
                    enabled: false,
                    ..Default::default()
                },
            )
            .unwrap();
        let mut fake = session().resolving("src/app.ts", 3, 4);
        let items = store.all().to_vec();
        // Before the session starts nothing is set.
        assert!(
            fake.set_breakpoints("src/app.ts", &items)
                .iter()
                .all(|s| matches!(s, BreakpointState::Unverified { .. }))
        );
        fake.start().unwrap();
        let states = fake.set_breakpoints("src/app.ts", &items);
        assert_eq!(states[0], BreakpointState::Verified { line: 4 });
        assert!(matches!(states[1], BreakpointState::Unverified { .. }));
    }

    fn var(name: &str) -> Variable {
        Variable {
            name: name.to_owned(),
            value: "1".to_owned(),
            type_name: "number".to_owned(),
            variables_ref: None,
            evaluate_name: Some(name.to_owned()),
        }
    }

    #[test]
    fn variables_come_in_pages_and_only_while_paused() {
        let many: Vec<Variable> = (0..250).map(|i| var(&i.to_string())).collect();
        let mut fake = session().with_children("local", many);
        fake.start().unwrap();
        assert!(matches!(
            fake.variables("local", 0, 10),
            Err(DebugError::NotAllowed { .. })
        ));
        fake.advance();
        let page = fake.variables("local", 0, 1000).unwrap();
        assert_eq!(page.variables.len(), PAGE_SIZE);
        assert_eq!(page.total, 250);
        let tail = fake.variables("local", 200, 100).unwrap();
        assert_eq!(tail.variables.len(), 50);
        assert_eq!(tail.variables[0].name, "200");
        assert!(matches!(
            fake.variables("nope", 0, 1),
            Err(DebugError::Backend(_))
        ));
    }
}
