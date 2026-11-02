use std::ops::Range;
#[derive(Clone, Debug)]
pub struct ConflictBlock {
    pub range: Range<usize>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
}
#[derive(Clone, Debug, Default)]
pub struct ConflictBlocks {
    pub blocks: Vec<ConflictBlock>,
    pub malformed: bool,
}
impl ConflictBlocks {
    pub fn is_resolved(&self) -> bool {
        !self.malformed && self.blocks.is_empty()
    }
}
/// Byte ranges into the editable UTF-8 result. Supports merge/diff3/zdiff3 and
/// custom marker widths; malformed markers block staging but never block saving.
pub fn conflict_blocks(text: &str) -> ConflictBlocks {
    let mut result = ConflictBlocks::default();
    let (mut offset, mut start, mut ours, mut ours_end, mut theirs, mut width) =
        (0, None, 0, None, None, 0);
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let marker = trimmed
            .as_bytes()
            .first()
            .copied()
            .filter(|b| b"<|=>".contains(b));
        let count = marker.map_or(0, |m| trimmed.bytes().take_while(|b| *b == m).count());
        let marker = marker.filter(|m| {
            count >= 7
                && (trimmed.len() == count
                    || (*m != b'=' && trimmed.as_bytes().get(count) == Some(&b' ')))
        });
        if let Some(m) = marker {
            match m {
                b'<' if start.is_none() => {
                    start = Some(offset);
                    ours = offset + line.len();
                    width = count;
                    ours_end = None;
                    theirs = None;
                }
                b'|' if start.is_some()
                    && theirs.is_none()
                    && ours_end.is_none()
                    && count == width =>
                {
                    ours_end = Some(offset)
                }
                b'=' if start.is_some() && theirs.is_none() && count == width => {
                    ours_end.get_or_insert(offset);
                    theirs = Some(offset + line.len());
                }
                b'>' if start.is_some() && theirs.is_some() && count == width => {
                    result.blocks.push(ConflictBlock {
                        range: start.take().unwrap()..offset + line.len(),
                        ours: ours..ours_end.unwrap(),
                        theirs: theirs.take().unwrap()..offset,
                    });
                }
                _ => result.malformed = true,
            }
        }
        offset += line.len();
    }
    result.malformed |= start.is_some();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_preserve_unicode_diff3_crlf_and_custom_markers() {
        let s = "keep 🚀\r\n<<<<<<<<<< ours\r\n中文\r\n|||||||||| base\r\nold\r\n==========\r\né\r\n>>>>>>>>>> theirs\r\nend";
        let p = conflict_blocks(s);
        assert!(!p.malformed);
        assert_eq!(p.blocks.len(), 1);
        let b = &p.blocks[0];
        assert_eq!(&s[b.ours.clone()], "中文\r\n");
        assert_eq!(&s[b.theirs.clone()], "é\r\n");
        let resolved = format!(
            "{}{}{}",
            &s[..b.range.start],
            &s[b.ours.clone()],
            &s[b.range.end..]
        );
        assert_eq!(resolved, "keep 🚀\r\n中文\r\nend");
        assert!(conflict_blocks(&resolved).is_resolved());
    }
    #[test]
    fn incomplete_and_nested_markers_never_look_resolved() {
        for text in [
            "<<<<<<< ours\na\n",
            "=======\n",
            "<<<<<<< ours\n<<<<<<< nested\n=======\nx\n>>>>>>> end\n",
            "<<<<<<< ours\na\n========\nb\n>>>>>>> theirs\n",
        ] {
            assert!(!conflict_blocks(text).is_resolved());
        }
        assert!(conflict_blocks("a << b\n// ======= explanatory\n").is_resolved());
    }
}
