//! Azure exposes commit-pinned file content, not unified PR patches. Build a
//! small, deterministic line diff in memory; never materialize repository files.
use super::*;

pub(super) fn patch(old: &str, new: &str, c: &CancellationToken) -> Result<Option<String>> {
    c.check()?;
    if old.len() > 128 * 1024 || new.len() > 128 * 1024 || old.contains('\0') || new.contains('\0')
    {
        return Ok(None);
    }
    let a: Vec<_> = old.split_inclusive('\n').collect();
    let b: Vec<_> = new.split_inclusive('\n').collect();
    if a.len() > 1000 || b.len() > 1000 {
        return Ok(None);
    }
    // At most ~2 MiB, bounded independently of response size and similarity.
    let width = b.len() + 1;
    let mut lcs = vec![0u16; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        c.check()?;
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                1 + lcs[(i + 1) * width + j + 1]
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut rows = Vec::new();
    while i < a.len() || j < b.len() {
        let (kind, text) = if i < a.len() && j < b.len() && a[i] == b[j] {
            let text = a[i];
            i += 1;
            j += 1;
            (' ', text)
        } else if i < a.len()
            && (j == b.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
        {
            let text = a[i];
            i += 1;
            ('-', text)
        } else {
            let text = b[j];
            j += 1;
            ('+', text)
        };
        rows.push((kind, text));
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (n, (kind, _)) in rows.iter().enumerate() {
        if *kind == ' ' {
            continue;
        }
        let start = n.saturating_sub(3);
        let end = (n + 4).min(rows.len());
        if let Some(last) = spans.last_mut().filter(|last| last.1 >= start) {
            last.1 = end;
        } else {
            spans.push((start, end));
        }
    }
    let mut result = String::new();
    for (start, end) in spans {
        c.check()?;
        let old_start = rows[..start].iter().filter(|r| r.0 != '+').count() + 1;
        let new_start = rows[..start].iter().filter(|r| r.0 != '-').count() + 1;
        let old_count = rows[start..end].iter().filter(|r| r.0 != '+').count();
        let new_count = rows[start..end].iter().filter(|r| r.0 != '-').count();
        result.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start - usize::from(old_count == 0),
            old_count,
            new_start - usize::from(new_count == 0),
            new_count
        ));
        for (kind, text) in &rows[start..end] {
            result.push(*kind);
            result.push_str(text.strip_suffix('\n').unwrap_or(text));
            result.push('\n');
            if !text.ends_with('\n') {
                result.push_str("\\ No newline at end of file\n");
            }
        }
        if result.len() > 16384 {
            return Ok(None);
        }
    }
    Ok((!result.is_empty()).then_some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn azure_diff_preserves_unicode_add_delete_and_final_newline_locations() {
        let c = CancellationToken::new();
        for (old, new, removed, added) in [
            ("", "資料\n", 0, 1),
            ("旧\n", "", 1, 0),
            ("same\nold\n", "same\nnew\n", 1, 1),
            ("line", "line\n", 1, 1),
        ] {
            let patch = patch(old, new, &c).unwrap().unwrap();
            let rows = inline::parse_lines(&patch).unwrap();
            assert_eq!(
                rows.iter().filter(|r| r.text.starts_with('-')).count(),
                removed
            );
            assert_eq!(
                rows.iter().filter(|r| r.text.starts_with('+')).count(),
                added
            );
        }
    }
    #[test]
    fn azure_diff_bounds_work_and_observes_cancellation() {
        let c = CancellationToken::new();
        assert!(patch(&"a\n".repeat(1001), "b", &c).unwrap().is_none());
        assert!(patch("a\0", "b", &c).unwrap().is_none());
        c.cancel();
        assert!(matches!(patch("a", "b", &c), Err(Error::Cancelled)));
    }
}
