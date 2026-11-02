//! Bounded line anchors for linked conflict scrolling. Unique unchanged lines
//! anchor the panes; changed/repeated regions interpolate between anchors.
//! This maps positions only: no padding or synthetic text enters the editor.
use std::collections::HashMap;
const MAX_LINES: usize = 20_000;

#[derive(Clone, Debug, Default)]
pub struct LineMap {
    points: Vec<(f32, f32)>,
}
impl LineMap {
    pub fn new(result: &str, side: &str) -> Self {
        let rc = result.lines().count().max(1);
        let sc = side.lines().count().max(1);
        let mut points = vec![(0.0, 0.0)];
        if rc <= MAX_LINES && sc <= MAX_LINES {
            let mut unique = HashMap::new();
            for (i, line) in side.lines().enumerate() {
                unique
                    .entry(line)
                    .and_modify(|v: &mut Option<usize>| *v = None)
                    .or_insert(Some(i));
            }
            let mut counts = HashMap::new();
            for line in result.lines() {
                *counts.entry(line).or_insert(0usize) += 1;
            }
            let candidates: Vec<_> = result
                .lines()
                .enumerate()
                .filter_map(|(i, line)| {
                    (counts[line] == 1 && !line.trim().is_empty())
                        .then(|| unique.get(line).copied().flatten().map(|j| (i, j)))
                        .flatten()
                })
                .collect();
            // Longest increasing subsequence prevents crossing anchors when
            // code moved. O(n log n), never a quadratic whole-file diff.
            let mut tails: Vec<usize> = Vec::new();
            let mut previous = vec![None; candidates.len()];
            for (i, &(_, side)) in candidates.iter().enumerate() {
                let position = tails.partition_point(|&t| candidates[t].1 < side);
                if position > 0 {
                    previous[i] = Some(tails[position - 1]);
                }
                if position == tails.len() {
                    tails.push(i);
                } else {
                    tails[position] = i;
                }
            }
            let mut chain = Vec::new();
            let mut next = tails.last().copied();
            while let Some(i) = next {
                chain.push(candidates[i]);
                next = previous[i];
            }
            for (a, b) in chain.into_iter().rev() {
                if a > 0 && b > 0 {
                    points.push((a as f32, b as f32));
                }
            }
        }
        points.push((rc as f32, sc as f32));
        Self { points }
    }
    pub fn to_side(&self, row: f32) -> f32 {
        self.map(row, false)
    }
    pub fn to_result(&self, row: f32) -> f32 {
        self.map(row, true)
    }
    fn map(&self, row: f32, inverse: bool) -> f32 {
        let coordinates = |p: (f32, f32)| if inverse { (p.1, p.0) } else { p };
        let row = if row.is_finite() { row.max(0.0) } else { 0.0 };
        let i = self.points.partition_point(|&p| coordinates(p).0 <= row);
        if i == 0 {
            return 0.0;
        }
        let (a, b) = coordinates(self.points[i - 1]);
        // Wrapping can give a pane more rows than its file has lines; those run on row for row.
        let Some(&next) = self.points.get(i) else {
            return b + (row - a);
        };
        let (c, d) = coordinates(next);
        b + (row - a) / (c - a).max(1.0) * (d - b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linked_positions_follow_common_lines_after_insertions_and_unicode() {
        let result = "start\n<<<<<<< ours\n日本語 🚀\n=======\ncafé\n>>>>>>> theirs\nanchor\nend\n";
        let map = LineMap::new(result, "start\n日本語 🚀\nanchor\nend\n");
        assert_eq!(map.to_side(6.0), 2.0);
        assert_eq!(map.to_result(2.0), 6.0);
        for row in 0..8 {
            assert!(map.to_side(row as f32) <= map.to_side(row as f32 + 0.5));
        }
        let moved = LineMap::new("a\nb\nc\nd\n", "a\nc\nb\nd\n");
        for row in 0..4 {
            assert!(moved.to_side(row as f32) <= moved.to_side(row as f32 + 0.5));
        }
    }
    #[test]
    fn rows_past_the_last_line_stay_reachable_both_ways() {
        let map = LineMap::new("a\nb\nc\n", "a\nb\n");
        assert_eq!(map.to_side(3.0), 2.0);
        assert_eq!(map.to_side(5.0), 4.0);
        assert_eq!(map.to_result(4.0), 5.0);
    }
    #[test]
    fn repeated_and_large_files_stay_bounded_without_modifying_text() {
        let map = LineMap::new(&"same\n".repeat(MAX_LINES + 1), "same\nsame\n");
        assert_eq!(map.points.len(), 2);
        assert_eq!(map.to_side((MAX_LINES + 1) as f32), 2.0);
        assert_eq!(map.to_side(f32::NAN), 0.0);
        let empty = LineMap::new("", "");
        assert_eq!(empty.to_side(0.0), 0.0);
    }
}
