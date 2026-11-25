use super::MAP_LINKS;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// A literal dependency in a particular Git snapshot. Lines start at one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssetReference {
    pub source: Arc<[u8]>,
    pub line: u32,
    pub target: Arc<[u8]>,
    pub kind: &'static str,
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

    pub(super) fn add(&mut self, source: &[u8], target: &[u8], line: u32, kind: &'static str) {
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
            line,
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
                    self.add(&path, &site.target, site.line, site.kind);
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
        for line in 1..=MAP_LINKS as u32 {
            index.add(b"caller", b"asset", line, "url()");
        }
        index.add(b"caller", b"asset", 2, "url()");
        assert!(!index.limited);
        index.add(b"caller", b"another", 2, "url()");
        assert!(index.limited);
        assert_eq!(index.for_file(b"asset").unwrap().len(), MAP_LINKS);
        assert_eq!(index.for_file(b"caller"), index.for_file(b"asset"));
        assert!(index.for_file(b"another").is_none());
    }
}
