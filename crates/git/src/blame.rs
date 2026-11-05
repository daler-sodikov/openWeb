//! `git blame --porcelain`, parsed.

use std::collections::HashMap;

use crate::error::GitError;

/// Who last changed a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    /// Full commit id (all zeros for lines not committed yet).
    pub commit: String,
    /// Author name.
    pub author: String,
    /// Author email (without `<>`).
    pub email: String,
    /// Author time, seconds since the epoch.
    pub time: i64,
    /// First line of the commit message.
    pub summary: String,
    /// Whether the line is committed (false for local changes).
    pub committed: bool,
}

#[derive(Default, Clone)]
struct Info {
    author: String,
    email: String,
    time: i64,
    summary: String,
}

fn is_header(line: &str) -> Option<(&str, usize)> {
    let mut parts = line.split(' ');
    let sha = parts.next()?;
    if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let _orig: usize = parts.next()?.parse().ok()?;
    let final_line: usize = parts.next()?.parse().ok()?;
    Some((sha, final_line))
}

/// One entry per line of the blamed text, in order.
pub fn parse_blame(output: &str) -> Result<Vec<BlameLine>, GitError> {
    let mut infos: HashMap<String, Info> = HashMap::new();
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if line.starts_with('\t') {
            if current.take().is_none() {
                return Err(GitError::Parse("blame content before a header".into()));
            }
            continue;
        }
        if let Some((sha, final_line)) = is_header(line) {
            current = Some(sha.to_owned());
            infos.entry(sha.to_owned()).or_default();
            lines.push((final_line, sha.to_owned()));
            continue;
        }
        let Some(sha) = current.as_ref() else {
            continue;
        };
        let info = infos.entry(sha.clone()).or_default();
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => info.author = value.to_owned(),
            "author-mail" => {
                info.email = value
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_owned();
            }
            "author-time" => info.time = value.parse().unwrap_or(0),
            "summary" => info.summary = value.to_owned(),
            _ => {}
        }
    }
    lines.sort_by_key(|(n, _)| *n);
    Ok(lines
        .into_iter()
        .map(|(_, sha)| {
            let info = infos.get(&sha).cloned().unwrap_or_default();
            BlameLine {
                committed: !sha.bytes().all(|b| b == b'0'),
                commit: sha,
                author: info.author,
                email: info.email,
                time: info.time,
                summary: info.summary,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_commits_and_local_lines() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Ada\nauthor-mail <ada@example.com>\nauthor-time 1700000000\n\
             author-tz +0000\nsummary First commit\nfilename f.txt\n\tone\n\
             {a} 2 2\nfilename f.txt\n\ttwo\n\
             {z} 3 3 1\nauthor Not Committed Yet\nauthor-mail <not.committed.yet>\n\
             author-time 1700000100\nsummary Version of f.txt from f.txt\nfilename f.txt\n\tthree\n"
        );
        let lines = parse_blame(&out).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].author, "Ada");
        assert_eq!(lines[1].email, "ada@example.com");
        assert_eq!(lines[1].summary, "First commit");
        assert_eq!(lines[1].time, 1_700_000_000);
        assert!(lines[1].committed);
        assert!(!lines[2].committed);
    }
}
