//! Bottom-pane composer — a deliberately small multi-line editor.
//!
//! One struct over `Vec<String>` lines with a (row, char-idx) cursor.
//! Enter submits, Ctrl+J / Alt+Enter inserts a newline, ↑/↓ walk history
//! at the edges. Bracketed paste lands via `insert_text`.

use unicode_width::UnicodeWidthChar;

pub struct Composer {
    lines: Vec<String>,
    /// cursor position: (row, char index within the row)
    cursor: (usize, usize),
    history: Vec<String>,
    /// `Some(i)` while cycling history — restores `stash` on edit.
    hist_idx: Option<usize>,
    stash: String,
}

impl Composer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            history: Vec::new(),
            hist_idx: None,
            stash: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.trim().is_empty())
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Take the buffer, push it into history, reset. Returns the text.
    pub fn take_submit(&mut self) -> String {
        let text = self.text();
        let trimmed = text.trim();
        if !trimmed.is_empty() && self.history.last().map(|h| h != trimmed).unwrap_or(true) {
            self.history.push(trimmed.to_string());
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.lines = vec![String::new()];
        self.cursor = (0, 0);
        self.hist_idx = None;
        text
    }

    fn cur_char_count(&self) -> usize {
        self.lines[self.cursor.0].chars().count()
    }

    fn byte_idx(&self, row: usize, char_idx: usize) -> usize {
        self.lines[row]
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.lines[row].len())
    }

    pub fn insert_char(&mut self, c: char) {
        self.hist_idx = None;
        if c == '\n' {
            self.newline();
            return;
        }
        let b = self.byte_idx(self.cursor.0, self.cursor.1);
        self.lines[self.cursor.0].insert(b, c);
        self.cursor.1 += 1;
    }

    /// Pasted/dropped text: \r\n → \n, splits into lines.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.hist_idx = None;
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let parts: Vec<&str> = text.split('\n').collect();
        let (row, col) = self.cursor;
        let b = self.byte_idx(row, col);
        let tail = self.lines[row].split_off(b);
        self.lines[row].push_str(parts[0]);
        self.lines
            .splice(row + 1..row + 1, parts[1..].iter().map(|p| p.to_string()));
        let last = row + parts.len() - 1;
        self.cursor = (
            last,
            parts.last().unwrap().chars().count() + if last == row { col } else { 0 },
        );
        self.lines[last].push_str(&tail);
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(String::new());
        self.cursor = (0, 0);
        self.hist_idx = None;
    }

    pub fn newline(&mut self) {
        self.hist_idx = None;
        let b = self.byte_idx(self.cursor.0, self.cursor.1);
        let tail = self.lines[self.cursor.0].split_off(b);
        self.lines.insert(self.cursor.0 + 1, tail);
        self.cursor = (self.cursor.0 + 1, 0);
    }

    pub fn backspace(&mut self) {
        self.hist_idx = None;
        if self.cursor.1 > 0 {
            let b = self.byte_idx(self.cursor.0, self.cursor.1 - 1);
            let e = self.byte_idx(self.cursor.0, self.cursor.1);
            self.lines[self.cursor.0].replace_range(b..e, "");
            self.cursor.1 -= 1;
        } else if self.cursor.0 > 0 {
            let moved = self.lines.remove(self.cursor.0);
            self.cursor.0 -= 1;
            self.cursor.1 = self.cur_char_count();
            self.lines[self.cursor.0].push_str(&moved);
        }
    }

    pub fn delete(&mut self) {
        self.hist_idx = None;
        if self.cursor.1 < self.cur_char_count() {
            let b = self.byte_idx(self.cursor.0, self.cursor.1);
            let e = self.byte_idx(self.cursor.0, self.cursor.1 + 1);
            self.lines[self.cursor.0].replace_range(b..e, "");
        } else if self.cursor.0 + 1 < self.lines.len() {
            let moved = self.lines.remove(self.cursor.0 + 1);
            self.lines[self.cursor.0].push_str(&moved);
        }
    }

    /// Ctrl+W — delete word backwards (whitespace-run then word-run).
    pub fn delete_word_back(&mut self) {
        if self.cursor.1 == 0 && self.cursor.0 > 0 {
            self.backspace();
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
        let mut i = self.cursor.1;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let b = self.byte_idx(self.cursor.0, i);
        let e = self.byte_idx(self.cursor.0, self.cursor.1);
        self.lines[self.cursor.0].replace_range(b..e, "");
        self.cursor.1 = i;
    }

    pub fn move_left(&mut self) {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.cur_char_count();
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor.1 < self.cur_char_count() {
            self.cursor.1 += 1;
        } else if self.cursor.0 + 1 < self.lines.len() {
            self.cursor = (self.cursor.0 + 1, 0);
        }
    }

    /// Up: previous line, or (at row 0) walk history back.
    pub fn move_up(&mut self) {
        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.cursor.1.min(self.cur_char_count());
            return;
        }
        if self.history.is_empty() {
            return;
        }
        match self.hist_idx {
            None => {
                self.stash = self.text();
                self.hist_idx = Some(self.history.len() - 1);
            }
            Some(i) if i > 0 => self.hist_idx = Some(i - 1),
            _ => return,
        }
        self.set_text(&self.history[self.hist_idx.unwrap()].clone());
    }

    /// Down: next line, or (at last row) walk history forward.
    pub fn move_down(&mut self) {
        if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.cursor.1 = self.cursor.1.min(self.cur_char_count());
            return;
        }
        match self.hist_idx {
            Some(i) if i + 1 < self.history.len() => {
                self.hist_idx = Some(i + 1);
                self.set_text(&self.history[i + 1].clone());
            }
            Some(_) => {
                self.hist_idx = None;
                let stash = self.stash.clone();
                self.set_text(&stash);
            }
            None => {}
        }
    }

    /// Replace the whole buffer; the cursor lands at the end of the last
    /// line — the natural spot after an autocomplete fill.
    pub fn set_text(&mut self, s: &str) {
        self.lines = s.split('\n').map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let r = self.lines.len() - 1;
        self.cursor = (r, self.lines[r].chars().count());
    }

    pub fn home(&mut self) {
        self.cursor.1 = 0;
    }

    pub fn end(&mut self) {
        self.cursor.1 = self.cur_char_count();
    }

    /// Hard-wrap the input and map its character cursor using the same
    /// cell widths. Reserve an empty row when the cursor is at a wrap edge.
    pub fn wrapped(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut rows = vec![String::new()];
        let mut col = 0;
        let mut cursor = (0, 0);
        for (r, line) in self.lines.iter().enumerate() {
            if r > 0 {
                rows.push(String::new());
                col = 0;
            }
            for (i, ch) in line.chars().enumerate() {
                let w = UnicodeWidthChar::width(ch).unwrap_or(0);
                if col + w > width {
                    rows.push(String::new());
                    col = 0;
                }
                if self.cursor == (r, i) {
                    cursor = (rows.len() - 1, col);
                }
                // A wide glyph cannot fit in a one-cell viewport.
                if w <= width {
                    rows.last_mut().unwrap().push(ch);
                    col += w;
                }
            }
            if self.cursor == (r, line.chars().count()) {
                if col == width {
                    rows.push(String::new());
                    col = 0;
                }
                cursor = (rows.len() - 1, col);
            }
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_splices_multiline_unicode_at_the_cursor_without_losing_suffix() {
        let mut composer = Composer::new();
        composer.set_text("前后");
        composer.move_left();
        composer.insert_text("中\r\n文\r末\n");
        assert_eq!(composer.text(), "前中\n文\n末\n后");
        composer.insert_char('尾');
        assert_eq!(composer.text(), "前中\n文\n末\n尾后");
        composer.insert_text("");
        assert_eq!(composer.text(), "前中\n文\n末\n尾后");
    }

    #[test]
    fn large_single_line_paste_remains_editable_in_the_middle() {
        let mut composer = Composer::new();
        composer.set_text("ab");
        composer.move_left();
        let paste = "界".repeat(80_000);
        composer.insert_text(&paste);
        assert_eq!(composer.text(), format!("a{paste}b"));
        composer.backspace();
        composer.insert_char('尾');
        assert!(composer.text().ends_with("界尾b"));
    }

    #[test]
    fn wrapped_cursor_follows_cells_and_exact_wrap_edges() {
        let mut composer = Composer::new();
        composer.set_text("abc中");
        let (rows, cursor) = composer.wrapped(4);
        assert_eq!(rows, ["abc", "中"]);
        assert_eq!(cursor, (1, 2));
        composer.home();
        assert_eq!(composer.wrapped(4).1, (0, 0));
        composer.set_text("abcd");
        let (rows, cursor) = composer.wrapped(4);
        assert_eq!(rows, ["abcd", ""]);
        assert_eq!(cursor, (1, 0));
        composer.set_text("e\u{301}中");
        assert_eq!(composer.wrapped(3).1, (1, 0));
    }
}
