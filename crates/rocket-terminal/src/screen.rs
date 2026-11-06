//! Bounded VT screen model. OSC payloads (including hyperlinks and clipboard
//! commands) are ignored. Unknown control sequences cannot execute host actions.
use std::collections::VecDeque;
use std::{ops::{Deref, DerefMut}, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
mod reflow;

// A malicious stream of combining marks must not grow one cell without limit.
const MAX_CLUSTER_BYTES: usize = 128;

/// Screen snapshots share unchanged rows. A parser edit detaches only its row;
/// scrollback and all older snapshots keep their exact original contents.
#[derive(Clone, Debug, Default)]
pub struct Row(Arc<Vec<Cell>>);
impl Row {
    fn blank(columns: usize) -> Self { vec![Cell::default(); columns].into() }
}
impl From<Vec<Cell>> for Row {
    fn from(cells: Vec<Cell>) -> Self { Self(Arc::new(cells)) }
}
impl Deref for Row {
    type Target = Vec<Cell>;
    fn deref(&self) -> &Self::Target { &self.0 }
}
impl DerefMut for Row {
    fn deref_mut(&mut self) -> &mut Self::Target { Arc::make_mut(&mut self.0) }
}
impl<'a> IntoIterator for &'a Row {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub ch: char,
    pub suffix: String,
    /// Display columns occupied by the cluster; zero marks a continuation cell.
    pub width: u8,
    pub foreground: u8,
    pub background: u8,
}
impl Cell {
    pub fn text(&self) -> String {
        if self.width == 0 {
            return String::new();
        }
        let mut text = String::with_capacity(self.ch.len_utf8() + self.suffix.len());
        text.push(self.ch);
        text.push_str(&self.suffix);
        text
    }
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            suffix: String::new(),
            width: 1,
            foreground: 7,
            background: 0,
        }
    }
}
#[derive(Clone, Debug)]
enum Parse {
    Text,
    Escape,
    Csi(String),
    Osc(bool),
    Charset,
}
#[derive(Clone, Debug)]
pub struct Screen {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<Row>,
    pub scrollback: VecDeque<Row>,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub revision: u64,
    saved: (usize, usize),
    region: (usize, usize),
    style: Cell,
    parse: Parse,
    utf8: Vec<u8>,
    wrap: bool,
    previous: Option<(usize, usize)>,
    // Soft wraps carry their used width (a wide glyph may wrap before the edge).
    wrapped: Vec<Option<usize>>,
    history_wrapped: VecDeque<Option<usize>>,
    replies: Vec<u8>,
}
impl Screen {
    pub fn new(columns: u16, rows: u16) -> Self {
        let columns = columns.clamp(20, 240);
        let rows = rows.clamp(4, 100);
        let (columns, rows) = (columns as usize, rows as usize);
        Self {
            columns,
            rows,
            cells: vec![Row::blank(columns); rows],
            scrollback: VecDeque::new(),
            cursor: (0, 0),
            cursor_visible: true,
            revision: 1,
            saved: (0, 0),
            region: (0, rows - 1),
            style: Cell::default(),
            parse: Parse::Text,
            utf8: Vec::new(),
            wrap: false,
            previous: None,
            wrapped: vec![None; rows],
            history_wrapped: VecDeque::new(),
            replies: Vec::new(),
        }
    }
    pub fn resize(&mut self, columns: u16, rows: u16) {
        self.reflow(columns.clamp(20, 240) as usize, rows.clamp(4, 100) as usize);
        self.region = (0, self.rows - 1);
        self.previous = None;
        self.revision += 1;
    }
    #[cfg(any(windows, test))]
    pub(crate) fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.history_wrapped.clear();
        self.revision += 1;
    }
    pub fn text(&self) -> String {
        self.scrollback
            .iter()
            .chain(&self.cells)
            .map(|r| {
                r.iter()
                    .filter(|c| c.width != 0)
                    .map(Cell::text)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.utf8.extend_from_slice(bytes);
        loop {
            let (valid, skip) = match std::str::from_utf8(&self.utf8) {
                Ok(_) => (self.utf8.len(), 0),
                Err(e) => (e.valid_up_to(), e.error_len().unwrap_or(0)),
            };
            let text = String::from_utf8_lossy(&self.utf8[..valid]).into_owned();
            for ch in text.chars() {
                self.character(ch);
            }
            self.utf8.drain(..valid + skip);
            if skip > 0 {
                self.character('\u{fffd}');
            } else {
                break;
            }
        }
        self.revision += 1;
    }
    fn character(&mut self, ch: char) {
        match std::mem::replace(&mut self.parse, Parse::Text) {
            Parse::Osc(escape) => {
                if ch != '\u{7}' && !(escape && ch == '\\') {
                    self.parse = Parse::Osc(ch == '\u{1b}');
                }
            }
            Parse::Charset => {}
            Parse::Escape => match ch {
                '[' => self.parse = Parse::Csi(String::new()),
                ']' => self.parse = Parse::Osc(false),
                '(' | ')' => self.parse = Parse::Charset,
                '7' => self.saved = self.cursor,
                '8' => {
                    self.previous = None;
                    self.wrap = false;
                    self.cursor = (
                        self.saved.0.min(self.columns - 1),
                        self.saved.1.min(self.rows - 1),
                    )
                }
                'D' => self.linefeed(),
                'E' => {
                    self.cursor.0 = 0;
                    self.linefeed();
                }
                'M' => {
                    self.previous = None;
                    self.wrap = false;
                    if self.cursor.1 > self.region.0 {
                        self.cursor.1 -= 1;
                    } else {
                        self.scroll_down();
                    }
                }
                'c' => {
                    let (cols, rows, revision) =
                        (self.columns as u16, self.rows as u16, self.revision);
                    *self = Self::new(cols, rows);
                    self.revision = revision;
                }
                _ => {}
            },
            Parse::Csi(mut sequence) => {
                if ('@'..='~').contains(&ch) {
                    self.csi(&sequence, ch);
                } else if sequence.len() < 128 {
                    sequence.push(ch);
                    self.parse = Parse::Csi(sequence);
                }
            }
            Parse::Text => match ch {
                '\u{1b}' => self.parse = Parse::Escape,
                '\r' => {
                    self.previous = None;
                    self.cursor.0 = 0;
                    self.wrap = false;
                }
                '\n' => self.linefeed(),
                '\u{8}' => {
                    self.previous = None;
                    self.cursor.0 = self.cursor.0.saturating_sub(1);
                    self.wrap = false;
                }
                '\t' => {
                    self.previous = None;
                    self.cursor.0 = ((self.cursor.0 / 8 + 1) * 8).min(self.columns - 1);
                    self.wrap = false;
                }
                ch if !ch.is_control() => self.print(ch),
                _ => {}
            },
        }
    }
    fn print(&mut self, ch: char) {
        if let Some((x, y)) = self.previous {
            let previous = &self.cells[y][x];
            // Most shell output is ASCII: avoid segmentation and allocations
            // for a pair which cannot form a multi-scalar grapheme.
            if !(ch.is_ascii() && previous.ch.is_ascii() && previous.suffix.is_empty()) {
                let mut text = previous.text();
                text.push(ch);
                if text.graphemes(true).nth(1).is_none() {
                    if text.len() > MAX_CLUSTER_BYTES {
                        return;
                    }
                    let mut cell = previous.clone();
                    cell.suffix.push(ch);
                    cell.width = text.width().max(1).min(self.columns) as u8;
                    let old_width = usize::from(previous.width);
                    clear_range(&mut self.cells[y], x, x + old_width);
                    // A late variation selector can make a one-column cluster
                    // wide. Reposition the complete cluster when it hits a margin.
                    self.cursor = (x, y);
                    self.wrap = false;
                    self.put(cell);
                    return;
                }
            }
        }
        let text = ch.to_string();
        self.put(Cell {
            ch,
            suffix: String::new(),
            width: text.width().max(1).min(self.columns) as u8,
            foreground: self.style.foreground,
            background: self.style.background,
        });
    }
    fn put(&mut self, cell: Cell) {
        let width = usize::from(cell.width);
        if self.wrap || self.cursor.0 + width > self.columns {
            self.wrapped[self.cursor.1] = Some(if self.wrap {
                self.columns
            } else {
                self.cursor.0
            });
            self.cursor.0 = 0;
            self.advance_line();
        }
        let (x, y) = self.cursor;
        clear_range(&mut self.cells[y], x, x + width);
        for offset in 1..width {
            self.cells[y][x + offset] = Cell {
                width: 0,
                foreground: cell.foreground,
                background: cell.background,
                ..Cell::default()
            };
        }
        self.cells[y][x] = cell;
        self.previous = Some((x, y));
        self.wrap = x + width == self.columns;
        self.cursor.0 = (x + width).min(self.columns - 1);
    }
    fn linefeed(&mut self) {
        self.wrapped[self.cursor.1] = None;
        self.advance_line();
    }
    fn advance_line(&mut self) {
        self.previous = None;
        self.wrap = false;
        if self.cursor.1 == self.region.1 {
            self.scroll_up();
        } else {
            self.cursor.1 = (self.cursor.1 + 1).min(self.rows - 1);
        }
    }
    fn scroll_up(&mut self) {
        let row = self.cells.remove(self.region.0);
        let wrapped = self.wrapped.remove(self.region.0);
        if self.region == (0, self.rows - 1) {
            self.scrollback.push_back(row);
            self.history_wrapped.push_back(wrapped);
            if self.scrollback.len() > 1000 {
                self.scrollback.pop_front();
                self.history_wrapped.pop_front();
            }
        }
        self.cells
            .insert(self.region.1, Row::blank(self.columns));
        self.wrapped.insert(self.region.1, None);
    }
    fn scroll_down(&mut self) {
        self.cells.remove(self.region.1);
        self.wrapped.remove(self.region.1);
        self.cells
            .insert(self.region.0, Row::blank(self.columns));
        self.wrapped.insert(self.region.0, None);
    }
    fn csi(&mut self, sequence: &str, command: char) {
        let private = sequence.starts_with('?');
        let values: Vec<usize> = sequence
            .trim_start_matches('?')
            .split(';')
            .take(32)
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        let first = values.first().copied().unwrap_or(0);
        let n = first.clamp(1, 1000);
        // SGR and cursor visibility do not move the cursor or break an ongoing
        // grapheme. In particular, SGR must preserve delayed right-margin wrap.
        if !matches!(command, 'm' | 'n' | 'c') && !(private && first == 25) {
            self.wrap = false;
            self.previous = None;
        }
        if private {
            if command == 'h' || command == 'l' {
                if first == 25 {
                    self.cursor_visible = command == 'h';
                } else if first == 1049 {
                    for row in &mut self.cells {
                        row.fill(Cell::default());
                    }
                    self.cursor = (0, 0);
                    self.wrapped.fill(None);
                }
            }
            return;
        }
        match command {
            'n' if sequence == "6" && self.replies.len() < 4000 => {
                self.replies.extend_from_slice(
                    format!("\x1b[{};{}R", self.cursor.1 + 1, self.cursor.0 + 1).as_bytes(),
                );
            }
            'A' => self.cursor.1 = self.cursor.1.saturating_sub(n),
            'B' | 'e' => self.cursor.1 = (self.cursor.1 + n).min(self.rows - 1),
            'C' | 'a' => self.cursor.0 = (self.cursor.0 + n).min(self.columns - 1),
            'D' => self.cursor.0 = self.cursor.0.saturating_sub(n),
            'E' => {
                self.cursor.1 = (self.cursor.1 + n).min(self.rows - 1);
                self.cursor.0 = 0;
            }
            'F' => {
                self.cursor.1 = self.cursor.1.saturating_sub(n);
                self.cursor.0 = 0;
            }
            'G' | '`' => self.cursor.0 = n.saturating_sub(1).min(self.columns - 1),
            'd' => self.cursor.1 = n.saturating_sub(1).min(self.rows - 1),
            'H' | 'f' => {
                self.cursor = (
                    values
                        .get(1)
                        .copied()
                        .unwrap_or(1)
                        .max(1)
                        .saturating_sub(1)
                        .min(self.columns - 1),
                    n.saturating_sub(1).min(self.rows - 1),
                );
            }
            's' => self.saved = self.cursor,
            'u' => {
                self.cursor = (
                    self.saved.0.min(self.columns - 1),
                    self.saved.1.min(self.rows - 1),
                );
            }
            'J' => {
                for y in 0..self.rows {
                    if first >= 2
                        || (first == 0 && y >= self.cursor.1)
                        || (first == 1 && y < self.cursor.1)
                    {
                        self.wrapped[y] = None;
                    }
                    for x in 0..self.columns {
                        if first >= 2
                            || (first == 0
                                && (y > self.cursor.1
                                    || (y == self.cursor.1 && x >= self.cursor.0)))
                            || (first == 1
                                && (y < self.cursor.1
                                    || (y == self.cursor.1 && x <= self.cursor.0)))
                        {
                            clear_range(&mut self.cells[y], x, x + 1);
                        }
                    }
                }
                if first == 3 {
                    self.scrollback.clear();
                    self.history_wrapped.clear();
                }
            }
            'K' => {
                if first != 1 {
                    self.wrapped[self.cursor.1] = None;
                }
                for x in 0..self.columns {
                    if first == 2
                        || (first == 0 && x >= self.cursor.0)
                        || (first == 1 && x <= self.cursor.0)
                    {
                        clear_range(&mut self.cells[self.cursor.1], x, x + 1);
                    }
                }
            }
            'X' => {
                let end = (self.cursor.0 + n).min(self.columns);
                clear_range(&mut self.cells[self.cursor.1], self.cursor.0, end);
            }
            'P' => {
                let row = &mut self.cells[self.cursor.1];
                let n = n.min(self.columns - self.cursor.0);
                clear_range(row, self.cursor.0, self.cursor.0 + n);
                row.drain(self.cursor.0..self.cursor.0 + n);
                row.resize(self.columns, Cell::default());
                repair_row(row);
            }
            '@' => {
                let row = &mut self.cells[self.cursor.1];
                if row[self.cursor.0].width == 0 {
                    clear_range(row, self.cursor.0, self.cursor.0 + 1);
                }
                for _ in 0..n.min(self.columns - self.cursor.0) {
                    row.insert(self.cursor.0, Cell::default());
                    row.pop();
                }
                repair_row(row);
            }
            'L' => {
                if self.cursor.1 >= self.region.0 && self.cursor.1 <= self.region.1 {
                    for _ in 0..n.min(self.rows) {
                        self.cells.remove(self.region.1);
                        self.wrapped.remove(self.region.1);
                        self.cells
                            .insert(self.cursor.1, Row::blank(self.columns));
                        self.wrapped.insert(self.cursor.1, None);
                    }
                }
            }
            'M' => {
                if self.cursor.1 >= self.region.0 && self.cursor.1 <= self.region.1 {
                    for _ in 0..n.min(self.rows) {
                        self.cells.remove(self.cursor.1);
                        self.wrapped.remove(self.cursor.1);
                        self.cells
                            .insert(self.region.1, Row::blank(self.columns));
                        self.wrapped.insert(self.region.1, None);
                    }
                }
            }
            'S' => {
                for _ in 0..n.min(self.rows) {
                    self.scroll_up();
                }
            }
            'T' => {
                for _ in 0..n.min(self.rows) {
                    self.scroll_down();
                }
            }
            'r' => {
                let top = first.max(1) - 1;
                let bottom = values.get(1).copied().unwrap_or(self.rows).max(1) - 1;
                if top < bottom && bottom < self.rows {
                    self.region = (top, bottom);
                    self.cursor = (0, 0);
                }
            }
            'm' => {
                let mut i = 0;
                while i < values.len() {
                    match values[i] {
                        0 => self.style = Cell::default(),
                        1 => self.style.foreground |= 8,
                        22 => self.style.foreground &= 7,
                        30..=37 => self.style.foreground = (values[i] - 30) as u8,
                        39 => self.style.foreground = 7,
                        40..=47 => self.style.background = (values[i] - 40) as u8,
                        49 => self.style.background = 0,
                        90..=97 => self.style.foreground = (values[i] - 90 + 8) as u8,
                        100..=107 => self.style.background = (values[i] - 100 + 8) as u8,
                        38 | 48 => {
                            i += if values.get(i + 1) == Some(&2) { 4 } else { 2 };
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
    }
}

// Erasing or overwriting either half of a wide cluster erases the whole cluster.
fn clear_range(row: &mut [Cell], start: usize, end: usize) {
    if start >= end || start >= row.len() {
        return;
    }
    let mut left = start;
    while left > 0 && row[left].width == 0 {
        left -= 1;
    }
    let mut right = end.min(row.len());
    let mut last = right - 1;
    while last > 0 && row[last].width == 0 {
        last -= 1;
    }
    right = right
        .max(last + usize::from(row[last].width))
        .min(row.len());
    row[left..right].fill(Cell::default());
}

// Column edits and shrinking must never leave a partial cluster at an edge.
fn repair_row(row: &mut [Cell]) {
    let mut x = 0;
    while x < row.len() {
        let width = usize::from(row[x].width);
        if width == 0
            || x + width > row.len()
            || row[x + 1..x + width].iter().any(|cell| cell.width != 0)
        {
            row[x] = Cell::default();
            x += 1;
        } else {
            x += width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_share_unchanged_rows_and_keep_text_after_edits_resize_and_clear() {
        let mut live = Screen::new(40, 4);
        live.feed("é\r\ntwo\r\nthree\r\nfour\r\nfive".as_bytes());
        let snapshot = live.clone();
        let text = snapshot.text();
        assert!(Arc::ptr_eq(&live.scrollback[0].0, &snapshot.scrollback[0].0));
        assert!(Arc::ptr_eq(&live.cells[3].0, &snapshot.cells[3].0));
        live.feed(b"\rchanged");
        assert!(!Arc::ptr_eq(&live.cells[3].0, &snapshot.cells[3].0));
        assert!(Arc::ptr_eq(&live.cells[0].0, &snapshot.cells[0].0));
        live.resize(20, 6);
        live.clear_scrollback();
        assert_eq!(snapshot.text(), text);
        assert!(live.text().contains("changed"));
    }
    #[test]
    fn resize_reflows_wrapped_unicode_preserves_hard_lines_and_reports_cursor() {
        let mut s = Screen::new(60, 8);
        let path = "PS C:\\long-path-with-中文-and-🚀-and-é-end";
        s.feed(format!("{path}\r\n> Write-Output 'pending'").as_bytes());
        for (columns, rows) in [(20, 4), (94, 8), (30, 4), (60, 8)] {
            s.resize(columns, rows);
            assert_grid(&s);
            let text: String = s.text().lines().collect();
            assert_eq!(text, format!("{path}> Write-Output 'pending'"));
            let cursor = s.cursor;
            s.feed(b"\x1b[");
            s.feed(b"6n");
            assert_eq!(
                s.take_replies(),
                format!("\x1b[{};{}R", cursor.1 + 1, cursor.0 + 1).into_bytes()
            );
            assert_eq!(s.cursor, cursor);
        }
        assert_eq!(s.text().lines().filter(|l| !l.is_empty()).count(), 2);
        s.feed(b"\x1b]52;c;untrusted\x07\x1b[?6n");
        assert!(s.take_replies().is_empty());
    }
    #[test]
    fn unicode_advances_by_display_columns_across_utf8_chunks() {
        for (text, columns) in [
            ("中文", 4),
            ("日本語", 6),
            ("한국어", 6),
            ("𠮷", 2),
            ("🚀", 2),
            ("e\u{301}", 1),
            ("👩🏽‍💻", 2),
            ("👨‍👩‍👧‍👦", 2),
            ("🇲🇾", 2),
            ("1\u{fe0f}\u{20e3}", 2),
            ("❤\u{fe0f}", 2),
            ("☺\u{fe0e}", 1),
            ("\u{1100}\u{1161}", 2),
            ("é", 1),
        ] {
            let mut screen = Screen::new(20, 4);
            for byte in text.as_bytes() {
                screen.feed(&[*byte]);
            }
            assert_eq!(screen.cursor, (columns, 0), "{text}");
            assert_eq!(screen.text().trim_end(), text);
            assert_grid(&screen);
        }
    }
    fn assert_grid(screen: &Screen) {
        for row in screen.scrollback.iter().chain(&screen.cells) {
            let mut x = 0;
            while x < row.len() {
                let width = usize::from(row[x].width);
                assert!(
                    width > 0 && x + width <= row.len(),
                    "broken cell {x}: {row:?}"
                );
                assert!(
                    row[x + 1..x + width]
                        .iter()
                        .all(|c| c.width == 0 && c.suffix.is_empty())
                );
                assert!(row[x].text().len() <= MAX_CLUSTER_BYTES);
                x += width;
            }
        }
    }
    #[test]
    fn delayed_wrap_and_late_width_changes_keep_complete_clusters() {
        let mut s = Screen::new(20, 4);
        s.feed(b"1234567890123456789");
        s.feed("中".as_bytes());
        assert_eq!(s.cursor, (2, 1));
        assert_eq!(s.cells[1][0].text(), "中");
        s.feed(b"\x1b[1;20H");
        s.feed("❤".as_bytes());
        s.feed(b"\x1b[31m");
        s.feed("\u{fe0f}".as_bytes());
        assert_eq!(s.cells[0][19].text(), " ");
        assert_eq!(s.cells[1][0].text(), "❤️");
        assert_eq!(s.cursor, (2, 1));
        s.feed(b"\x1b[3;20He\x1b[32m");
        s.feed("\u{301}".as_bytes());
        assert_eq!(s.cells[2][19].text(), "é");
        assert_eq!(s.cursor, (19, 2));
        s.feed(b"!");
        assert_eq!(s.cursor, (1, 3));
        assert_eq!(s.cells[3][0].ch, '!');
        assert_grid(&s);
    }
    #[test]
    fn overwrite_erase_insert_delete_and_resize_cannot_split_wide_cells() {
        for command in [
            "X", "\x1b[X", "\x1b[K", "\x1b[1K", "\x1b[J", "\x1b[1J", "\x1b[P", "\x1b[@",
        ] {
            let mut s = Screen::new(20, 4);
            s.feed("A中B文C".as_bytes());
            s.feed(b"\x1b[3G"); // second half of 中
            s.feed(command.as_bytes());
            assert!(!s.text().contains('中'), "{command:?}");
            assert_grid(&s);
        }
        let mut s = Screen::new(22, 4);
        s.feed(b"\x1b[20G");
        s.feed("中".as_bytes());
        s.resize(20, 4);
        assert!(s.text().contains('中')); // Resize reflows instead of truncating the glyph.
        assert_grid(&s);
    }
    #[test]
    fn cursor_backspace_tabs_and_style_use_columns_not_scalars() {
        let mut s = Screen::new(20, 4);
        s.feed("中\t文".as_bytes());
        assert_eq!(s.cursor, (10, 0));
        s.feed(b"\x08X");
        assert_eq!(s.cursor, (10, 0));
        assert_eq!(s.cells[0][8].ch, ' ');
        assert_eq!(s.cells[0][9].ch, 'X');
        s.feed(b"\r\x1b[44m");
        s.feed("🚀".as_bytes());
        assert_eq!(s.cells[0][0].background, 4);
        assert_eq!(s.cells[0][1].background, 4);
        assert_grid(&s);
    }
    #[test]
    fn streaming_chunks_scrolling_and_long_combining_sequences_remain_bounded() {
        let text = "中é👩🏽‍💻🇲🇾1️⃣\x1b[31m文\x1b[0m\r\n";
        for chunk_size in 1..=text.len() {
            let mut s = Screen::new(20, 4);
            for _ in 0..8 {
                for chunk in text.as_bytes().chunks(chunk_size) {
                    s.feed(chunk);
                    assert_grid(&s);
                }
            }
            assert!(
                s.scrollback
                    .iter()
                    .any(|r| r.iter().any(|c| c.text() == "👩🏽‍💻"))
            );
        }
        let mut s = Screen::new(20, 4);
        s.feed(format!("e{}Z", "\u{301}".repeat(10_000)).as_bytes());
        assert_eq!(s.cursor, (2, 0));
        assert_eq!(s.cells[0][1].ch, 'Z');
        assert_grid(&s);
    }
    #[test]
    fn cursor_edits_unicode_chunks_and_hostile_sequences_are_bounded() {
        let mut s = Screen::new(40, 4);
        s.feed(b"wrong\r\x1b[2Kright ");
        let text = "é".as_bytes();
        s.feed(&text[..1]);
        s.feed(&text[1..]);
        assert!(s.text().starts_with("right é"));
        s.feed(b"\x1b]52;c;private clipboard payload\x07\x1b[2;1Hsecond");
        assert!(!s.text().contains("private"));
        assert!(s.text().contains("second"));
        s.feed(b"\x1b[9999999999999999999999999;9999999999999999Hsafe");
        for _ in 0..2000 {
            s.feed(b"line\r\n");
        }
        assert_eq!(s.scrollback.len(), 1000);
        assert_eq!(s.cells.len(), 4);
        s.resize(20, 6);
        assert_eq!(s.cells.len(), 6);
        assert!(s.cells.iter().all(|r| r.len() == 20));
    }
}
