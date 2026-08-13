//! The multi-line input buffer.
//!
//! Pure state: text, a cursor, and the operations a key binding maps onto. It knows nothing
//! about crossterm or ratatui, which is what makes the motion and deletion rules testable.
//!
//! Positions are byte offsets that always sit on a grapheme boundary, so a cursor never
//! lands inside an emoji or a combining sequence. Vertical motion works on *visual* rows —
//! a wrapped line is several rows, and pressing up inside one moves within it.

mod kill_ring;
mod paste;
mod undo;

pub use kill_ring::KillRing;
pub use paste::PasteStore;
pub use kill_ring::LastAction;
pub use undo::Snapshot;
pub use undo::UndoStack;

use crate::wrap::grapheme_width;
use crate::wrap::text_width;
use crate::wrap::wrap_ranges;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// One display row: a slice of one logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRow {
    pub line: usize,
    pub range: Range<usize>,
}

/// The buffer laid out for a given width, with the cursor placed on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorLayout {
    pub rows: Vec<VisualRow>,
    pub cursor_row: usize,
    pub cursor_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    lines: Vec<String>,
    row: usize,
    col: usize,
    /// Display column vertical motion returns to after passing through a short row.
    sticky: Option<usize>,
    /// What the cut commands cut, and what yank puts back.
    kill: KillRing,
    /// Snapshots taken before each edit, coalesced a word at a time.
    undo: UndoStack,
    /// What the last keystroke did, which decides whether a kill merges with the one before
    /// it, whether yank-pop is allowed, and whether typing opens a new undo unit.
    last: LastAction,
    /// Large pastes held aside, stood in for by a marker in the text.
    pastes: PasteStore,
    /// Prompts already submitted, oldest first, browsed with up at the top of the buffer.
    history: Vec<String>,
    /// Where in `history` the buffer currently sits, and what was being typed before
    /// browsing started so it can be given back.
    browsing: Option<Browsing>,
}

