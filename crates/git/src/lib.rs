//! Git integration (`docs/ARCHITECTURE.md`, 4.9): runs the user's `git`
//! executable with machine-readable output and parses it. Line diffs (for
//! gutter markers, rollback and the diff viewer) are computed here.
//!
//! Nothing here uses the network or credentials: only local subcommands
//! are run, with prompts disabled.

mod blame;
mod branches;
mod diff;
mod error;
mod log;
mod repo;
mod status;

pub use blame::{BlameLine, parse_blame};
pub use branches::{BRANCH_FORMAT, Branch, parse_branches};
pub use diff::{
    DiffRow, Hunk, HunkKind, LineKind, Ranges, SplitRow, StagedHunk, diff_lines, hunk_base_text,
    line_count, locate_staged, rollback_hunk, split_rows, staged_diff, staged_hunks, unified_rows,
    word_ranges,
};
pub use error::{FailureKind, GitError};
pub use log::{Commit, LOG_FORMAT, parse_log};
pub use repo::{Git, Repo};
pub use status::{BranchInfo, Change, FileStatus, RepoStatus, StatusKind, parse_status};
