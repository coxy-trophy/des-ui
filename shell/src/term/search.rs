//! Incremental literal search over a snapshot of the configured VT history.
use std::time::{Duration, Instant};

const MAX_MATCHES: usize = 10_000;
const MAX_MATCH_CELLS: usize = 262_144;

#[derive(Default)]
pub(super) struct Search {
    pub open: bool,
    pub composing: bool,
    pub truncated: bool,
    cells_stored: usize,
    pub query: String,
    pub matches: Vec<Match>,
    pub current: usize,
    pub history: usize,
    pub focus: bool,
    pub dirty: bool,
    pub revision: u64,
    snapshot: Option<vt100::Screen>,
    row: usize,
    line: Vec<(String, (usize, u16, u16))>,
}

pub(super) struct Match {
    pub text: String,
    pub cells: Vec<(usize, u16, u16)>,
}

impl Search {
    pub fn restart(&mut self, screen: &vt100::Screen) {
        self.matches.clear();
        self.truncated = false;
        self.cells_stored = 0;
        self.current = 0;
        self.row = 0;
        self.line.clear();
        self.dirty = true;
        self.snapshot = None;
        if self.query.is_empty() { return; }
        let mut snapshot = screen.clone();
        snapshot.set_scrollback(usize::MAX);
        self.history = snapshot.scrollback();
        self.snapshot = Some(snapshot);
    }

    pub fn pending(&self) -> bool { self.snapshot.is_some() }

    pub fn advance(&mut self, previous: bool) {
        let n = self.matches.len();
        if n == 0 { return; }
        self.current = if previous { (self.current + n - 1) % n } else { (self.current + 1) % n };
        self.dirty = true;
    }

    pub fn scan(&mut self) {
        let started = Instant::now();
        let Some(mut screen) = self.snapshot.take() else { return; };
        let (rows, cols) = screen.size();
        let total = self.history + rows as usize;
        for _ in 0..128 {
            if self.row >= total { break; }
            // Inspect each historical row once; the final live rows share one viewport.
            let offset = self.history.saturating_sub(self.row);
            screen.set_scrollback(offset);
            let visible = self.row.saturating_sub(self.history) as u16;
            for col in 0..cols {
                let Some(cell) = screen.cell(visible, col) else { continue; };
                if cell.is_wide_continuation() { continue; }
                let text = if cell.has_contents() { cell.contents() } else { " " };
                self.line.push((text.to_owned(), (self.row, col, if cell.is_wide() { 2 } else { 1 })));
            }
            self.row += 1;
            let matched_through = self.match_line();
            if self.truncated { self.line.clear(); break; }
            if screen.row_wrapped(visible) && self.row < total {
                // Only a query-sized tail can participate in a later wrapped match.
                let keep = self.query.to_lowercase().len();
                let discard = matched_through.max(self.line.len().saturating_sub(keep));
                self.line.drain(..discard);
            } else { self.line.clear(); }
            if started.elapsed() >= Duration::from_millis(3) { break; }
        }
        if self.row < total && !self.truncated { self.snapshot = Some(screen); }
        self.dirty = true;
    }