/// A walk back through submitted prompts, and the draft it interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Browsing {
    index: usize,
    draft: Vec<String>,
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            sticky: None,
            kill: KillRing::new(),
            undo: UndoStack::new(),
            last: LastAction::Other,
            pastes: PasteStore::new(),
            history: Vec::new(),
            browsing: None,
        }
    }

    /// Take a snapshot of the buffer as it stands, for undo to return to.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            line: self.row,
            column: self.col,
        }
    }

    /// Record the buffer before an edit that is atomic on its own — a deletion, a newline, a
    /// paste, a yank. Typing goes through [`Editor::checkpoint_typing`] instead, which
    /// coalesces a run of word characters into one unit.
    fn checkpoint(&mut self) {
        let snapshot = self.snapshot();
        self.undo.push(snapshot);
    }

    /// Restore the buffer to before the last edit.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.lines = snapshot.lines;
        self.row = snapshot.line;
        self.col = snapshot.column;
        self.sticky = None;
        self.last = LastAction::Other;
        true
    }

    /// Insert the most recent kill at the cursor.
    pub fn yank(&mut self) {
        let Some(text) = self.kill.peek().map(str::to_string) else {
            return;
        };
        self.checkpoint();
        self.insert_inner(&text);
        self.last = LastAction::Yank;
    }

    /// Replace what yank just inserted with the kill before it.
    ///
    /// Only ever follows a yank: with anything else in between there is no way to know what
    /// to take back out, so the keystroke does nothing rather than guessing.
    pub fn yank_pop(&mut self) -> bool {
        if self.last != LastAction::Yank || self.kill.len() < 2 {
            return false;
        }
        let Some(inserted) = self.kill.peek().map(str::to_string) else {
            return false;
        };
        for _ in 0..inserted.graphemes(true).count() {
            self.backspace_inner();
        }
        let Some(text) = self.kill.rotate().map(str::to_string) else {
            return false;
        };
        self.insert_inner(&text);
        self.last = LastAction::Yank;
        true
    }

    /// Remember a submitted prompt, so up can bring it back.
    pub fn remember(&mut self, prompt: &str) {
        if prompt.trim().is_empty() {
            return;
        }
        // A prompt sent twice in a row is one entry: browsing past a repeat is noise.
        if self.history.last().map(String::as_str) != Some(prompt) {
            self.history.push(prompt.to_string());
        }
        self.browsing = None;
        self.undo.clear();
    }

    /// Step back through submitted prompts. Returns false when there is nothing older,
    /// which is what tells the caller to move the cursor instead.
    pub fn history_previous(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let index = match &self.browsing {
            Some(browsing) if browsing.index == 0 => return false,
            Some(browsing) => browsing.index - 1,
            None => self.history.len() - 1,
        };
        let draft = match self.browsing.take() {
            Some(browsing) => browsing.draft,
            None => self.lines.clone(),
        };
        self.browsing = Some(Browsing { index, draft });
        self.load(self.history[index].clone());
        true
    }

    /// Step forward again, ending on the draft that browsing interrupted.
    pub fn history_next(&mut self) -> bool {
        let Some(browsing) = self.browsing.take() else {
            return false;
        };
        match browsing.index + 1 < self.history.len() {
            true => {
                let index = browsing.index + 1;
                let text = self.history[index].clone();
                self.browsing = Some(Browsing {
                    index,
                    draft: browsing.draft,
                });
                self.load(text);
            }
            false => {
                self.lines = browsing.draft;
                self.cursor_to_end();
            }
        }
        true
    }

    /// Whether the buffer is showing a prompt from history rather than a draft.
    pub fn is_browsing_history(&self) -> bool {
        self.browsing.is_some()
    }

    fn load(&mut self, text: String) {
        self.lines = text.split('\n').map(str::to_string).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_to_end();
    }

    fn cursor_to_end(&mut self) {
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].len();
        self.sticky = None;
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The buffer as a single string, logical lines joined by newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }

    /// Byte position of the cursor as `(logical line, byte offset)`.
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = normalize(text).split('\n').map(str::to_string).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.move_end();
    }

    /// Empty the buffer and return what it held.
    /// Take the prompt to send: markers stand in on screen, but what leaves here is what
    /// they stand for.
    pub fn take(&mut self) -> String {
        let text = self.expanded_text();
        self.clear();
        text
    }

    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.row = 0;
        self.col = 0;
        self.sticky = None;
        // An emptied buffer is no longer showing anything from history, and the run that was
        // in progress ended with the text it applied to.
        self.browsing = None;
        self.last = LastAction::Other;
        // The pastes belonged to the prompt that just left; nothing still refers to them.
        self.pastes.clear();
    }

    pub fn insert_char(&mut self, character: char) {
        // A run of word characters is one undo unit; a space or a punctuation mark opens the
        // next one, so undo takes back a word together with the space that preceded it.
        if undo::opens_new_unit(character, self.last) {
            self.checkpoint();
        }
        self.last = match undo::is_word_character(character) {
            true => LastAction::TypeWord,
            false => LastAction::Other,
        };
        let mut buffer = [0u8; 4];
        self.insert_inner(character.encode_utf8(&mut buffer));
    }

    /// Insert text at the cursor. Newlines split the line, which is how a multi-line paste
    /// stays one block instead of being submitted a line at a time.
    /// Insert text as one atomic edit — a paste, or a completion being committed.
    ///
    /// It ends whatever run was in progress, which is what stops a kill that follows a
    /// paste from merging with the kill that came before it.
    pub fn insert_str(&mut self, text: &str) {
        self.checkpoint();
        self.last = LastAction::Other;
        self.insert_inner(text);
    }

    fn insert_inner(&mut self, text: &str) {
        let text = normalize(text);
        if text.is_empty() {
            return;
        }
        let tail = self.lines[self.row].split_off(self.col);
        let mut parts = text.split('\n');

        let first = parts.next().unwrap_or_default();
        self.lines[self.row].push_str(first);

        for part in parts {
            self.row += 1;
            self.lines.insert(self.row, part.to_string());
        }
        self.col = self.lines[self.row].len();
        self.lines[self.row].push_str(&tail);
        self.sticky = None;
    }

    pub fn insert_newline(&mut self) {
        self.insert_str("\n");
    }

    /// Replace the `bytes` immediately before the cursor with `text`.
    ///
    /// This is how a completion commits: what the user typed toward the command is the run
    /// of bytes behind the cursor, and the chosen command takes their place.
    pub fn replace_before_cursor(&mut self, bytes: usize, text: &str) {
        let start = self.col.saturating_sub(bytes);
        self.lines[self.row].replace_range(start..self.col, "");
        self.col = start;
        self.insert_str(text);
    }

    /// Delete one grapheme back. Plain deletion never touches the kill ring — only the
    /// word and line kills do — and it ends whatever run was in progress.
    pub fn backspace(&mut self) {
        self.checkpoint();
        self.last = LastAction::Other;
        // A marker stands for a whole paste, so it goes all at once rather than losing its
        // closing bracket and becoming ordinary text.
        if let Some(marker) = self.marker_before_cursor() {
            self.delete_marker(marker);
            return;
        }
        self.backspace_inner();
    }

    fn backspace_inner(&mut self) {
        if self.col > 0 {
            let start = prev_boundary(&self.lines[self.row], self.col);
            self.lines[self.row].replace_range(start..self.col, "");
            self.col = start;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].push_str(&line);
        }
        self.sticky = None;
    }

    /// Delete one grapheme forward. Like backspace, this never touches the kill ring and
    /// ends whatever run was in progress.
    pub fn delete(&mut self) {
        self.checkpoint();
        self.last = LastAction::Other;
        // A marker stands for a whole paste, so it goes all at once rather than losing its
        // opening bracket and becoming ordinary text.
        if let Some(marker) = self.marker_after_cursor() {
            self.delete_marker(marker);
            return;
        }
        self.delete_inner();
    }

    fn delete_inner(&mut self) {
        let length = self.lines[self.row].len();
        if self.col < length {
            let end = next_boundary(&self.lines[self.row], self.col);
            self.lines[self.row].replace_range(self.col..end, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
        self.sticky = None;
    }

    pub fn delete_word_before(&mut self) {
        if self.col == 0 {
            self.kill_newline_before();
            return;
        }
        self.checkpoint();
        let start = word_start_before(&self.lines[self.row], self.col);
        let killed = self.lines[self.row][start..self.col].to_string();
        self.lines[self.row].replace_range(start..self.col, "");
        self.col = start;
        self.sticky = None;
        self.record_kill(&killed, true);
    }

    pub fn delete_word_after(&mut self) {
        if self.col == self.lines[self.row].len() {
            self.kill_newline_after();
            return;
        }
        self.checkpoint();
        let end = word_end_after(&self.lines[self.row], self.col);
        let killed = self.lines[self.row][self.col..end].to_string();
        self.lines[self.row].replace_range(self.col..end, "");
        self.sticky = None;
        self.record_kill(&killed, false);
    }

    pub fn delete_to_line_start(&mut self) {
        self.checkpoint();
        let killed = self.lines[self.row][..self.col].to_string();
        self.lines[self.row].replace_range(0..self.col, "");
        self.col = 0;
        self.sticky = None;
        self.record_kill(&killed, true);
    }

    pub fn delete_to_line_end(&mut self) {
        self.checkpoint();
        let length = self.lines[self.row].len();
        if self.col < length {
            let killed = self.lines[self.row][self.col..].to_string();
            self.lines[self.row].truncate(self.col);
            self.sticky = None;
            self.record_kill(&killed, false);
            return;
        }
        // At the end of a line the newline itself is what gets killed, and it joins the ring
        // as a literal newline so yanking it back puts the break where it was.
        if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            self.record_kill("\n", false);
        }
        self.sticky = None;
    }

    /// A backward kill at the start of a line takes the line break before it.
    fn kill_newline_before(&mut self) {
        if self.row == 0 {
            return;
        }
        self.checkpoint();
        let line = self.lines.remove(self.row);
        self.row -= 1;
        self.col = self.lines[self.row].len();
        self.lines[self.row].push_str(&line);
        self.sticky = None;
        self.record_kill("\n", true);
    }

    /// A forward kill at the end of a line takes the line break after it.
    fn kill_newline_after(&mut self) {
        if self.row + 1 >= self.lines.len() {
            return;
        }
        self.checkpoint();
        let next = self.lines.remove(self.row + 1);
        self.lines[self.row].push_str(&next);
        self.sticky = None;
        self.record_kill("\n", false);
    }

    /// Put killed text on the ring, merging with the kill before it when that is what the
    /// last keystroke was.
    fn record_kill(&mut self, text: &str, backward: bool) {
        let accumulate = self.last == LastAction::Kill;
        self.kill.push(text, backward, accumulate);
        self.last = LastAction::Kill;
    }

    pub fn move_left(&mut self) {
        // Motion steps over a marker whole; there is nothing meaningful inside it to land on.
        if let Some(marker) = self.marker_before_cursor() {
            self.col = marker.start;
            self.sticky = None;
            return;
        }
        if self.col > 0 {
            self.col = prev_boundary(&self.lines[self.row], self.col);
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].len();
        }
        self.sticky = None;
    }

    pub fn move_right(&mut self) {
        if let Some(marker) = paste::marker_starting_at(&self.lines[self.row], self.col) {
            self.col = marker.end;
            self.sticky = None;
            return;
        }
        if self.col < self.lines[self.row].len() {
            self.col = next_boundary(&self.lines[self.row], self.col);
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
        self.sticky = None;
    }

    pub fn move_word_left(&mut self) {
        if self.col == 0 {
            self.move_left();
            return;
        }
        self.col = word_start_before(&self.lines[self.row], self.col);
        self.sticky = None;
        self.snap_out_of_marker(false);
    }

    pub fn move_word_right(&mut self) {
        if self.col == self.lines[self.row].len() {
            self.move_right();
            return;
        }
        self.col = word_end_after(&self.lines[self.row], self.col);
        self.sticky = None;
        self.snap_out_of_marker(true);
    }

    pub fn move_line_start(&mut self) {
        self.col = 0;
        self.sticky = None;
    }

    pub fn move_line_end(&mut self) {
        self.col = self.lines[self.row].len();
        self.sticky = None;
    }

    pub fn move_start(&mut self) {
        self.row = 0;
        self.col = 0;
        self.sticky = None;
    }

    pub fn move_end(&mut self) {
        self.row = self.lines.len().saturating_sub(1);
        self.col = self.lines[self.row].len();
        self.sticky = None;
    }

    /// Move one display row up. Returns false when already on the first row, which lets a
    /// caller give the key another meaning at the top of the buffer.
    /// Take pasted text, holding a large paste aside behind a marker.
    ///
    /// A paste is cleaned first — line endings normalized, tabs widened, control characters
    /// dropped — and a path pasted onto the end of a word gets a space before it, so
    /// `cat` and `/etc/hosts` do not fuse into one token.
    pub fn paste(&mut self, text: &str) {
        let text = paste::clean(text);
        if text.is_empty() {
            return;
        }
        self.checkpoint();
        self.last = LastAction::Other;

        if paste::needs_separator(&self.lines[self.row][..self.col], &text) {
            self.insert_inner(" ");
        }

        match paste::is_large(&text) {
            true => {
                let marker = self.pastes.store(&text);
                self.insert_inner(&marker);
            }
            false => self.insert_inner(&text),
        }
    }

    /// The prompt as the model should see it, with every marker replaced by what it stands
    /// for. [`Editor::text`] gives what is on screen; this gives what was meant.
    pub fn expanded_text(&self) -> String {
        self.pastes.expand(&self.text())
    }

    /// Push the cursor out of any marker it landed inside.
    ///
    /// Word motion works on words, and a marker is full of them — but they describe the
    /// paste rather than being part of the prompt, so a motion that lands inside one carries
    /// on to its edge. Called after the move rather than before it, which is what makes it
    /// hold however the cursor arrived.
    fn snap_out_of_marker(&mut self, forward: bool) {
        if let Some(marker) = paste::marker_containing(&self.lines[self.row], self.col) {
            self.col = match forward {
                true => marker.end,
                false => marker.start,
            };
            self.sticky = None;
        }
    }

    /// The marker the cursor sits just after, if there is one.
    fn marker_before_cursor(&self) -> Option<paste::Marker> {
        paste::marker_ending_at(&self.lines[self.row], self.col)
    }

    /// The marker the cursor sits just before, if there is one.
    fn marker_after_cursor(&self) -> Option<paste::Marker> {
        paste::marker_starting_at(&self.lines[self.row], self.col)
    }

    /// Delete a whole marker and forget the paste behind it.
    ///
    /// The numbers of the pastes after it close up, and the markers still in the text are
    /// rewritten to match, so a prompt never shows `#3` when it holds two pastes.
    fn delete_marker(&mut self, marker: paste::Marker) {
        self.lines[self.row].replace_range(marker.start..marker.end, "");
        self.col = marker.start;
        self.sticky = None;

        let moved = self.pastes.remove(marker.id);
        if !moved.is_empty() {
            self.lines = self
                .lines
                .iter()
                .map(|line| paste::renumber(line, &moved))
                .collect();
        }
    }

    /// Move to the next occurrence of `target`, searching on across lines.
    ///
    /// The current line is searched from just past the cursor, and every line after it (or
    /// before it, going backward) in full. With no match anywhere the cursor stays put,
    /// rather than jumping somewhere arbitrary.
    pub fn jump_to_char(&mut self, target: char, forward: bool) {
        self.last = LastAction::Other;
        let rows: Vec<usize> = match forward {
            true => (self.row..self.lines.len()).collect(),
            false => (0..=self.row).rev().collect(),
        };

        for row in rows {
            let line = &self.lines[row];
            let found = match (forward, row == self.row) {
                (true, true) => {
                    let from = next_boundary(line, self.col);
                    line[from..].find(target).map(|index| from + index)
                }
                (true, false) => line.find(target),
                (false, true) => line[..self.col].rfind(target),
                (false, false) => line.rfind(target),
            };
            if let Some(column) = found {
                self.row = row;
                self.col = column;
                self.sticky = None;
                return;
            }
        }
    }

    /// True when the character immediately before the cursor is a backslash.
    ///
    /// Enter then inserts a line break instead of submitting, and takes the backslash with
    /// it — the way to type a multi-line prompt in a terminal that has no shift+enter.
    pub fn escapes_submit(&self) -> bool {
        self.lines[self.row][..self.col].ends_with('\\')
    }

    /// Take the escaping backslash out and break the line where it was.
    pub fn escape_newline(&mut self) {
        self.checkpoint();
        self.last = LastAction::Other;
        if self.col > 0 {
            let start = prev_boundary(&self.lines[self.row], self.col);
            self.lines[self.row].replace_range(start..self.col, "");
            self.col = start;
        }
        self.insert_inner("\n");
    }

    /// Rows a page key moves by: ohm's `max(5, floor(rows * 0.3))`, so a taller terminal
    /// pages further but a short one still moves a useful distance.
    pub fn page_rows(rows: usize) -> usize {
        5.max((rows as f64 * 0.3) as usize)
    }

    /// Move up a page, stopping at the top.
    pub fn page_up(&mut self, width: usize, rows: usize) {
        for _ in 0..Self::page_rows(rows) {
            if !self.move_up(width) {
                break;
            }
        }
    }

    /// Move down a page, stopping at the bottom.
    pub fn page_down(&mut self, width: usize, rows: usize) {
        for _ in 0..Self::page_rows(rows) {
            if !self.move_down(width) {
                break;
            }
        }
    }

    pub fn move_up(&mut self, width: usize) -> bool {
        let layout = self.layout(width);
        if layout.cursor_row == 0 {
            return false;
        }
        self.move_to_row(&layout, layout.cursor_row - 1, layout.cursor_column);
        true
    }

    /// Move one display row down. Returns false when already on the last row.
    pub fn move_down(&mut self, width: usize) -> bool {
        let layout = self.layout(width);
        if layout.cursor_row + 1 >= layout.rows.len() {
            return false;
        }
        self.move_to_row(&layout, layout.cursor_row + 1, layout.cursor_column);
        true
    }

    fn move_to_row(&mut self, layout: &EditorLayout, target: usize, current_column: usize) {
        let column = self.sticky.unwrap_or(current_column);
        self.sticky = Some(column);
        let row = &layout.rows[target];
        self.row = row.line;
        self.col = column_to_byte(&self.lines[row.line], &row.range, column);
    }

    /// Rows the buffer occupies at `width` columns.
    pub fn height(&self, width: usize) -> usize {
        self.layout(width).rows.len()
    }

    /// Lay the buffer out for a viewport `width` columns wide and place the cursor on it.
    pub fn layout(&self, width: usize) -> EditorLayout {
        let mut rows = Vec::new();
        let mut cursor_row = 0;
        let mut cursor_column = 0;

        for (index, line) in self.lines.iter().enumerate() {
            let ranges = wrap_ranges(line, width);
            for range in ranges {
                if index == self.row && cursor_row_matches(&range, self.col, line.len()) {
                    cursor_row = rows.len();
                    cursor_column = text_width(&line[range.start..self.col]);
                }
                rows.push(VisualRow { line: index, range });
            }
        }

        EditorLayout {
            rows,
            cursor_row,
            cursor_column,
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

/// The cursor belongs to the row that contains it, and to the final row of a line when it
/// sits at the very end. A position on a wrap boundary belongs to the row that starts there.
fn cursor_row_matches(range: &Range<usize>, cursor: usize, line_length: usize) -> bool {
    if cursor < range.end {
        return cursor >= range.start;
    }
    cursor == line_length && range.end == line_length
}

/// The byte offset within `range` whose display column is closest to `column`.
fn column_to_byte(line: &str, range: &Range<usize>, column: usize) -> usize {
    // A row that ends mid-line is followed by another row starting at the same byte, so the
    // cursor stops one grapheme short to stay visually on this row.
    let limit = if range.end < line.len() {
        prev_boundary(line, range.end).max(range.start)
    } else {
        range.end
    };

    let mut index = range.start;
    let mut used = 0usize;
    for (offset, grapheme) in line[range.start..limit].grapheme_indices(true) {
        let advance = grapheme_width(grapheme);
        if used + advance > column {
            return range.start + offset;
        }
        used += advance;
        index = range.start + offset + grapheme.len();
    }
    index
}

fn prev_boundary(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(offset, _)| offset)
        .take_while(|offset| *offset < index)
        .last()
        .unwrap_or(0)
}

fn next_boundary(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(offset, grapheme)| offset + grapheme.len())
        .find(|offset| *offset > index)
        .unwrap_or(text.len())
}

/// What kind of character this is, for deciding where a word ends.
///
/// Punctuation is its own kind rather than a separator like whitespace. A path or a call
/// is a run of words with punctuation between them, and treating the punctuation as a gap
/// would step over `foo.bar` in one move where a reader expects three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn class_of(text: &str) -> CharClass {
    match text.chars().next() {
        Some(character) if character.is_whitespace() => CharClass::Whitespace,
        Some(character) if character.is_alphanumeric() || character == '_' => CharClass::Word,
        Some(character) if character.is_ascii_punctuation() => CharClass::Punctuation,
        // Anything else — a symbol, an emoji — is a thing rather than a gap.
        Some(_) => CharClass::Word,
        None => CharClass::Whitespace,
    }
}

