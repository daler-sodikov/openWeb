//! `git log` with separator bytes, parsed.

/// The `--format` that [`parse_log`] reads: fields separated by 0x1f,
/// records ended by 0x1e.
pub const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%P%x1f%s%x1e";

/// A commit in a history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Full id.
    pub id: String,
    /// Abbreviated id.
    pub short_id: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author time, seconds since the epoch.
    pub time: i64,
    /// Parent ids.
    pub parents: Vec<String>,
    /// First line of the message.
    pub subject: String,
}

/// Parses output produced with [`LOG_FORMAT`].
pub fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split('\x1e')
        .map(|r| r.trim_start_matches(['\n', '\r']))
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let f: Vec<&str> = record.split('\x1f').collect();
            let [id, short_id, author, email, time, parents, subject] = f.as_slice() else {
                return None;
            };
            Some(Commit {
                id: (*id).to_owned(),
                short_id: (*short_id).to_owned(),
                author: (*author).to_owned(),
                email: (*email).to_owned(),
                time: time.parse().unwrap_or(0),
                parents: parents.split_whitespace().map(str::to_owned).collect(),
                subject: (*subject).to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records() {
        let out = "abc\x1fab\x1fAda\x1fada@x\x1f17\x1fp1 p2\x1fMerge it\x1e\n\
                   def\x1fde\x1fBob\x1fbob@x\x1f16\x1f\x1fFirst\x1e\n";
        let log = parse_log(out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].parents, ["p1", "p2"]);
        assert_eq!(log[0].subject, "Merge it");
        assert_eq!(log[1].time, 16);
        assert!(log[1].parents.is_empty());
    }
}
