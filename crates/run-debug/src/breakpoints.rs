//! The breakpoint store (`docs/ARCHITECTURE.md`, 4.10.2 and 4.10.5).
//!
//! Pure: it keeps the project's breakpoints sorted, follows edits, and turns
//! itself into and out of the text of the stored form. `ferrule-core` does
//! the file IO (Ferrule's data folder, next to trust).

use serde::{Deserialize, Serialize};

use crate::debug::BreakpointState;

/// Version of the stored form.
const FORMAT_VERSION: u32 = 1;

/// A breakpoint. The state is session data and is not stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Breakpoint {
    /// Ferrule's id, stable across sessions.
    pub id: u64,
    /// Project-relative with `/` when inside the project, else absolute.
    pub path: String,
    /// Zero-based line.
    pub line: u32,
    /// Pause only when this expression is truthy.
    pub condition: Option<String>,
    /// Print this message instead of pausing (`text {expr} text`, see
    /// [`crate::logpoint_condition`]).
    pub log_message: Option<String>,
    /// A disabled breakpoint is kept but not set.
    pub enabled: bool,
    /// What the debugger made of it.
    #[serde(skip, default = "BreakpointState::no_session")]
    pub state: BreakpointState,
}

/// The editable parts of a breakpoint. Blank texts mean "none".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BreakpointEdit {
    /// The condition.
    pub condition: Option<String>,
    /// The log message.
    pub log_message: Option<String>,
    /// Enabled or not.
    pub enabled: bool,
}

/// What [`BreakpointStore::toggle`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toggled {
    /// A breakpoint was added; this is its id.
    Added(u64),
    /// The breakpoint on that line was removed.
    Removed(Breakpoint),
}

/// What [`BreakpointStore::shift_for_edit`] changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shifted {
    /// Ids of breakpoints that moved to another line (now unverified; a
    /// running session sets them again).
    pub moved: Vec<u64>,
    /// Breakpoints whose line was deleted.
    pub removed: Vec<Breakpoint>,
}

/// Why a breakpoint operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BreakpointError {
    /// No breakpoint has this id.
    #[error("no breakpoint with id {0}")]
    NotFound(u64),
    /// The stored text is not a breakpoint file.
    #[error("the breakpoints file is not valid: {0}")]
    Invalid(String),
    /// The file was written by a newer Ferrule.
    #[error("the breakpoints file has version {0}, this Ferrule reads version {FORMAT_VERSION}")]
    UnsupportedVersion(u32),
}

/// The stored form: a version and the breakpoints sorted by path then line.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: u32,
    breakpoints: Vec<Breakpoint>,
}

/// The project's breakpoints, sorted by path then line, one per line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BreakpointStore {
    items: Vec<Breakpoint>,
    next_id: u64,
}