    fn match_line(&mut self) -> usize {
        let mut matched_through = 0;
        let needle = self.query.to_lowercase();
        let mut haystack = String::new();
        let mut map = Vec::new();
        for (index, (text, _)) in self.line.iter().enumerate() {
            let folded = text.to_lowercase();
            map.extend(std::iter::repeat_n(index, folded.len()));
            haystack.push_str(&folded);
        }
        for (start, _) in haystack.match_indices(&needle) {
            let end = start + needle.len() - 1;
            let from = map[start];
            let to = map[end];
            // Retained tails were searched in a previous row.
            if self.line[to].1.0 + 1 != self.row { continue; }
            let cells = to - from + 1;
            if self.cells_stored + cells > MAX_MATCH_CELLS { self.truncated = true; break; }
            self.cells_stored += cells;
            matched_through = to + 1;
            self.matches.push(Match {
                text: self.line[from..=to].iter().map(|(s, _)| s.as_str()).collect(),
                cells: self.line[from..=to].iter().map(|(_, cell)| *cell).collect(),
            });
            if self.matches.len() >= MAX_MATCHES { self.truncated = true; break; }
        }
        matched_through
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_queries_also_bound_total_stored_match_cells() {
        let mut parser = vt100::Parser::new(2, 100, 4000);
        parser.process("a".repeat(300_000).as_bytes());
        let mut search = Search::default();
        search.query = "a".repeat(512);
        search.restart(parser.screen());
        while search.pending() { search.scan(); }
        assert!(search.truncated);
        assert!(search.matches.len() < MAX_MATCHES);
        assert!(search.cells_stored <= MAX_MATCH_CELLS);
        assert_eq!(search.cells_stored, search.matches.iter().map(|m| m.cells.len()).sum::<usize>());
    }

    #[test]
    fn common_character_results_stop_at_explicit_limit() {
        let mut parser = vt100::Parser::new(2, 100, 1000);
        parser.process("a".repeat(20_000).as_bytes());
        let mut search = Search::default();
        search.query = "a".into();
        search.restart(parser.screen());
        while search.pending() { search.scan(); }
        assert_eq!(search.matches.len(), MAX_MATCHES);
        assert!(search.truncated);
        assert!(search.cells_stored <= MAX_MATCH_CELLS);
        search.query = "absent".into();
        search.restart(parser.screen());
        assert!(!search.truncated);
    }

    #[test]
    fn history_unicode_literal_and_wrap_navigation() {
        let mut parser = vt100::Parser::new(2, 20, 100);
        parser.process("Äpfel [one]\r\nsecond\r\nÄPFEL [two]\r\nlast".as_bytes());
        let mut search = Search::default();
        search.query = "äpfel".into();
        search.restart(parser.screen());
        while search.snapshot.is_some() { search.scan(); }
        assert_eq!(search.matches.len(), 2);
        assert!(search.matches[0].cells[0].0 < search.history);
        search.advance(true);
        assert_eq!(search.current, 1);
        search.advance(false);
        assert_eq!(search.current, 0);
        search.query = "[".into();
        search.restart(parser.screen());
        while search.snapshot.is_some() { search.scan(); }
        assert_eq!(search.matches.len(), 2);
    }
    #[test]
    fn large_history_yields_and_query_changes_discard_stale_results() {
        let mut parser = vt100::Parser::new(2, 20, 1000);
        for _ in 0..400 { parser.process(b"old needle\r\n"); }
        let mut search = Search::default();
        search.query = "needle".into();
        search.restart(parser.screen());
        search.scan();
        assert!(search.pending(), "full history must yield between frame budgets");
        assert!(search.matches.len() <= 128);
        search.query = "absent".into();
        search.restart(parser.screen());
        assert!(search.matches.is_empty());
        while search.pending() { search.scan(); }
        assert!(search.matches.is_empty());
        search.query.clear();
        search.restart(parser.screen());
        assert!(!search.pending());
    }

    #[test]
    fn repeated_matches_keep_nonoverlapping_alignment_across_wraps() {
        let mut parser = vt100::Parser::new(3, 4, 10);
        parser.process(b"AAAAAAAA");
        let mut search = Search::default();
        search.query = "AAA".into();
        search.restart(parser.screen());
        while search.pending() { search.scan(); }
        assert_eq!(search.matches.len(), 2);
        assert_eq!(search.matches[1].cells[0], (0, 3, 1));
    }

    #[test]
    fn soft_wrapped_text_and_wide_cells_keep_coordinates() {
        let mut parser = vt100::Parser::new(3, 6, 10);
        parser.process("中文ABCdef".as_bytes());
        let mut search = Search::default();
        search.query = "文abcd".into();
        search.restart(parser.screen());
        while search.snapshot.is_some() { search.scan(); }
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].text, "文ABCd");
        assert_eq!(search.matches[0].cells[0], (0, 2, 2));
    }
}
