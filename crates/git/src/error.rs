//! Errors, with the failures the UI explains in plain words.

/// What kind of failure a `git` command reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// `user.name` or `user.email` is not set.
    MissingIdentity,
    /// A commit hook rejected the commit.
    HookFailed,
    /// Nothing is staged (or the amend changes nothing).
    NothingToCommit,
    /// A checkout would overwrite local changes.
    LocalChanges,
    /// `branch -d` of a branch that is not merged.
    NotMerged,
    /// A branch with that name exists.
    BranchExists,
    /// Anything else.
    Other,
}

/// Errors running `git`.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// `git` could not be started.
    #[error("cannot run git ({program}): {message}")]
    NotInstalled {
        /// The program tried.
        program: String,
        /// Why.
        message: String,
    },
    /// The command failed.
    #[error("{message}")]
    Failed {
        /// Recognized cause.
        kind: FailureKind,
        /// What git said (stderr, else stdout), trimmed.
        message: String,
    },
    /// Output that could not be parsed.
    #[error("unexpected git output: {0}")]
    Parse(String),
}

/// Classifies a failed command by its output (git runs with `LC_ALL=C`).
pub(crate) fn classify(output: &str) -> FailureKind {
    let has = |needle: &str| output.contains(needle);
    if has("Please tell me who you are")
        || has("Author identity unknown")
        || has("empty ident name")
        || has("unable to auto-detect email address")
    {
        FailureKind::MissingIdentity
    } else if has("nothing to commit")
        || has("no changes added to commit")
        || has("nothing added to commit")
    {
        FailureKind::NothingToCommit
    } else if has("would be overwritten by checkout")
        || has("would be overwritten by switch")
        || has("Your local changes to the following files would be overwritten")
    {
        FailureKind::LocalChanges
    } else if has("is not fully merged") {
        FailureKind::NotMerged
    } else if has("already exists") {
        FailureKind::BranchExists
    } else {
        FailureKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_failures() {
        assert_eq!(
            classify("*** Please tell me who you are.\n\nRun\n  git config --global user.email"),
            FailureKind::MissingIdentity
        );
        assert_eq!(
            classify("On branch main\nnothing to commit, working tree clean"),
            FailureKind::NothingToCommit
        );
        assert_eq!(
            classify(
                "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt"
            ),
            FailureKind::LocalChanges
        );
        assert_eq!(
            classify("error: the branch 'x' is not fully merged."),
            FailureKind::NotMerged
        );
        assert_eq!(
            classify("fatal: a branch named 'x' already exists"),
            FailureKind::BranchExists
        );
        assert_eq!(classify("fatal: bad revision"), FailureKind::Other);
    }
}