/// Start of the word before `index`: skip any whitespace, then one run of one kind.
fn word_start_before(text: &str, index: usize) -> usize {
    let mut cursor = index;
    while cursor > 0 {
        let previous = prev_boundary(text, cursor);
        if class_of(&text[previous..cursor]) != CharClass::Whitespace {
            break;
        }
        cursor = previous;
    }
    if cursor == 0 {
        return cursor;
    }

    let run = class_of(&text[prev_boundary(text, cursor)..cursor]);
    while cursor > 0 {
        let previous = prev_boundary(text, cursor);
        if class_of(&text[previous..cursor]) != run {
            break;
        }
        cursor = previous;
    }
    cursor
}

/// End of the word after `index`: skip any separators, then the word itself.
fn word_end_after(text: &str, index: usize) -> usize {
    let mut cursor = index;
    while cursor < text.len() {
        let next = next_boundary(text, cursor);
        if class_of(&text[cursor..next]) != CharClass::Whitespace {
            break;
        }
        cursor = next;
    }
    if cursor >= text.len() {
        return cursor;
    }

    let run = class_of(&text[cursor..next_boundary(text, cursor)]);
    while cursor < text.len() {
        let next = next_boundary(text, cursor);
        if class_of(&text[cursor..next]) != run {
            break;
        }
        cursor = next;
    }
    cursor
}

