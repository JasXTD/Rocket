use super::MAP_LINKS;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// A source location connecting two files in a particular Git snapshot. Lines start at one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssetReference {
    pub source: Arc<[u8]>,
    pub line: u32,
    pub target: Arc<[u8]>,
    pub kind: &'static str,
    /// Half-open UTF-8 byte columns within the committed source line.
    pub columns: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceLocation {
    pub line: u32,
    pub columns: (u32, u32),
}

/// Both ends open the same source location. Reuse paths for repeated references.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetReferences {
    files: BTreeMap<Arc<[u8]>, BTreeSet<AssetReference>>,
    count: usize,
    pub limited: bool,
}

impl AssetReferences {
    pub fn for_file(&self, path: &[u8]) -> Option<&BTreeSet<AssetReference>> {
        self.files.get(path)
    }

    pub(super) fn add(&mut self, source: &[u8], target: &[u8], location: SourceLocation, kind: &'static str) {
        if self.count >= MAP_LINKS && self.limited {
            return;
        }
        let path = |p: &[u8]| {
            self.files
                .get_key_value(p)
                .map_or_else(|| Arc::from(p), |(key, _)| key.clone())
        };
        let site = AssetReference {
            source: path(source),
            target: path(target),
            line: location.line,
            columns: location.columns,
            kind,
        };
        if self.count == MAP_LINKS {
            // A second folder read may replay an already indexed site at the limit.
            self.limited = !self
                .files
                .get(source)
                .is_some_and(|sites| sites.contains(&site));
        } else if self
            .files
            .entry(site.source.clone())
            .or_default()
            .insert(site.clone())
        {
            self.files
                .entry(site.target.clone())
                .or_default()
                .insert(site);
            self.count += 1;
        }
    }

    pub(super) fn merge(&mut self, other: Self) {
        self.limited |= other.limited;
        for (path, sites) in other.files {
            for site in sites {
                if site.source == path {
                    self.add(&path, &site.target, SourceLocation { line: site.line, columns: site.columns }, site.kind);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rereading_at_capacity_keeps_distinct_lines_and_flags_only_new_sites() {
        let mut index = AssetReferences::default();
        let site = |line| SourceLocation { line, columns: (0, 1) };
        for line in 1..=MAP_LINKS as u32 {
            index.add(b"caller", b"asset", site(line), "url()");
        }
        index.add(b"caller", b"asset", site(2), "url()");
        assert!(!index.limited);
        index.add(b"caller", b"another", site(2), "url()");
        assert!(index.limited);
        assert_eq!(index.for_file(b"asset").unwrap().len(), MAP_LINKS);
        assert_eq!(index.for_file(b"caller"), index.for_file(b"asset"));
        assert!(index.for_file(b"another").is_none());
    }
}
