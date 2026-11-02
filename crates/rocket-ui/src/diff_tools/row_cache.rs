//! Cache formatting instructions only. egui retains ownership of font/atlas
//! invalidation, including DPI changes, zoom and replacement font definitions.
use eframe::egui::{Color32, text::LayoutJob};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

const MAX_ROWS: usize = 256;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Style {
    pub file: u64,
    pub syntax: bool,
    pub words: bool,
    pub marks: bool,
    pub font: u8,
    pub current_match: usize,
    /// The theme palette the rows' syntax and highlight colours came from.
    pub palette: u64,
}

#[derive(Default)]
pub(super) struct Cache {
    style: Option<Style>,
    rows: HashMap<usize, (Color32, Arc<LayoutJob>, usize)>,
    order: VecDeque<usize>,
    bytes: usize,
}

impl Cache {
    pub fn clear(&mut self) {
        self.rows.clear();
        self.order.clear();
        self.bytes = 0;
    }

    pub fn get(&mut self, style: Style, row: usize, color: Color32) -> Option<Arc<LayoutJob>> {
        if self.style != Some(style) {
            self.clear();
            self.style = Some(style);
        }
        self.rows
            .get(&row)
            .filter(|(old, _, _)| *old == color)
            .map(|(_, job, _)| job.clone())
    }

    pub fn insert(&mut self, row: usize, color: Color32, job: Arc<LayoutJob>) {
        let bytes = job.text.capacity()
            + job.sections.capacity() * std::mem::size_of::<eframe::egui::text::LayoutSection>()
            + std::mem::size_of::<LayoutJob>();
        if let Some((_, _, old_bytes)) = self.rows.remove(&row) {
            self.bytes -= old_bytes;
            self.order.retain(|old| *old != row);
        }
        if bytes > MAX_BYTES {
            return;
        }
        while self.rows.len() >= MAX_ROWS || self.bytes + bytes > MAX_BYTES {
            if let Some(old) = self.order.pop_front()
                && let Some((_, _, size)) = self.rows.remove(&old)
            {
                self.bytes -= size;
            }
        }
        self.bytes += bytes;
        self.order.push_back(row);
        self.rows.insert(row, (color, job, bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_bounded_and_style_and_color_changes_miss() {
        let mut cache = Cache::default();
        let style = Style {
            file: 1,
            syntax: true,
            words: true,
            marks: true,
            font: 13,
            current_match: 0,
            palette: 0,
        };
        let job = Arc::new(LayoutJob::default());
        for row in 0..MAX_ROWS + 10 {
            assert!(cache.get(style, row, Color32::WHITE).is_none());
            cache.insert(row, Color32::WHITE, job.clone());
        }
        assert_eq!(cache.rows.len(), MAX_ROWS);
        assert!(cache.get(style, 0, Color32::WHITE).is_none());
        assert!(cache.get(style, MAX_ROWS, Color32::RED).is_none());
        assert!(cache.get(style, MAX_ROWS, Color32::WHITE).is_some());
        assert!(
            cache
                .get(
                    Style {
                        current_match: 1,
                        ..style
                    },
                    MAX_ROWS,
                    Color32::WHITE
                )
                .is_none()
        );
        assert!(cache.rows.is_empty());
        cache.insert(MAX_ROWS, Color32::WHITE, job.clone());
        assert!(
            cache.get(Style { palette: 1, ..style }, MAX_ROWS, Color32::WHITE).is_none(),
            "a new theme palette rebuilds the rows"
        );
        let huge = Arc::new(LayoutJob {
            text: "x".repeat(MAX_BYTES + 1),
            ..Default::default()
        });
        cache.insert(0, Color32::WHITE, huge);
        assert!(cache.rows.is_empty());
        assert_eq!(cache.bytes, 0);
    }
}
