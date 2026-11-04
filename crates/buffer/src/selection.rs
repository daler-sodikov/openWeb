//! Multi-range selections in UTF-16 offsets.

/// One selection range. `anchor == head` is a caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelRange {
    /// Fixed end of the range.
    pub anchor: usize,
    /// Moving end of the range (where the caret is drawn).
    pub head: usize,
}

impl SelRange {
    /// A caret at `pos`.
    pub fn cursor(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// A range from `anchor` to `head`.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Start of the range.
    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// End of the range.
    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// Whether this range is a caret.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// A set of ranges with one main range (the primary caret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The ranges. Normalized selections are sorted and non-overlapping.
    pub ranges: Vec<SelRange>,
    /// Index of the main range in `ranges`.
    pub main: usize,
}

impl Selection {
    /// A single caret at `pos`.
    pub fn cursor(pos: usize) -> Self {
        Self::single(SelRange::cursor(pos))
    }

    /// A selection with one range.
    pub fn single(range: SelRange) -> Self {
        Self {
            ranges: vec![range],
            main: 0,
        }
    }

    /// The main range. An empty selection is treated as a caret at 0.
    pub fn main_range(&self) -> SelRange {
        self.ranges
            .get(self.main)
            .or_else(|| self.ranges.first())
            .copied()
            .unwrap_or(SelRange::cursor(0))
    }

    /// Clamps all offsets to `len`, sorts the ranges and merges overlapping
    /// ones, keeping track of the main range.
    pub fn normalized(self, len: usize) -> Self {
        let main_range = self.main_range();
        let mut ranges: Vec<SelRange> = self
            .ranges
            .into_iter()
            .map(|r| SelRange::new(r.anchor.min(len), r.head.min(len)))
            .collect();
        if ranges.is_empty() {
            return Self::cursor(0);
        }
        let main_clamped = SelRange::new(main_range.anchor.min(len), main_range.head.min(len));
        ranges.sort_by_key(|r| (r.from(), r.to()));

        let mut merged: Vec<SelRange> = Vec::with_capacity(ranges.len());
        let mut main = 0;
        for range in ranges {
            let overlaps = merged.last().is_some_and(|last: &SelRange| {
                range.from() < last.to()
                    || (range.from() == last.to() && (range.is_empty() || last.is_empty()))
            });
            if overlaps && let Some(last) = merged.last_mut() {
                let from = last.from().min(range.from());
                let to = last.to().max(range.to());
                *last = if last.head < last.anchor {
                    SelRange::new(to, from)
                } else {
                    SelRange::new(from, to)
                };
            } else {
                merged.push(range);
            }
            if range == main_clamped {
                main = merged.len() - 1;
            }
        }
        Self {
            ranges: merged,
            main,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_tracks_main() {
        let sel = Selection {
            ranges: vec![
                SelRange::cursor(10),
                SelRange::new(2, 5),
                SelRange::cursor(4),
            ],
            main: 0,
        }
        .normalized(8);
        assert_eq!(sel.ranges, vec![SelRange::new(2, 5), SelRange::cursor(8)]);
        assert_eq!(sel.main, 1);
    }

    #[test]
    fn duplicate_carets_merge() {
        let sel = Selection {
            ranges: vec![SelRange::cursor(3), SelRange::cursor(3)],
            main: 1,
        }
        .normalized(10);
        assert_eq!(sel, Selection::cursor(3));
    }

    #[test]
    fn adjacent_non_empty_ranges_stay_separate() {
        let sel = Selection {
            ranges: vec![SelRange::new(0, 2), SelRange::new(2, 4)],
            main: 1,
        }
        .normalized(10);
        assert_eq!(sel.ranges.len(), 2);
        assert_eq!(sel.main, 1);
    }
}
