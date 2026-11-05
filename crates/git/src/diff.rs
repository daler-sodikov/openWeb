//! Line and word diffs (Myers), gutter hunks, hunk rollback and the rows
//! the diff viewer draws.

use std::collections::HashMap;

/// Most differences the diff searches for; past that the rest of the
/// changed region becomes one hunk (keeps huge rewrites fast).
const MAX_EDITS: usize = 2000;

/// A run of the diff script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `n` equal items.
    Equal(usize),
    /// `n` items only in the old sequence.
    Delete(usize),
    /// `n` items only in the new sequence.
    Insert(usize),
}

fn push(ops: &mut Vec<Op>, op: Op) {
    match (ops.last_mut(), op) {
        (Some(Op::Equal(a)), Op::Equal(b))
        | (Some(Op::Delete(a)), Op::Delete(b))
        | (Some(Op::Insert(a)), Op::Insert(b)) => *a += b,
        _ if matches!(op, Op::Equal(0) | Op::Delete(0) | Op::Insert(0)) => {}
        _ => ops.push(op),
    }
}

/// The edit script from `a` to `b`.
fn diff_ids(a: &[u32], b: &[u32]) -> Vec<Op> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ma, mb) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut ops = Vec::new();
    push(&mut ops, Op::Equal(prefix));
    for op in myers(ma, mb) {
        push(&mut ops, op);
    }
    push(&mut ops, Op::Equal(suffix));
    ops
}

fn idx(k: isize, d: usize) -> usize {
    // k ranges over -d..=d; stored at k + d.
    usize::try_from(k + isize::try_from(d).unwrap_or(isize::MAX)).unwrap_or(0)
}

/// Myers' O((N+M)·D) algorithm, keeping only the `[-d, d]` part of each
/// round for the backtrack (O(D²) memory).
fn myers(a: &[u32], b: &[u32]) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return vec![Op::Delete(n), Op::Insert(m)];
    }
    let (ni, mi) = (
        isize::try_from(n).unwrap_or(isize::MAX),
        isize::try_from(m).unwrap_or(isize::MAX),
    );
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut prev: Vec<isize> = vec![0; 1];
    let mut found = None;
    for d in 0..=MAX_EDITS.min(n + m) {
        let mut v = vec![0isize; 2 * d + 1];
        let di = isize::try_from(d).unwrap_or(isize::MAX);
        for k in (-di..=di).step_by(2) {
            let down = k == -di || (k != di && prev[idx(k - 1, d - 1)] < prev[idx(k + 1, d - 1)]);
            let mut x = if d == 0 {
                0
            } else if down {
                prev[idx(k + 1, d - 1)]
            } else {
                prev[idx(k - 1, d - 1)] + 1
            };
            let mut y = x - k;
            while x < ni && y < mi && a[x.unsigned_abs()] == b[y.unsigned_abs()] {
                x += 1;
                y += 1;
            }
            v[idx(k, d)] = x;
            if x >= ni && y >= mi {
                found = Some(d);
                break;
            }
        }
        trace.push(v.clone());
        prev = v;
        if found.is_some() {
            break;
        }
    }
    let Some(dmax) = found else {
        // Too different: one replacement.
        return vec![Op::Delete(n), Op::Insert(m)];
    };
    // Backtrack from (n, m).
    let mut rev = Vec::new();
    let (mut x, mut y) = (ni, mi);
    for d in (1..=dmax).rev() {
        let di = isize::try_from(d).unwrap_or(isize::MAX);
        let vprev = &trace[d - 1];
        let k = x - y;
        let down = k == -di || (k != di && vprev[idx(k - 1, d - 1)] < vprev[idx(k + 1, d - 1)]);
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = vprev[idx(prev_k, d - 1)];
        let prev_y = prev_x - prev_k;
        if down {
            // Came from above: an insertion, then the diagonal.
            rev.push(Op::Equal((x - prev_x).unsigned_abs()));
            rev.push(Op::Insert(1));
        } else {
            rev.push(Op::Equal((y - prev_y).unsigned_abs()));
            rev.push(Op::Delete(1));
        }
        x = prev_x;
        y = prev_y;
    }
    rev.push(Op::Equal(x.unsigned_abs()));
    let mut ops = Vec::new();
    for op in rev.into_iter().rev() {
        push(&mut ops, op);
    }
    ops
}

/// Interns items to ids so comparisons are integer compares.
fn intern<'a>(a: &[&'a str], b: &[&'a str]) -> (Vec<u32>, Vec<u32>) {
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let mut id = |s: &'a str| {
        let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        *ids.entry(s).or_insert(next)
    };
    let a = a.iter().map(|s| id(s)).collect();
    let b = b.iter().map(|s| id(s)).collect();
    (a, b)
}

