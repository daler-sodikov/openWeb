//! `git for-each-ref` over local and remote branches, parsed.

/// The `--format` that [`parse_branches`] reads (fields separated by 0x1f).
pub const BRANCH_FORMAT: &str = "--format=%(HEAD)%1f%(refname)%1f%(refname:short)%1f%(upstream:short)%1f%(objectname:short)%1f%(subject)";

/// A branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// Short name (`main`, `origin/main`).
    pub name: String,
    /// Remote-tracking branch.
    pub remote: bool,
    /// Checked out.
    pub current: bool,
    /// Upstream of a local branch.
    pub upstream: Option<String>,
    /// Abbreviated id of its commit.
    pub commit: String,
    /// Subject of its commit.
    pub subject: String,
}

/// Parses output produced with [`BRANCH_FORMAT`]; skips `origin/HEAD`.
pub fn parse_branches(output: &str) -> Vec<Branch> {
    output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\x1f').collect();
            let [head, refname, short, upstream, commit, subject] = f.as_slice() else {
                return None;
            };
            let remote = refname.starts_with("refs/remotes/");
            if remote && refname.ends_with("/HEAD") {
                return None;
            }
            Some(Branch {
                name: (*short).to_owned(),
                remote,
                current: *head == "*",
                upstream: (!upstream.is_empty()).then(|| (*upstream).to_owned()),
                commit: (*commit).to_owned(),
                subject: (*subject).to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_and_remote_branches() {
        let out = "*\x1frefs/heads/main\x1fmain\x1forigin/main\x1fabc\x1fInit\n\
                   \x20\x1frefs/heads/dev\x1fdev\x1f\x1fdef\x1fWork\n\
                   \x20\x1frefs/remotes/origin/HEAD\x1forigin\x1f\x1fabc\x1fInit\n\
                   \x20\x1frefs/remotes/origin/main\x1forigin/main\x1f\x1fabc\x1fInit\n";
        let b = parse_branches(out);
        assert_eq!(b.len(), 3);
        assert!(b[0].current);
        assert_eq!(b[0].upstream.as_deref(), Some("origin/main"));
        assert!(!b[1].current && !b[1].remote);
        assert!(b[2].remote);
        assert_eq!(b[2].name, "origin/main");
    }
}