/// Collapse line endings and drop control characters the terminal cannot draw. Tabs survive
/// because pasted code depends on them; they render one column wide.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(character),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(text: &str) -> Editor {
        let mut editor = Editor::new();
        editor.set_text(text);
        editor
    }

    #[test]
    fn typing_builds_a_line() {
        let mut editor = Editor::new();
        for character in "hello".chars() {
            editor.insert_char(character);
        }
        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.cursor(), (0, 5));
    }

    #[test]
    fn inserting_happens_at_the_cursor() {
        let mut editor = editor_with("helo");
        editor.move_left();
        editor.insert_char('l');
        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.cursor(), (0, 4));
    }

    #[test]
    fn newline_splits_the_line_at_the_cursor() {
        let mut editor = editor_with("hello world");
        editor.move_line_start();
        editor.move_word_right();
        editor.insert_newline();
        assert_eq!(editor.text(), "hello\n world");
        assert_eq!(editor.cursor(), (1, 0));
    }

    #[test]
    fn backspace_removes_one_grapheme() {
        let mut editor = editor_with("héllo");
        editor.backspace();
        assert_eq!(editor.text(), "héll");
    }

    #[test]
    fn backspace_at_the_line_start_joins_the_previous_line() {
        let mut editor = editor_with("one\ntwo");
        editor.move_line_start();
        editor.backspace();
        assert_eq!(editor.text(), "onetwo");
        assert_eq!(editor.cursor(), (0, 3));
    }

    #[test]
    fn delete_removes_forward_and_joins_lines() {
        let mut editor = editor_with("one\ntwo");
        editor.move_start();
        editor.delete();
        assert_eq!(editor.text(), "ne\ntwo");
        editor.move_line_end();
        editor.delete();
        assert_eq!(editor.text(), "netwo");
    }

    #[test]
    fn a_multi_codepoint_grapheme_is_deleted_whole() {
        let mut editor = editor_with("ok 👩‍💻");
        editor.backspace();
        assert_eq!(editor.text(), "ok ");
    }

    /// Whitespace is a gap; punctuation is a stop of its own. `beta.gamma` is three
    /// moves, not one, which is what a reader editing a path or a call expects.
    #[test]
    fn word_motion_stops_at_punctuation_as_well_as_at_words() {
        let mut editor = editor_with("alpha  beta.gamma");
        editor.move_line_start();
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 5, "past alpha");
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 11, "past the gap and beta");
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 12, "past the dot on its own");
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 17, "past gamma");

        editor.move_word_left();
        assert_eq!(editor.cursor().1, 12, "back to the start of gamma");
        editor.move_word_left();
        assert_eq!(editor.cursor().1, 11, "back over the dot");
        editor.move_word_left();
        assert_eq!(editor.cursor().1, 7, "back to the start of beta");
    }

    /// A run of punctuation is one stop, not one per character.
    #[test]
    fn a_run_of_punctuation_moves_as_one() {
        let mut editor = editor_with("a ==> b");
        editor.move_line_start();
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 1);
        editor.move_word_right();
        assert_eq!(editor.cursor().1, 5, "the whole ==> at once");
    }

    #[test]
    fn word_motion_crosses_line_boundaries() {
        let mut editor = editor_with("one\ntwo");
        editor.move_start();
        editor.move_word_left();
        assert_eq!(editor.cursor(), (0, 0));
        editor.move_line_end();
        editor.move_word_right();
        assert_eq!(editor.cursor(), (1, 0));
    }

    #[test]
    fn deleting_a_word_removes_it_and_its_separators() {
        let mut editor = editor_with("remove the  last");
        editor.delete_word_before();
        assert_eq!(editor.text(), "remove the  ");
        editor.delete_word_before();
        assert_eq!(editor.text(), "remove ");
    }

    #[test]
    fn deleting_a_word_forward_stops_at_the_next_word_end() {
        let mut editor = editor_with("keep this away");
        editor.move_line_start();
        editor.move_word_right();
        editor.delete_word_after();
        assert_eq!(editor.text(), "keep away");
    }

    #[test]
    fn line_deletions_cut_either_side_of_the_cursor() {
        let mut editor = editor_with("abcdef");
        editor.move_line_start();
        editor.move_word_right();
        editor.delete_to_line_start();
        assert_eq!(editor.text(), "");

        let mut editor = editor_with("abc def");
        editor.move_line_start();
        editor.move_word_right();
        editor.delete_to_line_end();
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn replacing_before_the_cursor_swaps_what_was_typed() {
        let mut editor = editor_with("/mod");
        editor.replace_before_cursor(4, "/model ");
        assert_eq!(editor.text(), "/model ");
        assert_eq!(editor.cursor(), (0, 7));

        // The rest of the line is left where it was.
        let mut editor = editor_with("/mod tail");
        editor.move_line_start();
        for _ in 0..4 {
            editor.move_right();
        }
        editor.replace_before_cursor(4, "/model");
        assert_eq!(editor.text(), "/model tail");
    }

    #[test]
    fn a_multi_line_paste_stays_one_block() {
        let mut editor = editor_with("prefix ");
        editor.insert_str("first\nsecond\nthird");
        assert_eq!(editor.text(), "prefix first\nsecond\nthird");
        assert_eq!(editor.cursor(), (2, 5));
    }

    #[test]
    fn a_paste_lands_inside_existing_text() {
        let mut editor = editor_with("ab");
        editor.move_line_start();
        editor.move_right();
        editor.insert_str("X\nY");
        assert_eq!(editor.text(), "aX\nYb");
        assert_eq!(editor.cursor(), (1, 1));
    }

    #[test]
    fn paste_normalizes_line_endings_and_drops_control_characters() {
        let mut editor = Editor::new();
        editor.insert_str("one\r\ntwo\rthree\u{1b}[0m\tfour");
        assert_eq!(editor.text(), "one\ntwo\nthree[0m\tfour");
    }

    #[test]
    fn vertical_motion_moves_between_logical_lines() {
        let mut editor = editor_with("first line\nsecond");
        editor.move_line_start();
        assert!(editor.move_up(40));
        assert_eq!(editor.cursor(), (0, 0));
        assert!(editor.move_down(40));
        assert_eq!(editor.cursor(), (1, 0));
        assert!(!editor.move_down(40));
    }

    #[test]
    fn vertical_motion_moves_within_a_wrapped_line() {
        let mut editor = editor_with("aaaa bbbb cccc");
        // Rows at width 5: "aaaa ", "bbbb ", "cccc".
        assert_eq!(editor.height(5), 3);
        assert!(editor.move_up(5));
        assert_eq!(editor.cursor(), (0, 9));
        assert!(editor.move_up(5));
        assert_eq!(editor.cursor(), (0, 4));
        assert!(!editor.move_up(5));
    }

    #[test]
    fn vertical_motion_remembers_the_column_across_a_short_line() {
        let mut editor = editor_with("longer line\nx\nlonger line");
        editor.move_end();
        let column = editor.layout(40).cursor_column;
        editor.move_up(40);
        editor.move_up(40);
        assert_eq!(editor.layout(40).cursor_column, column);
        assert_eq!(editor.cursor(), (0, 11));
    }

    #[test]
    fn layout_places_the_cursor_on_the_row_that_holds_it() {
        let mut editor = editor_with("aaaa bbbb");
        editor.move_start();
        let layout = editor.layout(5);
        assert_eq!(layout.cursor_row, 0);
        assert_eq!(layout.cursor_column, 0);

        editor.move_end();
        let layout = editor.layout(5);
        assert_eq!(layout.cursor_row, 1);
        assert_eq!(layout.cursor_column, 4);
    }

    #[test]
    fn take_empties_the_buffer() {
        let mut editor = editor_with("submit me");
        assert_eq!(editor.take(), "submit me");
        assert!(editor.is_empty());
        assert_eq!(editor.cursor(), (0, 0));
        assert_eq!(editor.height(20), 1);
    }

    #[test]
    fn wide_characters_advance_the_display_column() {
        let editor = editor_with("日本語");
        assert_eq!(editor.layout(40).cursor_column, 6);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        let mut editor = Editor::new();
        editor.insert_str(text);
        editor
    }

    #[test]
    fn a_word_kill_can_be_yanked_back() {
        let mut editor = editor("hello world");
        editor.delete_word_before();
        assert_eq!(editor.text(), "hello ");
        editor.yank();
        assert_eq!(editor.text(), "hello world");
    }

    #[test]
    fn consecutive_kills_yank_back_in_reading_order() {
        let mut editor = editor("one two three");
        editor.delete_word_before();
        editor.delete_word_before();
        assert_eq!(editor.text(), "one ");
        editor.yank();
        assert_eq!(editor.text(), "one two three");
    }

    /// A kill that is not adjacent to the one before it starts its own entry, so yank
    /// returns only the newer one.
    #[test]
    fn a_kill_interrupted_by_typing_starts_its_own_entry() {
        let mut editor = editor("alpha beta");
        editor.delete_word_before();
        editor.insert_char('x');
        editor.delete_word_before();
        editor.yank();
        assert_eq!(editor.text(), "alpha x");
    }

    #[test]
    fn yank_pop_reaches_the_kill_before_the_last_one() {
        let mut editor = editor("first");
        editor.delete_to_line_start();
        editor.insert_str("second");
        editor.delete_to_line_start();

        editor.yank();
        assert_eq!(editor.text(), "second");
        assert!(editor.yank_pop());
        assert_eq!(editor.text(), "first");
    }

    #[test]
    fn yank_pop_does_nothing_unless_a_yank_came_first() {
        let mut editor = editor("one");
        editor.delete_to_line_start();
        editor.insert_str("two");
        editor.delete_to_line_start();
        assert!(!editor.yank_pop(), "the last action was a kill, not a yank");
    }

    #[test]
    fn plain_backspace_leaves_the_ring_alone() {
        let mut editor = editor("word");
        editor.delete_word_before();
        editor.insert_str("kept");
        editor.backspace();
        editor.yank();
        assert_eq!(editor.text(), "kepword", "the ring still holds the word kill");
    }

    #[test]
    fn a_run_of_word_characters_undoes_as_one_unit() {
        let mut editor = Editor::new();
        for character in "hello".chars() {
            editor.insert_char(character);
        }
        editor.undo();
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn a_space_makes_the_word_after_it_separately_undoable() {
        let mut editor = Editor::new();
        for character in "one two".chars() {
            editor.insert_char(character);
        }
        editor.undo();
        assert_eq!(editor.text(), "one ", "the second word goes back on its own");
        editor.undo();
        assert_eq!(editor.text(), "one", "and then the space");
    }

    #[test]
    fn undo_with_nothing_to_undo_changes_nothing() {
        let mut editor = Editor::new();
        assert!(!editor.undo());
        assert_eq!(editor.text(), "");
    }

    fn big() -> String {
        "a line of pasted text\n".repeat(40)
    }

    #[test]
    fn a_small_paste_goes_straight_in() {
        let mut editor = Editor::new();
        editor.paste("just a little");
        assert_eq!(editor.text(), "just a little");
        assert_eq!(editor.expanded_text(), "just a little");
    }

    #[test]
    fn a_large_paste_is_stood_in_for_by_a_marker() {
        let mut editor = Editor::new();
        editor.paste(&big());
        assert_eq!(editor.text(), "[paste #1 +41 lines]");
        assert_eq!(editor.expanded_text(), big(), "and comes back on submit");
    }

    /// The marker is one unit: backspace takes all of it rather than breaking it into text
    /// that no longer stands for anything.
    #[test]
    fn backspace_takes_a_whole_marker() {
        let mut editor = Editor::new();
        editor.insert_str("see ");
        editor.paste(&big());
        editor.backspace();
        assert_eq!(editor.text(), "see ");
        assert_eq!(editor.expanded_text(), "see ");
    }

    /// Forward delete is the other half of the same rule: a marker in front of the cursor
    /// goes all at once rather than losing its opening bracket.
    #[test]
    fn forward_delete_takes_a_whole_marker() {
        let mut editor = Editor::new();
        editor.paste(&big());
        editor.insert_str(" see");
        editor.move_line_start();
        editor.delete();
        assert_eq!(editor.text(), " see");
        assert_eq!(editor.expanded_text(), " see");
    }

    /// Forward delete is a change like any other, so it can be taken back.
    #[test]
    fn forward_delete_can_be_undone() {
        let mut editor = Editor::new();
        editor.insert_str("hello");
        editor.move_line_start();
        editor.delete();
        assert_eq!(editor.text(), "ello");
        editor.undo();
        assert_eq!(editor.text(), "hello");
    }

    #[test]
    fn motion_steps_over_a_marker_whole() {
        let mut editor = Editor::new();
        editor.paste(&big());
        let end = editor.cursor().1;
        editor.move_left();
        assert_eq!(editor.cursor().1, 0, "left of the whole marker");
        editor.move_right();
        assert_eq!(editor.cursor().1, end, "and back past all of it");
    }

    /// Deleting the first of two pastes renumbers the second, so the prompt never shows a
    /// number higher than the count of pastes in it.
    #[test]
    fn removing_a_paste_renumbers_the_markers_after_it() {
        let mut editor = Editor::new();
        editor.paste(&big());
        editor.insert_str(" and ");
        editor.paste(&big());

        editor.move_line_start();
        editor.move_right();
        editor.backspace();

        assert_eq!(editor.text(), " and [paste #1 +41 lines]");
        assert_eq!(editor.expanded_text(), format!(" and {}", big()));
    }

    #[test]
    fn word_motion_steps_over_a_marker_whole() {
        let mut editor = Editor::new();
        editor.insert_str("before ");
        editor.paste(&big());
        editor.insert_str(" after");

        editor.move_line_start();
        editor.move_word_right();
        editor.move_word_right();
        let after_marker = editor.cursor().1;
        assert_eq!(
            &editor.text()[..after_marker],
            "before [paste #1 +41 lines]",
            "it landed past the whole marker, never inside it"
        );

        editor.move_word_left();
        assert_eq!(editor.cursor().1, 7, "and back to the front of it");
    }

    #[test]
    fn a_path_pasted_onto_a_word_gets_a_space_first() {
        let mut editor = Editor::new();
        editor.insert_str("cat");
        editor.paste("/etc/hosts");
        assert_eq!(editor.text(), "cat /etc/hosts");
    }

    #[test]
    fn a_paste_is_cleaned_before_it_lands() {
        let mut editor = Editor::new();
        editor.paste("a\r\nb\tc");
        assert_eq!(editor.text(), "a\nb    c");
    }

    #[test]
    fn submitting_forgets_the_pastes_that_belonged_to_the_prompt() {
        let mut editor = Editor::new();
        editor.paste(&big());
        let sent = editor.take();
        assert_eq!(sent, big());
        editor.paste(&big());
        assert_eq!(editor.text(), "[paste #1 +41 lines]", "numbering starts over");
    }

    #[test]
    fn a_jump_lands_on_the_next_occurrence() {
        let mut editor = editor("alpha beta gamma");
        editor.move_line_start();
        editor.jump_to_char('a', true);
        assert_eq!(editor.cursor().1, 4, "the a in alpha, not the one under the cursor");
        editor.jump_to_char('g', true);
        assert_eq!(editor.cursor().1, 11);
    }

    #[test]
    fn a_jump_searches_on_across_lines() {
        let mut editor = Editor::new();
        editor.insert_str("first\nsecond\nthird");
        editor.move_start();
        editor.jump_to_char('c', true);
        assert_eq!(editor.cursor(), (1, 2), "the c in second, on the line below");
    }

    #[test]
    fn a_backward_jump_looks_the_other_way() {
        let mut editor = Editor::new();
        editor.insert_str("one\ntwo\nthree");
        editor.jump_to_char('w', false);
        assert_eq!(editor.cursor(), (1, 1));
    }

    #[test]
    fn a_jump_with_no_match_leaves_the_cursor_alone() {
        let mut editor = editor("nothing here");
        let before = editor.cursor();
        editor.jump_to_char('z', true);
        assert_eq!(editor.cursor(), before);
    }

    #[test]
    fn a_trailing_backslash_escapes_submit() {
        let mut editor = editor("first line\\");
        assert!(editor.escapes_submit());
        editor.escape_newline();
        assert_eq!(editor.text(), "first line\n", "the backslash went with it");
        assert!(!editor.escapes_submit());
    }

    #[test]
    fn text_without_a_trailing_backslash_submits() {
        assert!(!editor("plain").escapes_submit());
        assert!(!editor("").escapes_submit());
    }

    /// ohm pages by `max(5, floor(rows * 0.3))`, so a tall terminal pages further while a
    /// short one still moves a useful distance.
    #[test]
    fn a_page_is_a_third_of_the_screen_but_never_under_five_rows() {
        assert_eq!(Editor::page_rows(10), 5);
        assert_eq!(Editor::page_rows(20), 6);
        assert_eq!(Editor::page_rows(50), 15);
    }

    #[test]
    fn paging_stops_at_the_edges_rather_than_running_off() {
        let mut editor = Editor::new();
        editor.insert_str("one\ntwo\nthree");
        // Paging keeps the display column, the way every vertical motion does, so only the
        // row is asserted here.
        editor.page_up(40, 24);
        assert_eq!(editor.cursor().0, 0, "stopped at the top rather than running off");
        editor.page_down(40, 24);
        assert_eq!(editor.cursor().0, 2, "and at the bottom");
    }

    #[test]
    fn history_walks_back_through_submitted_prompts() {
        let mut editor = Editor::new();
        editor.remember("first");
        editor.remember("second");

        assert!(editor.history_previous());
        assert_eq!(editor.text(), "second");
        assert!(editor.history_previous());
        assert_eq!(editor.text(), "first");
        assert!(!editor.history_previous(), "nothing older to reach");
    }

    #[test]
    fn history_gives_back_the_draft_it_interrupted() {
        let mut editor = Editor::new();
        editor.remember("sent");
        editor.insert_str("half written");

        assert!(editor.history_previous());
        assert_eq!(editor.text(), "sent");
        assert!(editor.history_next());
        assert_eq!(editor.text(), "half written");
        assert!(!editor.is_browsing_history());
    }

    #[test]
    fn the_same_prompt_twice_running_is_one_entry() {
        let mut editor = Editor::new();
        editor.remember("same");
        editor.remember("same");
        editor.history_previous();
        assert!(!editor.history_previous(), "only one entry to reach");
    }
}