/// Lines without terminators (`\r\n` and `\n`).
fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// What a hunk does to the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    /// Lines only in the new text.
    Added,
    /// Lines changed.
    Modified,
    /// Lines only in the base.
    Deleted,
}

/// A changed region. Lines are zero-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// First base line.
    pub old_start: usize,
    /// Number of base lines.
    pub old_lines: usize,
    /// First new line (for a deletion: the line after it).
    pub new_start: usize,
    /// Number of new lines.
    pub new_lines: usize,
    /// Kind.
    pub kind: HunkKind,
}

/// Changed regions from `old` (the base) to `new`.
pub fn diff_lines(old: &str, new: &str) -> Vec<Hunk> {
    let (a, b) = (lines(old), lines(new));
    let (ia, ib) = intern(&a, &b);
    let mut hunks = Vec::new();
    let (mut x, mut y) = (0, 0);
    let mut open: Option<Hunk> = None;
    for op in diff_ids(&ia, &ib) {
        match op {
            Op::Equal(n) => {
                if let Some(h) = open.take() {
                    hunks.push(h);
                }
                x += n;
                y += n;
            }
            Op::Delete(n) | Op::Insert(n) => {
                let h = open.get_or_insert(Hunk {
                    old_start: x,
                    old_lines: 0,
                    new_start: y,
                    new_lines: 0,
                    kind: HunkKind::Modified,
                });
                if matches!(op, Op::Delete(_)) {
                    h.old_lines += n;
                    x += n;
                } else {
                    h.new_lines += n;
                    y += n;
                }
            }
        }
    }
    if let Some(h) = open {
        hunks.push(h);
    }
    for h in &mut hunks {
        h.kind = match (h.old_lines, h.new_lines) {
            (0, _) => HunkKind::Added,
            (_, 0) => HunkKind::Deleted,
            _ => HunkKind::Modified,
        };
    }
    hunks
}

/// The base text of a hunk's lines (what rollback restores), `\n`-joined.
pub fn hunk_base_text(old: &str, hunk: &Hunk) -> String {
    lines(old)
        .get(hunk.old_start..hunk.old_start + hunk.old_lines)
        .map(|l| l.join("\n"))
        .unwrap_or_default()
}

/// The edit to `new` that puts back the base lines of `hunk`: a byte range
/// of `new` and its replacement. `None` if the hunk does not fit `new`.
pub fn rollback_hunk(old: &str, new: &str, hunk: &Hunk) -> Option<(usize, usize, String)> {
    let (a, mut b) = (lines(old), lines(new));
    let base = a.get(hunk.old_start..hunk.old_start + hunk.old_lines)?;
    if hunk.new_start + hunk.new_lines > b.len() {
        return None;
    }
    b.splice(
        hunk.new_start..hunk.new_start + hunk.new_lines,
        base.iter().copied(),
    );
    let mut result = b.join("\n");
    let trailing = new.ends_with('\n') || (new.is_empty() && old.ends_with('\n'));
    if trailing && !result.is_empty() {
        result.push('\n');
    }
    // The smallest replacement: strip the common prefix and suffix.
    let prefix = common_prefix(new, &result);
    let suffix = common_suffix(&new[prefix..], &result[prefix..]);
    Some((
        prefix,
        new.len() - suffix,
        result[prefix..result.len() - suffix].to_owned(),
    ))
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i)
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n += x.len_utf8();
    }
    n
}

