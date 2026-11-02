//! Bounded display-only word matching. Never changes raw patch line identities.
use crate::diff::{Kind, Line};
use rocket_git::{CancellationToken, Result};
use std::{collections::BTreeMap, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

const MAX_CELLS: usize = 2_000_000;
const MAX_PAIR_CELLS: usize = 65_536;

#[derive(Default)]
pub struct Highlights {
    pub spans: BTreeMap<usize, Vec<Range<usize>>>,
    pub limited: bool,
}

pub fn compute(lines: &[Line], cancel: &CancellationToken) -> Result<Highlights> {
    let mut result = Highlights::default();
    let mut budget = MAX_CELLS;
    let mut i = 0;
    while i < lines.len() {
        cancel.check()?;
        let mut removed = Vec::new();
        let mut added = Vec::new();
        while i < lines.len() && matches!(lines[i].kind, Kind::Removed | Kind::Added) {
            if lines[i].kind == Kind::Removed {
                removed.push(i);
            } else {
                added.push(i);
            }
            i += 1;
        }
        for (left, right) in removed.into_iter().zip(added) {
            cancel.check()?;
            // Match the same positional pairs shown in Rocket's split view.
            let a = tokens(&lines[left].text);
            let b = tokens(&lines[right].text);
            let mut prefix = 0;
            while prefix < a.len().min(b.len()) && a[prefix].1 == b[prefix].1 {
                prefix += 1;
            }
            let mut suffix = 0;
            while suffix < a.len().min(b.len()) - prefix
                && a[a.len() - suffix - 1].1 == b[b.len() - suffix - 1].1
            {
                suffix += 1;
            }
            let (an, bn) = (a.len() - prefix - suffix, b.len() - prefix - suffix);
            let cells = (an + 1).saturating_mul(bn + 1);
            if cells > MAX_PAIR_CELLS || cells > budget {
                result.limited = true;
                continue; // The normal whole-line tint remains visible.
            }
            budget -= cells;
            let mut table = vec![0_u16; cells];
            let stride = bn + 1;
            for x in (0..an).rev() {
                cancel.check()?;
                for y in (0..bn).rev() {
                    table[x * stride + y] = if a[prefix + x].1 == b[prefix + y].1 {
                        1 + table[(x + 1) * stride + y + 1]
                    } else {
                        table[(x + 1) * stride + y].max(table[x * stride + y + 1])
                    };
                }
            }
            let (mut x, mut y) = (0, 0);
            let (mut old, mut new) = (Vec::new(), Vec::new());
            while x < an || y < bn {
                if x < an && y < bn && a[prefix + x].1 == b[prefix + y].1 {
                    x += 1;
                    y += 1;
                } else if x < an
                    && (y == bn || table[(x + 1) * stride + y] >= table[x * stride + y + 1])
                {
                    append(&mut old, &a[prefix + x]);
                    x += 1;
                } else {
                    append(&mut new, &b[prefix + y]);
                    y += 1;
                }
            }
            if !old.is_empty() {
                result.spans.insert(left, old);
            }
            if !new.is_empty() {
                result.spans.insert(right, new);
            }
        }
        if i < lines.len() {
            i += 1;
        }
    }
    cancel.check()?;
    Ok(result)
}

fn tokens(line: &str) -> Vec<(usize, &str)> {
    // UAX word boundaries retain combining marks and emoji sequences together.
    line.get(1..)
        .unwrap_or_default()
        .split_word_bound_indices()
        .map(|(i, word)| (i + 1, word))
        .collect()
}
fn append(out: &mut Vec<Range<usize>>, token: &(usize, &str)) {
    let end = token.0 + token.1.len();
    if let Some(last) = out.last_mut()
        && last.end == token.0
    {
        last.end = end;
    } else {
        out.push(token.0..end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Patch;

    #[test]
    fn unicode_words_preserve_clusters_and_original_patch_identities() {
        let p = Patch::parse("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n-let greeting = \"Hello 👩‍💻 café\";\n-render(12);\n+let greeting = \"你好 👨‍💻 café\";\n+render(16);\n".as_bytes());
        let lines = &p.files[0].lines;
        let h = compute(lines, &CancellationToken::new()).unwrap();
        let highlighted = |i| {
            h.spans[&i]
                .iter()
                .map(|r| &lines[i].text[r.clone()])
                .collect::<String>()
        };
        assert_eq!(highlighted(4), "Hello👩‍💻café");
        assert_eq!(highlighted(6), "你好👨‍💻café");
        assert_eq!(highlighted(5), "12");
        assert_eq!(highlighted(7), "16");
        assert_eq!(lines[4].raw_index, 4);
        assert!(!h.limited);
    }

    #[test]
    fn repeated_words_whitespace_and_bounded_fallback() {
        let bytes = format!(
            "@@ -1,2 +1,2 @@\n-one one two\n-{}\n+one two one\n+{}\n",
            "a ".repeat(2000),
            "b ".repeat(2000)
        );
        let p = Patch::parse(bytes.as_bytes());
        let h = compute(&p.files[0].lines, &CancellationToken::new()).unwrap();
        assert!(h.limited);
        assert!(h.spans.contains_key(&1));
        assert!(h.spans.contains_key(&3));
        assert!(!h.spans.contains_key(&2));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            compute(&p.files[0].lines, &cancel),
            Err(rocket_git::Error::Cancelled)
        ));
    }
}
