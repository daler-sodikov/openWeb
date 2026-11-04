//! IntelliJ-style fuzzy matching: the pattern's characters must appear in
//! order (case-insensitively); matches at word starts, camel humps and in
//! consecutive runs score higher.

/// A successful match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Higher is better.
    pub score: i64,
    /// Matched char indices in the candidate, ascending.
    pub positions: Vec<usize>,
}

const MATCH: i64 = 16;
const START_BONUS: i64 = 32;
const BOUNDARY_BONUS: i64 = 24;
const CAMEL_BONUS: i64 = 22;
const CONSECUTIVE_BONUS: i64 = 18;
const CASE_BONUS: i64 = 1;
const GAP_PENALTY: i64 = 1;
/// Bonus for matching inside the file name rather than the directory.
const FILE_NAME_BONUS: i64 = 200;

fn is_separator(c: char) -> bool {
    matches!(c, '/' | '\\' | '.' | '_' | '-' | ' ' | ':')
}

fn position_bonus(chars: &[char], j: usize) -> i64 {
    let Some(&c) = chars.get(j) else {
        return 0;
    };
    if j == 0 {
        return START_BONUS;
    }
    let prev = chars[j - 1];
    if is_separator(prev) {
        BOUNDARY_BONUS
    } else if (prev.is_lowercase() && c.is_uppercase())
        || (prev.is_alphabetic() && c.is_ascii_digit())
    {
        CAMEL_BONUS
    } else {
        0
    }
}

fn eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Matches `pattern` against `candidate`. Whitespace in the pattern is
/// ignored. Returns `None` if the pattern is not a subsequence.
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<FuzzyMatch> {
    let p: Vec<char> = pattern.chars().filter(|c| !c.is_whitespace()).collect();
    let c: Vec<char> = candidate.chars().collect();
    if p.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    // Quick subsequence check.
    let mut it = c.iter();
    if !p.iter().all(|pc| it.any(|cc| eq_ignore_case(*pc, *cc))) {
        return None;
    }

    let (m, n) = (p.len(), c.len());
    const NONE: i64 = i64::MIN / 4;
    // score[i][j]: best score with p[i] matched at c[j]; from[i][j]: where p[i-1] matched.
    let mut score = vec![vec![NONE; n]; m];
    let mut from = vec![vec![usize::MAX; n]; m];
    for i in 0..m {
        // Best (score[i-1][k] - gap) over k <= j - 2, maintained incrementally.
        let mut run = NONE;
        let mut run_k = usize::MAX;
        for j in 0..n {
            if i > 0 && j >= 2 {
                let candidate_run = score[i - 1][j - 2];
                if run != NONE {
                    run -= GAP_PENALTY;
                }
                if candidate_run != NONE && candidate_run - GAP_PENALTY > run {
                    run = candidate_run - GAP_PENALTY;
                    run_k = j - 2;
                }
            }
            if !eq_ignore_case(p[i], c[j]) {
                continue;
            }
            let base = MATCH + position_bonus(&c, j) + if p[i] == c[j] { CASE_BONUS } else { 0 };
            if i == 0 {
                // Small penalty for where the match starts.
                score[i][j] = base - (j as i64).min(16);
                continue;
            }
            let mut best = NONE;
            let mut best_k = usize::MAX;
            if j >= 1 && score[i - 1][j - 1] != NONE {
                best = score[i - 1][j - 1] + CONSECUTIVE_BONUS;
                best_k = j - 1;
            }
            if run != NONE && run > best {
                best = run;
                best_k = run_k;
            }
            if best != NONE {
                score[i][j] = best + base;
                from[i][j] = best_k;
            }
        }
    }

    let (end, &best) = score[m - 1]
        .iter()
        .enumerate()
        .filter(|(_, s)| **s != NONE)
        .max_by_key(|(j, s)| (**s, std::cmp::Reverse(*j)))?;
    let mut positions = vec![0; m];
    let mut j = end;
    for i in (0..m).rev() {
        positions[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    Some(FuzzyMatch {
        score: best,
        positions,
    })
}

/// Matches a project-relative path (`/` separators). Without `/` in the
/// pattern, the file name is matched first and preferred; positions are
/// always char indices into the full path.
pub fn match_path(pattern: &str, path: &str) -> Option<FuzzyMatch> {
    let name_start = path.rfind('/').map_or(0, |i| i + 1);
    if !pattern.contains('/') {
        let name = &path[name_start..];
        if let Some(mut m) = fuzzy_match(pattern, name) {
            let offset = path[..name_start].chars().count();
            m.positions.iter_mut().for_each(|p| *p += offset);
            m.score += FILE_NAME_BONUS;
            return Some(m);
        }
    }
    fuzzy_match(pattern, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best<'a>(pattern: &str, candidates: &[&'a str]) -> Vec<&'a str> {
        let mut scored: Vec<_> = candidates
            .iter()
            .filter_map(|c| match_path(pattern, c).map(|m| (m.score, *c)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("xyz", "Counter").is_none());
        assert!(fuzzy_match("cnt", "Counter").is_some());
        assert_eq!(fuzzy_match("", "abc").map(|m| m.score), Some(0));
    }

    #[test]
    fn camel_humps_and_word_starts_win() {
        let m = fuzzy_match("fP", "formatPrice").unwrap();
        assert_eq!(m.positions, vec![0, 6]);
        let m = fuzzy_match("ap", "AppProps").unwrap();
        assert_eq!(m.positions, vec![0, 3], "camel humps: A and P");
        let m = fuzzy_match("pp", "AppProps").unwrap();
        assert_eq!(m.positions, vec![2, 3], "consecutive + hump");
        let hump = fuzzy_match("sb", "SearchBar").unwrap().score;
        let inner = fuzzy_match("sb", "isabel").unwrap().score;
        assert!(hump > inner);
    }

    #[test]
    fn file_names_rank_above_directories() {
        let files = [
            "src/components/Counter.tsx",
            "src/counter/index.ts",
            "src/utils/format.ts",
            "docs/contents.md",
        ];
        let ranked = best("cnt", &files);
        let pos = |f: &str| ranked.iter().position(|r| *r == f).unwrap();
        assert!(
            pos("src/components/Counter.tsx") < pos("src/counter/index.ts"),
            "file-name match beats directory-only match: {ranked:?}"
        );
        assert_eq!(best("format", &files), vec!["src/utils/format.ts"]);
        let m = match_path("cnt", "src/components/Counter.tsx").unwrap();
        assert_eq!(m.positions[0], 15, "positions index the full path");
    }

    #[test]
    fn slash_patterns_match_paths() {
        let files = ["src/utils/format.ts", "src/format/x.ts", "lib/utils.ts"];
        assert_eq!(best("ut/fo", &files)[0], "src/utils/format.ts");
    }

    #[test]
    fn non_ascii() {
        let m = fuzzy_match("éc", "Écran").unwrap();
        assert_eq!(m.positions, vec![0, 1]);
    }
}