fn blank_to_none(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

impl BreakpointStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// All breakpoints, sorted by path then line.
    pub fn all(&self) -> &[Breakpoint] {
        &self.items
    }

    /// The breakpoints of one file, by line.
    pub fn for_path<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a Breakpoint> {
        self.items.iter().filter(move |b| b.path == path)
    }

    /// The breakpoint with this id.
    pub fn get(&self, id: u64) -> Option<&Breakpoint> {
        self.items.iter().find(|b| b.id == id)
    }

    /// Mutable access to the state of a breakpoint (session data only).
    pub fn set_state(&mut self, id: u64, state: BreakpointState) -> Result<(), BreakpointError> {
        let item = self
            .items
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(BreakpointError::NotFound(id))?;
        item.state = state;
        Ok(())
    }

    fn sort(&mut self) {
        self.items
            .sort_by(|a, b| (&a.path, a.line, a.id).cmp(&(&b.path, b.line, b.id)));
    }

    /// Adds a breakpoint on `path:line`, or removes the one that is there.
    pub fn toggle(&mut self, path: &str, line: u32) -> Toggled {
        if let Some(at) = self
            .items
            .iter()
            .position(|b| b.path == path && b.line == line)
        {
            return Toggled::Removed(self.items.remove(at));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Breakpoint {
            id,
            path: path.to_owned(),
            line,
            condition: None,
            log_message: None,
            enabled: true,
            state: BreakpointState::no_session(),
        });
        self.sort();
        Toggled::Added(id)
    }

    /// Replaces the condition, log message and enabled flag.
    ///
    /// # Errors
    /// [`BreakpointError::NotFound`] for an unknown id.
    pub fn update(
        &mut self,
        id: u64,
        edit: BreakpointEdit,
    ) -> Result<&Breakpoint, BreakpointError> {
        let item = self
            .items
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(BreakpointError::NotFound(id))?;
        item.condition = blank_to_none(edit.condition);
        item.log_message = blank_to_none(edit.log_message);
        item.enabled = edit.enabled;
        Ok(item)
    }

    /// Removes one breakpoint.
    pub fn remove(&mut self, id: u64) -> Option<Breakpoint> {
        let at = self.items.iter().position(|b| b.id == id)?;
        Some(self.items.remove(at))
    }

    /// Removes all breakpoints ("Remove all breakpoints"); this is also what
    /// drops those of files that no longer exist.
    pub fn clear(&mut self) -> Vec<Breakpoint> {
        std::mem::take(&mut self.items)
    }

    /// Follows an edit of a document. The edit replaces `removed_lines`
    /// whole lines from `start_line` with `inserted_lines` lines (`core`
    /// widens a text edit to the lines it touches), so:
    ///
    /// - lines before `start_line` stay;
    /// - of the replaced lines, the first `min(removed, inserted)` keep
    ///   their breakpoints (typing in a line, Enter in a line) and the rest
    ///   are deleted (deleting a line, joining two lines);
    /// - lines after the edit move by `inserted - removed` (insert above,
    ///   delete above); a pure insertion (`removed_lines == 0`) moves the
    ///   breakpoints from `start_line` down.
    pub fn shift_for_edit(
        &mut self,
        path: &str,
        start_line: u32,
        removed_lines: u32,
        inserted_lines: u32,
    ) -> Shifted {
        let mut shifted = Shifted::default();
        let kept = removed_lines.min(inserted_lines);
        let replaced_end = start_line.saturating_add(removed_lines);
        let items = std::mem::take(&mut self.items);
        for mut bp in items {
            if bp.path != path || bp.line < start_line {
                self.items.push(bp);
            } else if bp.line < replaced_end {
                if bp.line - start_line < kept {
                    self.items.push(bp);
                } else {
                    shifted.removed.push(bp);
                }
            } else {
                let line =
                    i64::from(bp.line) + i64::from(inserted_lines) - i64::from(removed_lines);
                if line != i64::from(bp.line) {
                    // The moved line cannot be negative: bp.line >= replaced_end.
                    bp.line = u32::try_from(line).unwrap_or(u32::MAX);
                    bp.state = BreakpointState::Unverified {
                        reason: "The line moved".to_owned(),
                    };
                    shifted.moved.push(bp.id);
                }
                self.items.push(bp);
            }
        }
        self.sort();
        shifted
    }

    /// The stored form as text.
    ///
    /// # Errors
    /// [`BreakpointError::Invalid`] if serialization fails (it does not for
    /// this data; the error exists so no panic is needed).
    pub fn to_json(&self) -> Result<String, BreakpointError> {
        let stored = Stored {
            version: FORMAT_VERSION,
            breakpoints: self.items.clone(),
        };
        serde_json::to_string_pretty(&stored).map_err(|e| BreakpointError::Invalid(e.to_string()))
    }

    /// Reads the stored form. Breakpoints are sorted again; a second one on
    /// the same line is dropped. Ids stay as stored and new ids start after
    /// the largest.
    ///
    /// # Errors
    /// [`BreakpointError::Invalid`] for text that is not the file,
    /// [`BreakpointError::UnsupportedVersion`] for a newer version.
    pub fn from_json(text: &str) -> Result<Self, BreakpointError> {
        let stored: Stored =
            serde_json::from_str(text).map_err(|e| BreakpointError::Invalid(e.to_string()))?;
        if stored.version != FORMAT_VERSION {
            return Err(BreakpointError::UnsupportedVersion(stored.version));
        }
        let mut store = Self::new();
        store.items = stored.breakpoints;
        store.sort();
        store
            .items
            .dedup_by(|later, first| later.path == first.path && later.line == first.line);
        store.next_id = store.items.iter().map(|b| b.id + 1).max().unwrap_or(0);
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(lines: &[(&str, u32)]) -> BreakpointStore {
        let mut s = BreakpointStore::new();
        for (path, line) in lines {
            s.toggle(path, *line);
        }
        s
    }

    fn lines(s: &BreakpointStore, path: &str) -> Vec<u32> {
        s.for_path(path).map(|b| b.line).collect()
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut s = BreakpointStore::new();
        let Toggled::Added(id) = s.toggle("a.ts", 4) else {
            panic!("expected an added breakpoint");
        };
        assert_eq!(s.get(id).map(|b| b.line), Some(4));
        assert!(s.get(id).is_some_and(|b| b.enabled));
        let Toggled::Removed(gone) = s.toggle("a.ts", 4) else {
            panic!("expected a removed breakpoint");
        };
        assert_eq!(gone.id, id);
        assert!(s.all().is_empty());
        // A new breakpoint never reuses the id.
        let Toggled::Added(next) = s.toggle("a.ts", 4) else {
            panic!("expected an added breakpoint");
        };
        assert_ne!(next, id);
    }

    #[test]
    fn breakpoints_are_sorted_by_path_then_line() {
        let s = store(&[("b.ts", 1), ("a.ts", 9), ("a.ts", 2), ("b.ts", 0)]);
        let order: Vec<_> = s.all().iter().map(|b| (b.path.as_str(), b.line)).collect();
        assert_eq!(order, [("a.ts", 2), ("a.ts", 9), ("b.ts", 0), ("b.ts", 1)]);
        assert_eq!(lines(&s, "a.ts"), [2, 9]);
        assert!(lines(&s, "c.ts").is_empty());
    }

    #[test]
    fn update_sets_fields_and_treats_blank_as_none() {
        let mut s = store(&[("a.ts", 1)]);
        let id = s.all()[0].id;
        let bp = s
            .update(
                id,
                BreakpointEdit {
                    condition: Some("i > 3".to_owned()),
                    log_message: Some("   ".to_owned()),
                    enabled: false,
                },
            )
            .unwrap();
        assert_eq!(bp.condition.as_deref(), Some("i > 3"));
        assert_eq!(bp.log_message, None);
        assert!(!bp.enabled);
        assert_eq!(
            s.update(99, BreakpointEdit::default()).unwrap_err(),
            BreakpointError::NotFound(99)
        );
    }

    #[test]
    fn remove_and_clear() {
        let mut s = store(&[("a.ts", 1), ("a.ts", 2)]);
        let id = s.all()[0].id;
        assert_eq!(s.remove(id).map(|b| b.line), Some(1));
        assert!(s.remove(id).is_none());
        assert_eq!(s.clear().len(), 1);
        assert!(s.all().is_empty());
    }

    #[test]
    fn the_stored_form_round_trips_without_session_state() {
        let mut s = store(&[("b.ts", 7), ("a.ts", 2)]);
        let id = s.all()[0].id;
        s.update(
            id,
            BreakpointEdit {
                condition: Some("x".to_owned()),
                log_message: Some("v={v}".to_owned()),
                enabled: false,
            },
        )
        .unwrap();
        s.set_state(id, BreakpointState::Verified { line: 3 })
            .unwrap();
        let text = s.to_json().unwrap();
        assert!(!text.contains("Verified"));
        assert!(!text.contains("verified"));
        let back = BreakpointStore::from_json(&text).unwrap();
        // Same data; the state of a loaded breakpoint is "no session".
        s.set_state(id, BreakpointState::no_session()).unwrap();
        assert_eq!(back, s);
        // New ids continue after the stored ones.
        let mut back = back;
        let Toggled::Added(next) = back.toggle("c.ts", 0) else {
            panic!("expected an added breakpoint");
        };
        assert!(back.all().iter().filter(|b| b.id == next).count() == 1);
        assert!(back.all().iter().all(|b| b.id <= next));
    }

    #[test]
    fn a_bad_or_newer_file_is_an_error_not_a_panic() {
        assert!(matches!(
            BreakpointStore::from_json("not json"),
            Err(BreakpointError::Invalid(_))
        ));
        assert!(matches!(
            BreakpointStore::from_json(r#"{"version":1}"#),
            Err(BreakpointError::Invalid(_))
        ));
        assert_eq!(
            BreakpointStore::from_json(r#"{"version":2,"breakpoints":[]}"#).unwrap_err(),
            BreakpointError::UnsupportedVersion(2)
        );
    }

    #[test]
    fn a_file_with_unsorted_or_duplicate_entries_is_tidied() {
        let text = r#"{"version":1,"breakpoints":[
            {"id":5,"path":"b.ts","line":1,"condition":null,"logMessage":null,"enabled":true},
            {"id":2,"path":"a.ts","line":3,"condition":null,"logMessage":null,"enabled":true},
            {"id":3,"path":"a.ts","line":3,"condition":null,"logMessage":null,"enabled":true}]}"#;
        let s = BreakpointStore::from_json(text).unwrap();
        let order: Vec<_> = s.all().iter().map(|b| (b.path.as_str(), b.line)).collect();
        assert_eq!(order, [("a.ts", 3), ("b.ts", 1)]);
    }

    // shift_for_edit: every case.

    #[test]
    fn insert_above_moves_the_breakpoint_down() {
        let mut s = store(&[("a.ts", 5), ("b.ts", 5)]);
        let out = s.shift_for_edit("a.ts", 2, 0, 3);
        assert_eq!(lines(&s, "a.ts"), [8]);
        assert_eq!(lines(&s, "b.ts"), [5], "other files are untouched");
        assert_eq!(out.moved.len(), 1);
        assert!(out.removed.is_empty());
        assert!(matches!(
            s.all()[0].state,
            BreakpointState::Unverified { .. }
        ));
    }

    #[test]
    fn inserting_at_the_breakpoint_line_pushes_it_down() {
        let mut s = store(&[("a.ts", 5)]);
        s.shift_for_edit("a.ts", 5, 0, 1);
        assert_eq!(lines(&s, "a.ts"), [6]);
    }

    #[test]
    fn insert_below_changes_nothing() {
        let mut s = store(&[("a.ts", 5)]);
        let out = s.shift_for_edit("a.ts", 6, 0, 4);
        assert_eq!(lines(&s, "a.ts"), [5]);
        assert_eq!(out, Shifted::default());
    }

    #[test]
    fn delete_above_moves_the_breakpoint_up() {
        let mut s = store(&[("a.ts", 9)]);
        let out = s.shift_for_edit("a.ts", 2, 3, 0);
        assert_eq!(lines(&s, "a.ts"), [6]);
        assert!(out.removed.is_empty());
    }

    #[test]
    fn deleting_the_line_removes_the_breakpoint() {
        let mut s = store(&[("a.ts", 4), ("a.ts", 5), ("a.ts", 6)]);
        let out = s.shift_for_edit("a.ts", 5, 1, 0);
        assert_eq!(lines(&s, "a.ts"), [4, 5]);
        assert_eq!(out.removed.len(), 1);
        assert_eq!(out.removed[0].line, 5);
        assert_eq!(out.moved.len(), 1, "the one below moved up");
    }

    #[test]
    fn a_multi_line_edit_across_it_removes_what_has_no_replacement_line() {
        // Lines 2..6 (four lines) are replaced by two lines.
        let mut s = store(&[
            ("a.ts", 1),
            ("a.ts", 2),
            ("a.ts", 3),
            ("a.ts", 5),
            ("a.ts", 6),
        ]);
        let out = s.shift_for_edit("a.ts", 2, 4, 2);
        // 1 stays; 2 and 3 have replacement lines; 5 is deleted; 6 moves up by two.
        assert_eq!(lines(&s, "a.ts"), [1, 2, 3, 4]);
        assert_eq!(out.removed.len(), 1);
        assert_eq!(out.removed[0].line, 5);
        assert_eq!(out.moved.len(), 1);
    }

    #[test]
    fn typing_in_the_line_and_enter_in_the_line_keep_the_breakpoint() {
        let mut s = store(&[("a.ts", 3)]);
        let out = s.shift_for_edit("a.ts", 3, 1, 1);
        assert_eq!(lines(&s, "a.ts"), [3]);
        assert_eq!(out, Shifted::default());
        // Enter in the middle of the line: one line becomes two.
        s.shift_for_edit("a.ts", 3, 1, 2);
        assert_eq!(lines(&s, "a.ts"), [3]);
    }

    #[test]
    fn joining_two_lines_keeps_the_first_breakpoint_only() {
        let mut s = store(&[("a.ts", 3), ("a.ts", 4), ("a.ts", 5)]);
        s.shift_for_edit("a.ts", 3, 2, 1);
        assert_eq!(lines(&s, "a.ts"), [3, 4]);
    }

    #[test]
    fn an_edit_before_the_start_of_a_file_does_not_underflow() {
        let mut s = store(&[("a.ts", 0), ("a.ts", 1)]);
        s.shift_for_edit("a.ts", 0, 2, 0);
        assert!(s.all().is_empty());
        let mut s = store(&[("a.ts", u32::MAX)]);
        s.shift_for_edit("a.ts", 0, 0, 5);
        assert_eq!(lines(&s, "a.ts"), [u32::MAX]);
    }
}