/// Words, runs of spaces and single other characters.
fn tokens(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut prev: Option<u8> = None;
    for (i, c) in line.char_indices() {
        let k = class(c);
        if prev.is_some_and(|p| p != k || k == 2) {
            out.push(&line[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

fn utf16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// UTF-16 `[from, to)` ranges in a line.
pub type Ranges = Vec<(u32, u32)>;

/// Changed parts of a modified line pair, as UTF-16 ranges in each line.
pub fn word_ranges(old: &str, new: &str) -> (Ranges, Ranges) {
    let (ta, tb) = (tokens(old), tokens(new));
    let (ia, ib) = intern(&ta, &tb);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut x, mut y) = (0usize, 0usize);
    let (mut ox, mut oy) = (0u32, 0u32);
    let add = |ranges: &mut Vec<(u32, u32)>, from: u32, to: u32| match ranges.last_mut() {
        Some(last) if last.1 == from => last.1 = to,
        _ if from < to => ranges.push((from, to)),
        _ => {}
    };
    for op in diff_ids(&ia, &ib) {
        match op {
            Op::Equal(n) => {
                ox += ta[x..x + n].iter().map(|t| utf16_len(t)).sum::<u32>();
                oy += tb[y..y + n].iter().map(|t| utf16_len(t)).sum::<u32>();
                x += n;
                y += n;
            }
            Op::Delete(n) => {
                let len: u32 = ta[x..x + n].iter().map(|t| utf16_len(t)).sum();
                add(&mut left, ox, ox + len);
                ox += len;
                x += n;
            }
            Op::Insert(n) => {
                let len: u32 = tb[y..y + n].iter().map(|t| utf16_len(t)).sum();
                add(&mut right, oy, oy + len);
                oy += len;
                y += n;
            }
        }
    }
    (left, right)
}

/// How a diff row is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged.
    Context,
    /// Only in the new text.
    Added,
    /// Only in the base.
    Deleted,
}

/// One line of a diff view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    /// One-based base line number.
    pub old_line: Option<u32>,
    /// One-based new line number.
    pub new_line: Option<u32>,
    /// Kind.
    pub kind: LineKind,
    /// Text.
    pub text: String,
    /// Changed word ranges (UTF-16) for lines of a modified pair.
    pub words: Vec<(u32, u32)>,
}

/// A row of the side-by-side view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitRow {
    /// Base side.
    pub left: Option<DiffRow>,
    /// New side.
    pub right: Option<DiffRow>,
}

fn line_no(i: usize) -> Option<u32> {
    u32::try_from(i + 1).ok()
}

/// Rows of a hunk: deleted and added lines, with word ranges paired by
/// position for modified hunks.
fn hunk_rows(a: &[&str], b: &[&str], h: &Hunk) -> (Vec<DiffRow>, Vec<DiffRow>) {
    let mut del: Vec<DiffRow> = (0..h.old_lines)
        .map(|i| DiffRow {
            old_line: line_no(h.old_start + i),
            new_line: None,
            kind: LineKind::Deleted,
            text: a[h.old_start + i].to_owned(),
            words: Vec::new(),
        })
        .collect();
    let mut add: Vec<DiffRow> = (0..h.new_lines)
        .map(|i| DiffRow {
            old_line: None,
            new_line: line_no(h.new_start + i),
            kind: LineKind::Added,
            text: b[h.new_start + i].to_owned(),
            words: Vec::new(),
        })
        .collect();
    for (d, n) in del.iter_mut().zip(add.iter_mut()) {
        let (l, r) = word_ranges(&d.text, &n.text);
        d.words = l;
        n.words = r;
    }
    (del, add)
}

fn context(a: &[&str], x: usize, y: usize) -> DiffRow {
    DiffRow {
        old_line: line_no(x),
        new_line: line_no(y),
        kind: LineKind::Context,
        text: a[x].to_owned(),
        words: Vec::new(),
    }
}

/// The unified view: every line, deletions before additions.
pub fn unified_rows(old: &str, new: &str) -> Vec<DiffRow> {
    let (a, b) = (lines(old), lines(new));
    let mut rows = Vec::new();
    let (mut x, mut y) = (0, 0);
    for h in diff_lines(old, new) {
        while x < h.old_start {
            rows.push(context(&a, x, y));
            x += 1;
            y += 1;
        }
        let (del, add) = hunk_rows(&a, &b, &h);
        rows.extend(del);
        rows.extend(add);
        x += h.old_lines;
        y += h.new_lines;
    }
    while x < a.len() {
        rows.push(context(&a, x, y));
        x += 1;
        y += 1;
    }
    rows
}

/// The side-by-side view: base on the left, new text on the right, changed
/// lines aligned row by row.
pub fn split_rows(old: &str, new: &str) -> Vec<SplitRow> {
    let (a, b) = (lines(old), lines(new));
    let mut rows = Vec::new();
    let (mut x, mut y) = (0, 0);
    let both = |row: DiffRow| SplitRow {
        left: Some(row.clone()),
        right: Some(row),
    };
    for h in diff_lines(old, new) {
        while x < h.old_start {
            rows.push(both(context(&a, x, y)));
            x += 1;
            y += 1;
        }
        let (del, add) = hunk_rows(&a, &b, &h);
        let n = del.len().max(add.len());
        let (mut del, mut add) = (del.into_iter(), add.into_iter());
        for _ in 0..n {
            rows.push(SplitRow {
                left: del.next(),
                right: add.next(),
            });
        }
        x += h.old_lines;
        y += h.new_lines;
    }
    while x < a.len() {
        rows.push(both(context(&a, x, y)));
        x += 1;
        y += 1;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(new: &str, edit: (usize, usize, String)) -> String {
        format!("{}{}{}", &new[..edit.0], edit.2, &new[edit.1..])
    }

    #[test]
    fn finds_added_modified_and_deleted_hunks() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nB\nc\nx\ny\nd\n";
        let hunks = diff_lines(old, new);
        assert_eq!(
            hunks,
            [
                Hunk {
                    old_start: 1,
                    old_lines: 1,
                    new_start: 1,
                    new_lines: 1,
                    kind: HunkKind::Modified
                },
                Hunk {
                    old_start: 3,
                    old_lines: 0,
                    new_start: 3,
                    new_lines: 2,
                    kind: HunkKind::Added
                },
                Hunk {
                    old_start: 4,
                    old_lines: 1,
                    new_start: 6,
                    new_lines: 0,
                    kind: HunkKind::Deleted
                },
            ]
        );
        assert!(diff_lines(old, old).is_empty());
        assert!(
            diff_lines("a\r\nb\r\n", "a\nb\n").is_empty(),
            "line endings"
        );
    }

    #[test]
    fn rollback_restores_each_hunk() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nB\nc\nx\ny\nd\n";
        let hunks = diff_lines(old, new);
        assert_eq!(
            apply(new, rollback_hunk(old, new, &hunks[0]).unwrap()),
            "a\nb\nc\nx\ny\nd\n"
        );
        assert_eq!(
            apply(new, rollback_hunk(old, new, &hunks[1]).unwrap()),
            "a\nB\nc\nd\n"
        );
        assert_eq!(
            apply(new, rollback_hunk(old, new, &hunks[2]).unwrap()),
            "a\nB\nc\nx\ny\nd\ne\n"
        );
        // Rolling back every hunk, last first, gives the base.
        let mut text = new.to_owned();
        for h in hunks.iter().rev() {
            text = apply(&text, rollback_hunk(old, &text, h).unwrap());
        }
        assert_eq!(text, old);
    }

    #[test]
    fn rollback_at_the_ends_and_without_final_newline() {
        let (old, new) = ("a\nb", "a\nb\nc");
        let h = diff_lines(old, new);
        assert_eq!(h.len(), 1);
        assert_eq!(apply(new, rollback_hunk(old, new, &h[0]).unwrap()), "a\nb");
        let (old, new) = ("x\na\n", "a\n");
        let h = diff_lines(old, new);
        assert_eq!(apply(new, rollback_hunk(old, new, &h[0]).unwrap()), old);
        let (old, new) = ("", "new\n");
        let h = diff_lines(old, new);
        assert_eq!(apply(new, rollback_hunk(old, new, &h[0]).unwrap()), "");
        assert_eq!(
            hunk_base_text("a\nb\nc\n", &diff_lines("a\nb\nc\n", "a\nc\n")[0]),
            "b"
        );
    }

    #[test]
    fn diff_is_minimal_on_interleaved_changes() {
        let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let new: String = (0..200)
            .map(|i| {
                if i % 10 == 0 {
                    format!("changed {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let hunks = diff_lines(&old, &new);
        assert_eq!(hunks.len(), 20);
        assert!(hunks.iter().all(|h| h.old_lines == 1 && h.new_lines == 1));
    }

    #[test]
    fn huge_rewrites_fall_back_to_one_hunk() {
        let old: String = (0..5000).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..5000).map(|i| format!("b{i}\n")).collect();
        let hunks = diff_lines(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].old_lines, hunks[0].new_lines), (5000, 5000));
    }

    #[test]
    fn word_ranges_mark_changed_words() {
        let (l, r) = word_ranges("const total = sum(a);", "const count = sum(a, b);");
        assert_eq!(l, [(6, 11)]);
        assert_eq!(r, [(6, 11), (19, 22)]);
        // UTF-16 offsets.
        let (_, r) = word_ranges("é x", "é y");
        assert_eq!(r, [(2, 3)]);
    }

    #[test]
    fn unified_and_split_rows() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let u = unified_rows(old, new);
        let kinds: Vec<(LineKind, &str)> = u.iter().map(|r| (r.kind, r.text.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (LineKind::Context, "a"),
                (LineKind::Deleted, "b"),
                (LineKind::Added, "B"),
                (LineKind::Context, "c"),
                (LineKind::Added, "d"),
            ]
        );
        assert_eq!((u[3].old_line, u[3].new_line), (Some(3), Some(3)));
        assert_eq!(u[1].words, [(0, 1)]);
        let s = split_rows(old, new);
        assert_eq!(s.len(), 4);
        assert_eq!(s[1].left.as_ref().unwrap().text, "b");
        assert_eq!(s[1].right.as_ref().unwrap().text, "B");
        assert!(s[3].left.is_none());
        assert_eq!(s[3].right.as_ref().unwrap().new_line, Some(4));
    }
}
