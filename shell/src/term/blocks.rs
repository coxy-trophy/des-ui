//! Command blocks: the prompt, command and output marks a shell sends as OSC 133,
//! kept as absolute line numbers so that they stay put while the terminal scrolls.
//!
//! vt100 does not say how many rows have ever left the top of its grid, so this
//! module keeps that count itself. While the scrollback is not yet full, the growth
//! of the scrollback is exact. Once it is full, output is fed to the parser in
//! small tokens and the cursor is watched: a line feed on the bottom row, a wrap
//! from the last column, and `CSI n S` are the only ways a row can leave the grid.
//! Scroll regions that start below the top row are the one case counted wrongly
//! then, and only programs that do not use the alternate screen set them.
use std::collections::VecDeque;

const MAX_BLOCKS: usize = 2000;
/// A sequence split across chunks is held back for the next chunk, up to this much.
const MAX_HELD: usize = 4096;
const OSC_133: &[u8] = b"\x1b]133;";

/// One command at a prompt, in absolute lines: the prompt's line, the first line of
/// output once the command runs, and the line after the output with the exit status
/// once it has finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Block {
    pub prompt: u64,
    pub output: Option<u64>,
    pub end: Option<(u64, i32)>,
}

impl Block {
    /// The lines the output occupies, when the command has run.
    pub fn output_lines(&self) -> Option<(u64, u64)> {
        let start = self.output?;
        let end = self.end.map_or(u64::MAX, |(line, _)| line);
        (end > start).then_some((start, end))
    }

    pub fn status(&self) -> Option<i32> {
        self.end.map(|(_, status)| status)
    }

}

pub(super) struct Blocks {
    /// Rows that have left the top of the live grid since the terminal started.
    pub scrolled: u64,
    limit: usize,
    blocks: VecDeque<Block>,
    held: Vec<u8>,
}

/// How many rows the scrollback holds right now, without moving the view.
pub(super) fn history_len(screen: &mut vt100::Screen) -> usize {
    let offset = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(offset);
    len
}

