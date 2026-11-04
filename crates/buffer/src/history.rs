//! Undo/redo history with typing groups.

use crate::document::{ByteEdit, Edit, EditKind};
use crate::selection::Selection;

/// Transactions closer together than this are grouped (if their kind and
/// selections line up).
const GROUP_DELAY_MS: u64 = 500;

/// Maximum number of undo groups kept.
const MAX_GROUPS: usize = 1000;

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub forward: Vec<Edit>,
    pub inverse: Vec<Edit>,
    pub selection_before: Selection,
    pub selection_after: Selection,
    pub kind: EditKind,
    pub time_ms: u64,
}

type Group = Vec<Entry>;

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
}

impl History {
    pub fn record(&mut self, entry: Entry) {
        self.redo.clear();
        if let Some(group) = self.undo.last_mut()
            && let Some(prev) = group.last()
            && can_merge(prev, &entry)
        {
            group.push(entry);
            return;
        }
        self.undo.push(vec![entry]);
        if self.undo.len() > MAX_GROUPS {
            self.undo.remove(0);
        }
    }

    pub fn pop_undo(&mut self) -> Option<Group> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Group> {
        self.redo.pop()
    }

    pub fn push_redo(&mut self, group: Group) {
        self.redo.push(group);
    }

    pub fn push_undo(&mut self, group: Group) {
        self.undo.push(group);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

fn can_merge(prev: &Entry, next: &Entry) -> bool {
    prev.kind == next.kind
        && prev.kind != EditKind::Other
        && next.time_ms.saturating_sub(prev.time_ms) < GROUP_DELAY_MS
        && prev.selection_after == next.selection_before
}

/// Result of an undo or redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoOutcome {
    /// Version after the operation.
    pub version: u32,
    /// The edits applied, one list per version step, each relative to the
    /// text before that step.
    pub updates: Vec<Vec<Edit>>,
    /// Selection to show after the operation.
    pub selection: Selection,
    /// Byte edits for incremental parsing, in application order.
    pub byte_edits: Vec<ByteEdit>,
}
