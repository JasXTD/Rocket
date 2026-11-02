use super::*;

fn used(row: &[Cell]) -> usize {
    row.iter()
        .rposition(|c| c.width == 0 || c.ch != ' ' || !c.suffix.is_empty() || c.background != 0)
        .map_or(0, |i| i + 1)
}

impl Screen {
    /// Rebuild logical lines, retaining hard newlines and complete graphemes.
    /// ConPTY passes VT through and asks us for the resulting cursor position.
    pub(super) fn reflow(&mut self, columns: usize, rows: usize) {
        if (columns, rows) == (self.columns, self.rows) {
            return;
        }
        let history = self.scrollback.len();
        let cursor_row = history + self.cursor.1;
        let cursor_col = if self.wrap {
            self.columns
        } else {
            self.cursor.0
        };
        let last = self
            .cells
            .iter()
            .rposition(|r| used(r) != 0)
            .unwrap_or(0)
            .max(self.cursor.1);
        let mut output = Vec::new();
        let mut wraps = Vec::new();
        let mut logical = Vec::new();
        let mut position = None;
        let mut cursor = (0, 0);
        let mut pending_wrap = false;
        for (y, (row, wrap)) in self
            .scrollback
            .iter()
            .chain(self.cells.iter().take(last + 1))
            .zip(self.history_wrapped.iter().chain(self.wrapped.iter()))
            .enumerate()
        {
            let mut end = wrap.unwrap_or_else(|| used(row));
            if y == cursor_row {
                end = end.max(cursor_col);
                position = Some(logical.len() + cursor_col);
            }
            logical.extend_from_slice(&row[..end.min(row.len())]);
            if wrap.is_some() && y < history + last {
                continue;
            }
            let mut line = Vec::new();
            let mut offset = 0;
            while offset < logical.len() {
                let cell = &logical[offset];
                let width = usize::from(cell.width).max(1);
                if line.len() + width > columns {
                    wraps.push(Some(line.len()));
                    line.resize(columns, Cell::default());
                    output.push(Row::from(std::mem::take(&mut line)));
                }
                if position.is_some_and(|p| (offset..offset + width).contains(&p)) {
                    cursor = (line.len(), output.len());
                    pending_wrap = false;
                }
                line.extend_from_slice(&logical[offset..(offset + width).min(logical.len())]);
                offset += width;
            }
            if position == Some(logical.len()) {
                cursor = (line.len().min(columns - 1), output.len());
                pending_wrap = line.len() == columns;
            }
            line.resize(columns, Cell::default());
            output.push(Row::from(line));
            wraps.push(None);
            logical.clear();
            position = None;
        }
        // Keep the cursor visible when an application has output below it.
        let top = output.len().saturating_sub(rows).min(cursor.1);
        let keep = top.saturating_sub(1000);
        self.scrollback = output[keep..top].iter().cloned().collect();
        self.history_wrapped = wraps[keep..top].iter().copied().collect();
        self.cells = output.into_iter().skip(top).take(rows).collect();
        self.cells.resize(rows, Row::blank(columns));
        self.wrapped = wraps.into_iter().skip(top).take(rows).collect();
        self.wrapped.resize(rows, None);
        self.cursor = (cursor.0, cursor.1 - top);
        self.saved = (self.saved.0.min(columns - 1), self.saved.1.min(rows - 1));
        self.wrap = pending_wrap;
        self.columns = columns;
        self.rows = rows;
    }
}