impl Blocks {
    pub fn new(limit: usize) -> Blocks {
        Blocks { scrolled: 0, limit, blocks: VecDeque::new(), held: Vec::new() }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Block> {
        self.blocks.iter()
    }

    /// The most recent command that has finished with output.
    pub fn last_output(&self) -> Option<(u64, u64)> {
        self.blocks.iter().rev().filter(|b| b.end.is_some()).find_map(Block::output_lines)
    }

    pub fn at_prompt(&self, line: u64) -> Option<&Block> {
        self.blocks.iter().rev().find(|b| b.prompt == line)
    }

    /// The block whose prompt or output covers `line`.
    pub fn covering(&self, line: u64) -> Option<&Block> {
        self.blocks.iter().rev().find(|b| {
            b.prompt <= line && b.end.map_or(true, |(end, _)| line < end.max(b.prompt + 1))
        })
    }

    pub fn prompt_before(&self, line: u64) -> Option<u64> {
        self.blocks.iter().rev().map(|b| b.prompt).find(|&p| p < line)
    }

    pub fn prompt_after(&self, line: u64) -> Option<u64> {
        self.blocks.iter().map(|b| b.prompt).find(|&p| p > line)
    }

    /// Forgets blocks that have scrolled out of the retained history.
    pub fn prune(&mut self, oldest_retained: u64) {
        while self.blocks.front().is_some_and(|b| {
            b.end.map_or(b.prompt, |(end, _)| end.max(b.prompt)) < oldest_retained
        }) {
            self.blocks.pop_front();
        }
    }

    /// Feeds shell output to the parser, taking the OSC 133 marks out on the way.
    pub fn feed(&mut self, parser: &mut vt100::Parser, bytes: &[u8]) {
        let mut data;
        let bytes = if self.held.is_empty() {
            bytes
        } else {
            data = std::mem::take(&mut self.held);
            data.extend_from_slice(bytes);
            &data[..]
        };
        let mut start = 0;
        let mut at = 0;
        while let Some(found) = find(&bytes[at..], OSC_133) {
            let osc = at + found;
            let Some(end) = osc_end(bytes, osc) else {
                break;
            };
            self.process(parser, &bytes[start..osc]);
            self.mark(parser.screen(), &bytes[osc + OSC_133.len()..end.0]);
            start = end.1;
            at = end.1;
        }
        // A sequence that may continue in the next chunk waits for it.
        let keep = partial_tail(&bytes[start..]).filter(|&n| n <= MAX_HELD);
        let processed = bytes.len() - keep.unwrap_or(0);
        self.process(parser, &bytes[start..processed]);
        self.held = bytes[processed..].to_vec();
    }

    /// Feeds what was held back, when no continuation is coming.
    pub fn flush(&mut self, parser: &mut vt100::Parser) {
        if !self.held.is_empty() {
            let held = std::mem::take(&mut self.held);
            self.process(parser, &held);
        }
    }

    /// The absolute line the cursor is on.
    pub fn cursor_line(&self, screen: &vt100::Screen) -> u64 {
        self.scrolled + u64::from(screen.cursor_position().0)
    }

    /// The absolute line at the top of the view.
    pub fn top_line(&self, screen: &vt100::Screen) -> u64 {
        self.scrolled.saturating_sub(screen.scrollback() as u64)
    }

    /// The scrollback offset that puts `line` at the top of the view, as near as
    /// the retained history allows.
    pub fn offset_for(&self, line: u64) -> usize {
        usize::try_from(self.scrolled.saturating_sub(line)).unwrap_or(usize::MAX)
    }

    fn mark(&mut self, screen: &vt100::Screen, params: &[u8]) {
        let line = self.cursor_line(screen);
        let mut parts = params.split(|&b| b == b';');
        let kind = parts.next().unwrap_or_default();
        match kind {
            b"A" => {
                // A prompt drawn again, or one nothing has run at, moves rather than
                // starting another block.
                match self.blocks.back_mut() {
                    Some(last) if last.output.is_none() && last.end.is_none() => last.prompt = line,
                    _ => self.push(Block { prompt: line, output: None, end: None }),
                }
            }
            b"C" => match self.blocks.back_mut() {
                Some(last) if last.output.is_none() && last.end.is_none() => last.output = Some(line),
                _ => self.push(Block { prompt: line, output: Some(line), end: None }),
            },
            b"D" => {
                let status = parts
                    .next()
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                if let Some(last) = self.blocks.back_mut().filter(|last| last.end.is_none()) {
                    last.end = Some((line.max(last.output.unwrap_or(last.prompt)), status));
                }
            }
            _ => {}
        }
    }

    fn push(&mut self, block: Block) {
        if self.blocks.len() >= MAX_BLOCKS {
            self.blocks.pop_front();
        }
        self.blocks.push_back(block);
    }

    /// Feeds bytes to the parser and keeps `scrolled` right.
    fn process(&mut self, parser: &mut vt100::Parser, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if parser.screen().alternate_screen() {
            // Nothing leaves the grid from the alternate screen; a program may
            // leave it within this chunk, so what follows is still watched.
            match bytes.iter().position(|&b| b == 0x1b) {
                None => return parser.process(bytes),
                Some(esc) => {
                    parser.process(&bytes[..esc]);
                    let end = sequence_end(bytes, esc).unwrap_or(bytes.len());
                    parser.process(&bytes[esc..end]);
                    return self.process(parser, &bytes[end..]);
                }
            }
        }
        let rows = parser.screen().size().0 as usize;
        let before = history_len(parser.screen_mut());
        // Each byte can push at most one row, and one `CSI S` a screenful: when all
        // of that still fits, the growth of the scrollback is the exact count.
        if before + bytes.len() + rows < self.limit {
            parser.process(bytes);
            let after = history_len(parser.screen_mut());
            self.scrolled += (after - before) as u64;
            return;
        }
        let mut at = 0;
        while at < bytes.len() {
            let (rows, cols) = parser.screen().size();
            let (r0, c0) = parser.screen().cursor_position();
            let bottom = r0 + 1 == rows;
            let b = bytes[at];
            let end;
            let mut pushed = 0;
            if b == 0x1b {
                end = sequence_end(bytes, at).unwrap_or(bytes.len());
                let seq = &bytes[at..end];
                parser.process(seq);
                let (r1, _) = parser.screen().cursor_position();
                match seq {
                    b"\x1bD" | b"\x1bE" if bottom && r1 == r0 => pushed = 1,
                    _ if seq.len() > 2 && seq[1] == b'[' && seq[seq.len() - 1] == b'S' => {
                        let count = std::str::from_utf8(&seq[2..seq.len() - 1]).ok()
                            .and_then(|n| n.parse::<u16>().ok()).unwrap_or(1).max(1);
                        pushed = u64::from(count.min(rows));
                    }
                    _ => {}
                }
            } else if b < 0x20 || b == 0x7f {
                end = at + 1;
                parser.process(&bytes[at..end]);
                let (r1, _) = parser.screen().cursor_position();
                if matches!(b, 0x0a | 0x0b | 0x0c) && bottom && r1 == r0 {
                    pushed = 1;
                }
            } else {
                // A run of text short enough to fit in the row cannot wrap except
                // from a pending last column, where one character tells.
                let chars = if c0 + 1 >= cols { 1 } else { usize::from((cols - 1 - c0) / 2).max(1) };
                end = text_run_end(bytes, at, chars);
                parser.process(&bytes[at..end]);
                let (r1, c1) = parser.screen().cursor_position();
                if bottom && r1 == r0 && c0 + 1 >= cols && c1 < c0 {
                    pushed = 1;
                }
            }
            if parser.screen().alternate_screen() {
                self.scrolled += pushed;
                return self.process(parser, &bytes[end..]);
            }
            self.scrolled += pushed;
            at = end;
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Where an OSC that starts at `start` ends: the index of its terminator, and the
/// index after it.
fn osc_end(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut i = start + 2;
    while i < bytes.len() {
        match bytes[i] {
            0x07 => return Some((i, i + 1)),
            0x1b if bytes.get(i + 1) == Some(&b'\\') => return Some((i, i + 2)),
            // A stray ESC ends the string too; the parser then sees it afresh.
            0x1b => return Some((i, i)),
            _ => i += 1,
        }
    }
    None
}

/// The end (exclusive) of the escape sequence starting at `esc`, or `None` when it
/// continues beyond these bytes.
fn sequence_end(bytes: &[u8], esc: usize) -> Option<usize> {
    let next = *bytes.get(esc + 1)?;
    match next {
        b']' => osc_end(bytes, esc).map(|(_, after)| after),
        b'[' => {
            let mut i = esc + 2;
            while i < bytes.len() {
                if (0x40..=0x7e).contains(&bytes[i]) {
                    return Some(i + 1);
                }
                i += 1;
            }
            None
        }
        b'P' | b'X' | b'^' | b'_' => {
            let mut i = esc + 2;
            while i + 1 < bytes.len() {
                if bytes[i] == 0x1b && bytes[i + 1] == b'\\' {
                    return Some(i + 2);
                }
                i += 1;
            }
            None
        }
        0x20..=0x2f => {
            let mut i = esc + 2;
            while i < bytes.len() {
                if (0x30..=0x7e).contains(&bytes[i]) {
                    return Some(i + 1);
                }
                i += 1;
            }
            None
        }
        _ => Some(esc + 2),
    }
}

/// How many bytes at the end could be the start of an OSC 133 that continues in
/// the next chunk.
fn partial_tail(bytes: &[u8]) -> Option<usize> {
    let esc = bytes.iter().rposition(|&b| b == 0x1b)?;
    let tail = &bytes[esc..];
    if tail.len() < OSC_133.len() {
        OSC_133.starts_with(tail).then_some(tail.len())
    } else {
        (tail.starts_with(OSC_133) && osc_end(bytes, esc).is_none()).then_some(tail.len())
    }
}

/// The end of a run of up to `chars` printable characters starting at `at`.
fn text_run_end(bytes: &[u8], at: usize, chars: usize) -> usize {
    let mut i = at;
    let mut seen = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x20 || b == 0x7f {
            break;
        }
        if b & 0xc0 != 0x80 {
            if seen == chars {
                break;
            }
            seen += 1;
        }
        i += 1;
    }
    i.max(at + 1).min(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_top(parser: &mut vt100::Parser, blocks: &Blocks, line: u64) -> String {
        parser.screen_mut().set_scrollback(blocks.offset_for(line));
        let (_, cols) = parser.screen().size();
        parser.screen().rows(0, cols).next().unwrap_or_default()
    }

    #[test]
    fn marks_are_taken_out_whole_split_and_with_either_terminator() {
        let mut parser = vt100::Parser::new(4, 20, 100);
        let mut blocks = Blocks::new(100);
        blocks.feed(&mut parser, b"\x1b]133;A\x07$ ls\r\n\x1b]13");
        assert_eq!(blocks.held, b"\x1b]13");
        blocks.feed(&mut parser, b"3;C\x07one\r\ntwo\r\n\x1b]133;D;3\x1b\\\x1b]133;A\x07$ ");
        assert!(blocks.held.is_empty());
        let block = blocks.iter().next().unwrap();
        assert_eq!((block.prompt, block.output, block.end), (0, Some(1), Some((3, 3))));
        assert_eq!(blocks.iter().count(), 2);
        assert_eq!(parser.screen().rows(0, 20).map(|r| r.trim_end().to_owned()).collect::<Vec<_>>(), ["$ ls", "one", "two", "$"]);
    }

    #[test]
    fn a_lone_escape_waits_for_the_next_chunk_but_other_sequences_pass() {
        let mut parser = vt100::Parser::new(2, 20, 10);
        let mut blocks = Blocks::new(10);
        blocks.feed(&mut parser, b"a\x1b");
        assert_eq!(blocks.held, b"\x1b");
        blocks.feed(&mut parser, b"[1mb\x1b]0;title\x07c");
        assert!(blocks.held.is_empty());
        assert_eq!(parser.screen().contents(), "abc");
        assert!(parser.screen().cell(0, 1).unwrap().bold());
        blocks.feed(&mut parser, b"\x1b]133;D;1");
        assert_eq!(blocks.held, b"\x1b]133;D;1");
        blocks.flush(&mut parser);
        assert!(blocks.held.is_empty());
    }

    #[test]
    fn prompts_stay_findable_while_history_fills_and_trims() {
        let (rows, limit) = (3, 5);
        let mut parser = vt100::Parser::new(rows, 10, limit);
        let mut blocks = Blocks::new(limit);
        for n in 0..12u32 {
            blocks.feed(&mut parser, format!("\x1b]133;A\x07p{n}\r\n\x1b]133;C\x07out{n}\r\n\x1b]133;D;{}\x07", n % 2).as_bytes());
        }
        assert_eq!(blocks.scrolled, 22, "two rows per command, the last two still in the grid");
        let oldest = blocks.scrolled - history_len(parser.screen_mut()) as u64;
        blocks.prune(oldest);
        let kept: Vec<_> = blocks.iter().map(|b| b.prompt).collect();
        assert!(kept.iter().all(|&p| p + 2 >= oldest), "{kept:?} against {oldest}");
        assert!(kept.len() >= 3);
        for &prompt in &kept[1..] {
            let n = (prompt / 2) as u32;
            assert_eq!(screen_top(&mut parser, &blocks, prompt), format!("p{n}"));
            assert_eq!(blocks.at_prompt(prompt).unwrap().status(), Some((n % 2) as i32));
        }
    }

    #[test]
    fn full_history_counts_wraps_feeds_and_scroll_ups() {
        let mut parser = vt100::Parser::new(3, 6, 2);
        let mut blocks = Blocks::new(2);
        blocks.feed(&mut parser, b"\r\n\r\n\r\n\r\n");
        assert_eq!(blocks.scrolled, 2);
        // Twelve cells wrap twice from the bottom row; the counter must say so.
        blocks.feed(&mut parser, b"abcdefghijkl");
        assert_eq!(blocks.scrolled, 3, "the seventh character wraps from the pending last column");
        blocks.feed(&mut parser, b"m");
        assert_eq!(blocks.scrolled, 4);
        blocks.feed(&mut parser, "中文中文".as_bytes());
        assert_eq!(blocks.scrolled, 5);
        blocks.feed(&mut parser, b"\x1b[2S");
        assert_eq!(blocks.scrolled, 7);
        blocks.feed(&mut parser, b"\x1bE\x1bD");
        assert_eq!(blocks.scrolled, 9);
        let mut plain = vt100::Parser::new(3, 6, 2);
        plain.process("\r\n\r\n\r\n\r\nabcdefghijklm中文中文\x1b[2S\x1bE\x1bD".as_bytes());
        assert_eq!(parser.screen().contents_formatted(), plain.screen().contents_formatted());
    }

    #[test]
    fn the_alternate_screen_scrolls_nothing_away() {
        let mut parser = vt100::Parser::new(2, 10, 1);
        let mut blocks = Blocks::new(1);
        blocks.feed(&mut parser, b"\r\n\r\n");
        assert_eq!(blocks.scrolled, 1);
        blocks.feed(&mut parser, b"\x1b[?1049h\r\n\r\n\r\nin app\x1b[?1049l\r\n");
        assert_eq!(blocks.scrolled, 2);
        assert!(!parser.screen().alternate_screen());
    }

    #[test]
    fn navigation_finds_neighbouring_prompts_and_prompts_do_not_duplicate() {
        let mut parser = vt100::Parser::new(2, 10, 50);
        let mut blocks = Blocks::new(50);
        blocks.feed(&mut parser, b"\x1b]133;A\x07\x1b]133;A\x07$ a\r\n\x1b]133;C\x07x\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ b\r\n\x1b]133;C\x07y\r\n\x1b]133;D\x07\x1b]133;A\x07$ ");
        let prompts: Vec<_> = blocks.iter().map(|b| b.prompt).collect();
        assert_eq!(prompts, [0, 2, 4]);
        assert_eq!(blocks.prompt_before(4), Some(2));
        assert_eq!(blocks.prompt_before(0), None);
        assert_eq!(blocks.prompt_after(0), Some(2));
        assert_eq!(blocks.prompt_after(4), None);
        assert_eq!(blocks.last_output(), Some((3, 4)));
        assert_eq!(blocks.covering(3).unwrap().prompt, 2);
        assert_eq!(blocks.top_line(parser.screen()), 3);
        assert_eq!(blocks.offset_for(2), 1);
        assert_eq!(blocks.offset_for(4), 0);
    }

    #[test]
    fn blocks_are_bounded() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        let mut blocks = Blocks::new(0);
        for _ in 0..(MAX_BLOCKS + 10) {
            blocks.feed(&mut parser, b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;0\x07");
        }
        assert_eq!(blocks.iter().count(), MAX_BLOCKS);
    }
}
