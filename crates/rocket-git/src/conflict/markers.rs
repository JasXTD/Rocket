use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
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
    ConflictMarkers::from_sides(7, []).blocks(text)
}

/// The marker width Git used and literal underline lines present in the index sides.
/// An orphaned separator in an edited draft still needs explicit resolution.
#[derive(Clone, Debug)]
pub struct ConflictMarkers {
    width: usize,
    headings: BTreeMap<usize, BTreeSet<String>>,
}
impl ConflictMarkers {
    pub fn from_sides<'a>(width: usize, sides: impl IntoIterator<Item = &'a str>) -> Self {
        let mut headings: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for side in sides {
            let mut previous = "";
            for line in side.lines() {
                if (line.len() >= 7 || line.len() == width)
                    && !previous.trim().is_empty()
                    && line.bytes().all(|b| b == b'=')
                {
                    headings
                        .entry(line.len())
                        .or_default()
                        .insert(previous.to_owned());
                }
                previous = line;
            }
        }
        Self { width, headings }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn blocks(&self, text: &str) -> ConflictBlocks {
        let mut result = ConflictBlocks::default();
        let mut previous = "";
        let mut heading_separators: Vec<Range<usize>> = Vec::new();
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
                (count >= 7 || count == self.width)
                    && (trimmed.len() == count
                        || (*m != b'=' && trimmed.as_bytes().get(count) == Some(&b' ')))
            });
            if let Some(m) = marker {
                // A separator can directly follow text that was a heading in another
                // side. Use that line only if the block has no other possible separator.
                if m == b'>'
                    && start.is_some()
                    && count == width
                    && theirs.is_none()
                    && let [separator] = heading_separators.as_slice()
                {
                    ours_end.get_or_insert(separator.start);
                    theirs = Some(separator.end);
                }
                match m {
                    b'=' if self
                        .headings
                        .get(&count)
                        .is_some_and(|titles| titles.contains(previous)) =>
                    {
                        if start.is_some() && count == width && theirs.is_none() {
                            heading_separators.push(offset..offset + line.len());
                        }
                    }
                    b'<' => {
                        result.malformed |= start.is_some();
                        heading_separators.clear();
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
                        ours_end = Some(offset);
                        heading_separators.clear();
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
            previous = trimmed;
            offset += line.len();
        }
        result.malformed |= start.is_some();
        result
    }
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
    fn literal_underlines_inside_a_conflict_preserve_each_sides_byte_range() {
        let sides = [
            "Local title\n=======\nlocal\n",
            "Base title\n=======\nbase\n",
            "Their title\n=======\ntheirs\n",
        ];
        let rules = ConflictMarkers::from_sides(7, sides);
        for base in [String::new(), format!("||||||| base\n{}", sides[1])] {
            let text = format!(
                "<<<<<<< ours\n{}{base}=======\n{}>>>>>>> theirs\n",
                sides[0], sides[2]
            );
            let parsed = rules.blocks(&text);
            assert!(!parsed.malformed);
            assert_eq!(parsed.blocks.len(), 1);
            assert_eq!(&text[parsed.blocks[0].ours.clone()], sides[0]);
            assert_eq!(&text[parsed.blocks[0].theirs.clone()], sides[2]);
        }
        // A real separator can also follow a title known from a different side.
        let text = "<<<<<<< ours\nLocal title\n=======\nother\n>>>>>>> theirs\n";
        let parsed = rules.blocks(text);
        assert!(!parsed.malformed);
        assert_eq!(&text[parsed.blocks[0].ours.clone()], "Local title\n");
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
