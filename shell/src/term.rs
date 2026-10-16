//! A terminal tab: a shell on a PTY, a VT parser, and the egui renderer for its grid.

use crate::theme::Theme;
use eframe::egui::{
    self, Color32, Event, FontId, Key, Modifiers, Pos2, Rect, Response, Stroke, TextFormat, Ui,
    Vec2,
    text::LayoutJob,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};
use blocks::Block;

mod wheel;
mod links;
mod search;
mod blocks;
mod integration;

const OUTPUT_CHUNK_BYTES: usize = 8192;
const OUTPUT_QUEUE_CHUNKS: usize = 32;
const PUMP_BYTES: usize = 32 * 1024;
const PUMP_TIME: Duration = Duration::from_millis(4);
/// Width of the strip at the left of the grid that carries the command marks.
pub const GUTTER: f32 = 10.0;

/// The shape of the terminal's cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

/// How the terminal is drawn and behaves: the user's choices, from the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TermLook {
    pub font_size: f32,
    /// Row height, as a multiple of the font's own.
    pub line_height: f32,
    pub cursor: CursorShape,
    pub cursor_blink: bool,
    /// Show bold text in the bright version of its colour.
    pub bold_bright: bool,
    /// Copy text as soon as it is selected.
    pub copy_on_select: bool,
}

pub struct TermTab {
    parser: vt100::Parser,
    blocks: blocks::Blocks,
    /// Output of one command, chosen from the gutter, as absolute lines.
    block_selection: Option<(u64, u64)>,
    search: search::Search,
    revision: u64,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    pending_output: Vec<u8>,
    pending_offset: usize,
    ctx: egui::Context,
    size: (u16, u16),
    wheel: wheel::WheelState,
    input_this_frame: bool,
    /// Anchor and head of the mouse selection, as (row, col) cells of the visible grid.
    selection: Option<((u16, u16), (u16, u16))>,
    pub exited: bool,
}

impl TermTab {
    /// Spawns `shell` as a login shell in `cwd`. `ctx` is woken whenever output arrives.
    pub fn spawn(shell: &str, cwd: &Path, ctx: egui::Context, scrollback: usize) -> anyhow::Result<TermTab> {
        Self::spawn_shell(shell, cwd, ctx, scrollback, &[])
    }

    /// As `spawn`, with more environment for the shell; tests point HOME elsewhere.
    fn spawn_shell(shell: &str, cwd: &Path, ctx: egui::Context, scrollback: usize, env: &[(&str, &std::ffi::OsStr)]) -> anyhow::Result<TermTab> {
        let mut cmd = CommandBuilder::new(shell);
        match integration::prepare(shell) {
            Some(integration) => integration.apply(&mut cmd),
            None => cmd.arg("-l"),
        }
        cmd.cwd(cwd);
        for (key, value) in env {
            cmd.env(key, value);
        }
        Self::spawn_with(cmd, ctx, scrollback)
    }

    /// Spawns a program with its arguments in `cwd`, with no shell between: the tab
    /// ends when the program does. `argv[0]` is the program.
    pub fn spawn_command(argv: &[String], cwd: &Path, ctx: egui::Context, scrollback: usize) -> anyhow::Result<TermTab> {
        let (program, args) = argv.split_first().ok_or_else(|| anyhow::anyhow!("nothing to run"))?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        Self::spawn_with(cmd, ctx, scrollback)
    }

    fn spawn_with(mut cmd: CommandBuilder, ctx: egui::Context, scrollback: usize) -> anyhow::Result<TermTab> {
        let (rows, cols) = (24, 80);
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "des-ui");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        // Blocking send propagates backpressure through the PTY instead of
        // allocating an ever-growing queue when the UI cannot keep up.
        let (tx, rx) = sync_channel(OUTPUT_QUEUE_CHUNKS);
        let reader_ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; OUTPUT_CHUNK_BYTES];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        reader_ctx.request_repaint();
                    }
                }
            }
            reader_ctx.request_repaint();
        });

        Ok(TermTab {
            parser: vt100::Parser::new(rows, cols, scrollback),
            blocks: blocks::Blocks::new(scrollback),
            block_selection: None,
            search: search::Search::default(),
            revision: 0,
            writer,
            master: pair.master,
            child,
            rx,
            pending_output: Vec::new(),
            pending_offset: 0,
            ctx,
            size: (rows, cols),
            wheel: wheel::WheelState::default(),
            input_this_frame: false,
            selection: None,
            exited: false,
        })
    }

    /// Feeds pending shell output into the parser and notices when the shell has exited.
    pub fn pump(&mut self) {
        let started = Instant::now();
        let mut consumed = 0;
        let mut chunks = 0;
        let mut drained = false;
        loop {
            if consumed >= PUMP_BYTES || chunks >= OUTPUT_QUEUE_CHUNKS || started.elapsed() >= PUMP_TIME {
                // Reader repaint requests may have coalesced into this frame.
                // Always schedule the next frame when we stop at a budget.
                self.ctx.request_repaint();
                break;
            }
            if self.pending_offset == self.pending_output.len() {
                match self.rx.try_recv() {
                    Ok(bytes) => {
                        self.pending_output = bytes;
                        self.pending_offset = 0;
                        chunks += 1;
                    }
                    Err(TryRecvError::Empty) => {
                        drained = true;
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.exited = true;
                        drained = true;
                        break;
                    }
                }
            }
            // Small slices let the time budget be checked even in large chunks.
            let end = (self.pending_offset + 4096)
                .min(self.pending_output.len())
                .min(self.pending_offset + PUMP_BYTES - consumed);
            self.blocks.feed(&mut self.parser, &self.pending_output[self.pending_offset..end]);
            self.revision = self.revision.wrapping_add(1);
            consumed += end - self.pending_offset;
            self.pending_offset = end;
        }
        // A sequence cut off at the end of a chunk has nothing more coming now.
        if drained {
            self.blocks.flush(&mut self.parser);
        }
        // Reap the child, but let reader EOF after draining every queued byte
        // close the tab. Child exit can precede delivery of its final output.
        let _ = self.child.try_wait();
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// User input returns to the live grid; protocol replies do not.
    pub fn input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() { return; }
        self.selection = None;
        self.block_selection = None;
        self.wheel.reset();
        self.input_this_frame = true;
        self.parser.screen_mut().set_scrollback(0);
        self.write(bytes);
    }

    /// Open the visible find bar from a toolbar or desktop action.
    pub fn open_search(&mut self) {
        self.search.open = true;
        self.search.focus = true;
    }

    /// Whether this pane owns a find field instead of terminal keyboard input.
    pub fn searching(&self) -> bool { self.search.open }

    /// Working directory of the shell, used by the file browser and for new tabs.
    pub fn cwd(&self) -> Option<PathBuf> {
        let pid = self.child.process_id()?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    /// Name of the program currently in the foreground of this terminal.
    pub fn foreground_process(&self) -> Option<String> {
        let pid = self.master.process_group_leader()?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        Some(comm.trim().to_string())
    }

    /// The command line of what is running in the foreground, word by word; read only
    /// to see where it went, never run.
    pub fn foreground_command_line(&self) -> Option<Vec<String>> {
        let pid = self.master.process_group_leader()?;
        let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let words: Vec<String> = bytes
            .split(|byte| *byte == 0)
            .filter(|word| !word.is_empty())
            .take(64)
            .map(|word| String::from_utf8_lossy(word).into_owned())
            .collect();
        (!words.is_empty()).then_some(words)
    }

    /// The selection with its ends in reading order.
    fn selection_range(&self) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = self.selection?;
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    fn selected_text(&mut self) -> Option<String> {
        if let Some((from, to)) = self.block_selection {
            return self.history_text(from, to);
        }
        let ((r0, c0), (r1, c1)) = self.selection_range()?;
        let text = self.parser.screen().contents_between(r0, c0, r1, c1 + 1);
        (!text.is_empty()).then_some(text)
    }

    /// Scrolls to the prompt before (or after) the one at the top of the view.
    pub fn jump_prompt(&mut self, previous: bool) {
        let top = self.blocks.top_line(self.parser.screen());
        let target = if previous { self.blocks.prompt_before(top) } else { self.blocks.prompt_after(top) };
        if let Some(line) = target {
            let offset = self.blocks.offset_for(line);
            self.selection = None;
            self.block_selection = None;
            self.wheel.reset();
            self.parser.screen_mut().set_scrollback(offset);
        }
    }

    /// Selects the output of the command whose prompt (or output) is on this
    /// visible row, so that Ctrl+Shift+C copies it.
    fn select_block_output(&mut self, row: u16) {
        let rows = u64::from(self.parser.screen().size().0);
        let line = self.blocks.top_line(self.parser.screen()) + u64::from(row);
        let block = self.blocks.at_prompt(line).or_else(|| self.blocks.covering(line));
        self.block_selection = block
            .and_then(Block::output_lines)
            .map(|(from, to)| (from, to.min(self.blocks.scrolled + rows)));
    }

    /// The output of the last command that finished, for a toolbar or palette action.
    pub fn copy_last_output(&mut self) -> Option<String> {
        let (from, to) = self.blocks.last_output()?;
        self.history_text(from, to)
    }

    /// Puts the last command's output on the clipboard; how many lines, when there was one.
    pub fn copy_last_output_to_clipboard(&mut self) -> Option<usize> {
        let text = self.copy_last_output()?;
        let lines = text.lines().count();
        self.ctx.copy_text(text);
        Some(lines)
    }

    /// The text of these absolute lines, as far as the history still holds them;
    /// soft-wrapped rows are joined.
    fn history_text(&mut self, from: u64, to: u64) -> Option<String> {
        const MAX_ROWS: u64 = 10_000;
        const MAX_BYTES: usize = 1 << 20;
        let scrolled = self.blocks.scrolled;
        let screen = self.parser.screen_mut();
        let (rows, cols) = screen.size();
        let saved = screen.scrollback();
        let retained = blocks::history_len(screen) as u64;
        let from = from.max(scrolled.saturating_sub(retained));
        let to = to.min(scrolled + u64::from(rows)).min(from.saturating_add(MAX_ROWS));
        let mut text = String::new();
        let mut line = from;
        'pages: while line < to && text.len() < MAX_BYTES {
            screen.set_scrollback(usize::try_from(scrolled.saturating_sub(line)).unwrap_or(usize::MAX));
            let top = scrolled.saturating_sub(screen.scrollback() as u64);
            if line < top {
                break;
            }
            let first = usize::try_from(line - top).unwrap_or(usize::MAX);
            if first >= usize::from(rows) {
                break;
            }
            let page: Vec<String> = screen.rows(0, cols).collect();
            for (row, contents) in page.iter().enumerate().skip(first) {
                if line >= to || text.len() >= MAX_BYTES {
                    break 'pages;
                }
                text.push_str(contents);
                if !screen.row_wrapped(row as u16) {
                    text.push('\n');
                }
                line += 1;
            }
        }
        screen.set_scrollback(saved);
        (!text.trim().is_empty()).then_some(text)
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        self.revision = self.revision.wrapping_add(1);
        self.parser.screen_mut().set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Translates this frame's input events into bytes for the shell.
    ///
    /// Ctrl+Shift+C copies the selection; plain Ctrl+C always interrupts.
    pub fn handle_events(
        &mut self,
        events: &[Event],
        alt_held: bool,
        shift_held: bool,
        ctx: &egui::Context,
    ) {
        // Keep the entire frame isolated, including events following Escape.
        let shortcut = events.iter().any(|event| matches!(event,
            Event::Key { key: Key::F, pressed: true, modifiers, .. } if modifiers.ctrl && modifiers.shift));
        if self.search.open || shortcut {
            if shortcut { self.search.open = true; self.search.focus = true; }
            let composing = self.search.composing || events.iter().any(|e| matches!(e, Event::Ime(_)));
            for event in events {
                if let Event::Ime(ime) = event {
                    match ime {
                        egui::ImeEvent::Preedit(text) => self.search.composing = !text.is_empty(),
                        egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => self.search.composing = false,
                        egui::ImeEvent::Enabled => {}
                    }
                }
            }
            for event in events {
                if let Event::Key { key, pressed: true, modifiers, .. } = event {
                    match key {
                        Key::Escape if !composing => self.search.open = false,
                        Key::Escape => { self.search.composing = false; self.search.focus = true; },
                        Key::Enter if !composing => { self.search.advance(modifiers.shift); self.search.focus = true; },
                        _ => {}
                    }
                }
            }
            return;
        }
        let app_cursor = self.parser.screen().application_cursor();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut out: Vec<u8> = Vec::new();
        for event in events {
            match event {
                Event::Text(text) => {
                    if alt_held {
                        out.push(0x1b);
                    }
                    out.extend_from_slice(text.as_bytes());
                }
                Event::Paste(text) => {
                    if bracketed {
                        out.extend_from_slice(b"\x1b[200~");
                        out.extend_from_slice(text.as_bytes());
                        out.extend_from_slice(b"\x1b[201~");
                    } else {
                        out.extend_from_slice(text.replace('\n', "\r").as_bytes());
                    }
                }
                // egui turns Ctrl+C / Ctrl+X into clipboard commands; the shell wants them.
                Event::Copy => match self.selected_text() {
                    Some(text) if shift_held => ctx.copy_text(text),
                    _ => out.push(0x03),
                },
                Event::Cut => out.push(0x18),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    // Ctrl+Shift+Up and Down walk the prompts; they are the terminal's.
                    if modifiers.ctrl && modifiers.shift && matches!(key, Key::ArrowUp | Key::ArrowDown) {
                        self.jump_prompt(*key == Key::ArrowUp);
                        continue;
                    }
                    if let Some(bytes) = key_bytes(*key, *modifiers, app_cursor) {
                        out.extend_from_slice(&bytes);
                    }
                }
                _ => {}
            }
        }
        if !out.is_empty() {
            self.input(&out);
        }
    }

    /// Draws the terminal grid into `rect`, resizing the PTY to fit. `response` is the
    /// interaction result for `rect`, used for scrolling and mouse selection.
    /// `focused` is false for a pane that is in view but does not have the keyboard.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        theme: &Theme,
        response: &Response,
        look: &TermLook,
        focused: bool,
    ) {
        let mut rect = rect;
        if self.search.open {
            let bar_height = if rect.width() < 420.0 { 96.0 } else if rect.width() < 700.0 { 64.0 } else { 34.0 };
            let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bar_height));
            rect.min.y = (rect.min.y + bar_height + 2.0).min(rect.max.y - 1.0);
            ui.painter().rect_filled(bar, 0.0, crate::ui::PANEL);
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink(4.0)), |ui| {
                ui.set_clip_rect(bar);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("FIND").font(crate::ui::display(12.0)).color(crate::ui::SECONDARY));
                    let field = ui.add(egui::TextEdit::singleline(&mut self.search.query)
                        .hint_text("Search scrollback").char_limit(4096).font(crate::ui::mono(12.0))
                        .desired_width((bar.width() - 320.0).max(60.0)));
                    if self.search.focus { field.request_focus(); self.search.focus = false; }
                    if field.changed() { self.search.restart(self.parser.screen()); self.search.revision = self.revision; }
                    let total = self.search.matches.len();
                    ui.label(egui::RichText::new(format!("{}/{}{}", if total == 0 { 0 } else { self.search.current + 1 }, total, if self.search.truncated { "+ · refine search" } else if self.search.pending() { "…" } else { "" })).font(crate::ui::mono(12.0)));
                    if ui.button("Previous").on_hover_text("Shift+Enter").clicked() { self.search.advance(true); }
                    if ui.button("Next").on_hover_text("Enter").clicked() { self.search.advance(false); }
                    if ui.add_enabled(total > 0, egui::Button::new("Copy")).clicked() {
                        ui.ctx().copy_text(self.search.matches[self.search.current].text.clone());
                    }
                    if ui.button("Clear").clicked() { self.search.query.clear(); self.search.restart(self.parser.screen()); self.search.focus = true; }
                    if ui.button("Close").on_hover_text("Escape").clicked() { self.search.open = false; self.search.composing = false; response.request_focus(); }
                });
            });
            if self.search.revision != self.revision && !self.search.pending() {
                self.search.restart(self.parser.screen()); self.search.revision = self.revision;
            }
            self.search.scan();
            if self.search.pending() { ui.ctx().request_repaint(); }
            if self.search.dirty {
                if let Some(found) = self.search.matches.get(self.search.current) {
                    self.parser.screen_mut().set_scrollback(self.search.history.saturating_sub(found.cells[0].0));
                }
                self.search.dirty = false;
            }
        }
        let gutter = Rect::from_min_size(rect.min, Vec2::new(GUTTER, rect.height()));
        rect.min.x = (rect.min.x + GUTTER).min(rect.max.x - 1.0);
        let font = FontId::monospace(look.font_size);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let (cw, natural) = ui.fonts(|f| (cell_width(f, &font, pixels_per_point), f.row_height(&font)));
        let ch = (natural * look.line_height).round().max(1.0);
        // Text sits in the middle of a taller or shorter row.
        let inset = (ch - natural) / 2.0;
        let cols = ((rect.width() / cw).floor() as u16).max(2);
        let rows = ((rect.height() / ch).floor() as u16).max(1);
        self.resize(rows, cols);

        let cell_at = |pos: Pos2| {
            let col = ((pos.x - rect.min.x) / cw).floor().clamp(0.0, (cols - 1) as f32);
            let row = ((pos.y - rect.min.y) / ch).floor().clamp(0.0, (rows - 1) as f32);
            (row as u16, col as u16)
        };
        let pointer = response.interact_pointer_pos();
        if response.drag_started() {
            self.selection = ui.input(|i| i.pointer.press_origin()).filter(|p| rect.contains(*p))
                .map(|p| (cell_at(p), cell_at(pointer.unwrap_or(p))));
        } else if response.dragged() {
            if let (Some(p), Some((anchor, _))) = (pointer, self.selection) {
                self.selection = Some((anchor, cell_at(p)));
            }
        } else if response.clicked() {
            self.selection = None;
            self.block_selection = None;
            // A click on a prompt's mark takes that command's output.
            if let Some(pos) = pointer.filter(|p| gutter.contains(*p)) {
                let row = ((pos.y - rect.min.y) / ch).floor().clamp(0.0, (rows - 1) as f32) as u16;
                self.select_block_output(row);
            }
        }
        if response.hovered() {
            if let Some(pos) = ui.ctx().pointer_hover_pos().filter(|p| gutter.contains(*p)) {
                let row = ((pos.y - rect.min.y) / ch).floor().clamp(0.0, (rows - 1) as f32) as u16;
                let line = self.blocks.top_line(self.parser.screen()) + u64::from(row);
                if let Some(block) = self.blocks.at_prompt(line).or_else(|| self.blocks.covering(line)) {
                    let text = match block.status() {
                        Some(0) => "Finished without error\nClick to select its output".to_string(),
                        Some(status) => format!("Exit status {status}\nClick to select its output"),
                        None => "Running".to_string(),
                    };
                    response.clone().on_hover_text(text);
                }
            }
        }
        // Modifier-click opens links; ordinary dragging continues to select text.
        if response.hovered() {
            if let Some(pos) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) {
                let (row, col) = cell_at(pos);
                if let Some(url) = links::at(self.parser.screen(), row, col) {
                    response.clone().on_hover_text(format!("Ctrl+click to open\n{url}"));
                    if ui.input(|i| i.modifiers.ctrl) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if response.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                    }
                }
            }
        }
        if response.drag_stopped() && look.copy_on_select {
            if let Some(text) = self.selected_text() {
                ui.ctx().copy_text(text);
            }
        }

        let typed = std::mem::take(&mut self.input_this_frame);
        if response.hovered() && !typed && !self.search.open {
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                let cell = cell_at(pos);
                for event in ui.input(|input| input.events.clone()) {
                    if !matches!(event, Event::MouseWheel { .. }) { continue; }
                    let before = self.parser.screen().scrollback();
                    let bytes = self.wheel.handle(self.parser.screen_mut(), &event, cell, ch, rows);
                    if !bytes.is_empty() || self.parser.screen().scrollback() != before {
                        self.selection = None;
                    }
                    if !bytes.is_empty() { self.write(&bytes); }
                }
            }
        }

        let mut painter = ui.painter_at(rect);
        // At the start of a session the text comes on after the frame around it.
        let shown = crate::powerup::stage(ui.ctx()).alpha(crate::powerup::Part::Terminal);
        if shown < 1.0 {
            painter.set_opacity(shown);
        }
        painter.rect_filled(rect, 0.0, crate::ui::GROUND);
        // Backgrounds and selections use terminal cells, not font glyph advances.
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = self.parser.screen().cell(row, col) else { continue };
                if cell.is_wide_continuation() { continue; }
                let style = CellStyle::of(cell, theme, look.bold_bright);
                if style.bg != Color32::TRANSPARENT {
                    let width = if cell.is_wide() { 2.0 * cw } else { cw };
                    let min = rect.min + Vec2::new(col as f32 * cw, row as f32 * ch);
                    painter.rect_filled(Rect::from_min_size(min, Vec2::new(width, ch)), 0.0, style.bg);
                }
            }
        }
        if let Some(((r0, c0), (r1, c1))) = self.selection_range() {
            for row in r0..=r1.min(rows - 1) {
                let from = if row == r0 { c0 } else { 0 };
                let to = if row == r1 { c1 + 1 } else { cols };
                let min = rect.min + Vec2::new(from as f32 * cw, row as f32 * ch);
                let size = Vec2::new(to.saturating_sub(from) as f32 * cw, ch);
                painter.rect_filled(Rect::from_min_size(min, size), 0.0, theme.selection());
            }
        }

        // The marks in the gutter: a bar at each prompt, a square where a command
        // ended, in the caution colour when it failed; and a chosen output.
        let retained = blocks::history_len(self.parser.screen_mut()) as u64;
        self.blocks.prune(self.blocks.scrolled.saturating_sub(retained));
        let top = self.blocks.top_line(self.parser.screen());
        let visible = |line: u64| (line >= top && line < top + u64::from(rows)).then(|| (line - top) as f32 * ch);
        if let Some((from, to)) = self.block_selection {
            for line in from.max(top)..to.min(top + u64::from(rows)) {
                let min = Pos2::new(rect.min.x, rect.min.y + (line - top) as f32 * ch);
                painter.rect_filled(Rect::from_min_size(min, Vec2::new(rect.width(), ch)), 0.0, theme.selection());
            }
        }
        let gutter_painter = ui.painter_at(gutter);
        gutter_painter.rect_filled(gutter, 0.0, crate::ui::GROUND);
        for block in self.blocks.iter() {
            let failed = block.status().is_some_and(|status| status != 0);
            if let Some(y) = visible(block.prompt) {
                let color = if failed { crate::ui::CAUTION } else { theme.main };
                let bar = Rect::from_min_size(Pos2::new(gutter.min.x + 2.0, gutter.min.y + y + 2.0), Vec2::new(2.0, (ch - 4.0).max(2.0)));
                gutter_painter.rect_filled(bar, 0.0, color);
            }
            if let Some(y) = block.badge_line().and_then(visible) {
                let color = if failed { crate::ui::CAUTION } else { crate::ui::LABEL };
                let badge = Rect::from_center_size(Pos2::new(gutter.min.x + 7.0, gutter.min.y + y + ch / 2.0), Vec2::splat(4.0));
                gutter_painter.rect_filled(badge, 0.0, color);
            }
        }

        if self.search.open {
            if let Some(found) = self.search.matches.get(self.search.current) {
                let top = self.search.history.saturating_sub(self.parser.screen().scrollback());
                for &(row, col, width) in &found.cells {
                    if row >= top && row < top + rows as usize {
                        let min = rect.min + Vec2::new(col as f32 * cw, (row - top) as f32 * ch);
                        painter.rect_filled(Rect::from_min_size(min, Vec2::new(width as f32 * cw, ch)), 0.0, theme.selection());
                    }
                }
            }
        }
        let screen = self.parser.screen();

        // A blinking cursor is on for half of each second.
        let lit = !look.cursor_blink || !focused || ui.input(|i| i.time).rem_euclid(1.0) < 0.5;
        if look.cursor_blink && focused {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        let cursor = (!screen.hide_cursor() && screen.scrollback() == 0 && lit).then(|| {
            let (crow, ccol) = screen.cursor_position();
            let crow = crow.min(rows - 1);
            let mut ccol = ccol.min(cols - 1);
            if screen.cell(crow, ccol).is_some_and(|cell| cell.is_wide_continuation()) {
                ccol = ccol.saturating_sub(1);
            }
            let width = if screen.cell(crow, ccol).is_some_and(vt100::Cell::is_wide) { 2.0 * cw } else { cw };
            let cell = Rect::from_min_size(rect.min + Vec2::new(ccol as f32 * cw, crow as f32 * ch), Vec2::new(width, ch));
            (crow, ccol, cell)
        });

        for row in 0..rows {
            let job = ui.fonts(|fonts| row_job(fonts, screen, row, cols, theme, look, &font, cw, pixels_per_point));
            let galley = ui.fonts(|f| f.layout_job(job));
            let pos = Pos2::new(rect.min.x, rect.min.y + row as f32 * ch + inset);
            painter.galley(pos, galley, theme.term_fg);
        }

        if let Some((_, _, cell)) = cursor {
            let shape = match look.cursor {
                CursorShape::Block => cell,
                CursorShape::Bar => Rect::from_min_size(cell.min, Vec2::new(2.0, ch)),
                CursorShape::Underline => Rect::from_min_max(Pos2::new(cell.left(), cell.bottom() - 2.0), cell.max),
            };
            if focused {
                painter.rect_filled(shape, 0.0, theme.cursor);
            } else {
                // Without the keyboard, the cursor is an outline.
                painter.rect_stroke(cell, 0.0, Stroke::new(1.0, theme.cursor), egui::StrokeKind::Inside);
            }
        }

        // Under a block cursor the character is drawn again, dark on the cursor's
        // colour, so that it stays readable.
        if let (Some((crow, ccol, cell)), CursorShape::Block, true) = (cursor, look.cursor, focused) {
            if let Some(under) = screen.cell(crow, ccol).filter(|under| under.has_contents()) {
                painter.text(
                    Pos2::new(cell.left(), cell.top() + inset),
                    egui::Align2::LEFT_TOP,
                    under.contents(),
                    font.clone(),
                    crate::ui::GROUND,
                );
            }
        }
    }
}

fn cell_width(fonts: &egui::text::Fonts, font: &FontId, pixels_per_point: f32) -> f32 {
    // Match the renderer's per-glyph pixel rounding before sizing the PTY/grid.
    (fonts.glyph_width(font, 'M') * pixels_per_point).round().max(1.0) / pixels_per_point
}

fn row_job(fonts: &egui::text::Fonts, screen: &vt100::Screen, row: u16, cols: u16, theme: &Theme, look: &TermLook, font: &FontId, cw: f32, pixels_per_point: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut pen = 0.0;
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else { continue };
        if cell.is_wide_continuation() { continue; }
        let text = if cell.has_contents() { cell.contents() } else { " " };
        let style = CellStyle::of(cell, theme, look.bold_bright);
        let mut format = style.format(font);
        format.background = Color32::TRANSPARENT;
        // A section per VT cell prevents kerning/fallback widths from moving the
        // following column. Combining characters stay together inside their cell.
        let mut chars = text.chars();
        let first = chars.next().unwrap();
        let end = if chars.next().is_none() {
            // Most cells are one glyph: use its cached advance without allocating
            // a measurement galley for every space or ASCII character.
            fonts.glyph_width(font, first)
        } else {
            let glyphs = fonts.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
            glyphs.rows.first().and_then(|row| row.glyphs.last())
                .map_or(0.0, |glyph| glyph.pos.x + glyph.advance_width)
        };
        let advance = (end * pixels_per_point).round() / pixels_per_point;
        let target = col as f32 * cw;
        job.append(text, target - pen, format);
        pen = target + advance;
    }
    job
}

impl Drop for TermTab {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: Color32,
    bg: Color32,
    underline: bool,
    italic: bool,
}

impl CellStyle {
    fn of(cell: &vt100::Cell, theme: &Theme, bold_bright: bool) -> CellStyle {
        let resolve = |color: vt100::Color, default: Color32, bold: bool| match color {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) if bold && bold_bright && i < 8 => theme.indexed(i + 8),
            vt100::Color::Idx(i) => theme.indexed(i),
            vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        };
        let mut fg = resolve(cell.fgcolor(), theme.term_fg, cell.bold());
        let mut bg = resolve(cell.bgcolor(), Color32::TRANSPARENT, false);
        if cell.inverse() {
            let opaque_bg = if bg == Color32::TRANSPARENT {
                crate::ui::GROUND
            } else {
                bg
            };
            bg = fg;
            fg = opaque_bg;
        }
        if cell.dim() {
            fg = fg.gamma_multiply(0.6);
        }
        CellStyle {
            fg,
            bg,
            underline: cell.underline(),
            italic: cell.italic(),
        }
    }

    fn format(self, font: &FontId) -> TextFormat {
        TextFormat {
            font_id: font.clone(),
            color: self.fg,
            background: self.bg,
            italics: self.italic,
            underline: if self.underline {
                Stroke::new(1.0, self.fg)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    }
}

/// Encodes a non-text key press the way xterm does.
fn key_bytes(key: Key, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    // xterm's modifier parameter: 1, plus 1 for Shift, 2 for Alt, 4 for Ctrl.
    let modifier = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    // Arrows, Home and End: a letter, in one of three forms.
    let cursor = |c: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}")
        } else if app_cursor {
            format!("\x1bO{c}")
        } else {
            format!("\x1b[{c}")
        }
    };
    // Keys named by a number, such as Delete (3) and F5 (15).
    let numbered = |n: u8| {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~")
        } else {
            format!("\x1b[{n}~")
        }
    };
    let function = |c: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}")
        } else {
            format!("\x1bO{c}")
        }
    };
    let s: String = match key {
        // Shift+Enter and Alt+Enter ask for a new line without submitting, in programs
        // that tell the two apart.
        Key::Enter if m.shift || m.alt => "\x1b\r".into(),
        Key::Enter => "\r".into(),
        Key::Tab if m.shift => "\x1b[Z".into(),
        Key::Tab => "\t".into(),
        Key::Backspace if m.alt => "\x1b\x7f".into(),
        Key::Backspace if m.ctrl => "\x17".into(),
        Key::Backspace => "\x7f".into(),
        Key::Escape => "\x1b".into(),
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::Insert => numbered(2),
        Key::Delete => numbered(3),
        Key::PageUp => numbered(5),
        Key::PageDown => numbered(6),
        Key::F1 => function('P'),
        Key::F2 => function('Q'),
        Key::F3 => function('R'),
        Key::F4 => function('S'),
        Key::F5 => numbered(15),
        Key::F6 => numbered(17),
        Key::F7 => numbered(18),
        Key::F8 => numbered(19),
        Key::F9 => numbered(20),
        Key::F10 => numbered(21),
        Key::F11 => numbered(23),
        Key::F12 => numbered(24),
        _ if m.ctrl => {
            let byte = match key {
                Key::Space => 0x00,
                Key::OpenBracket => 0x1b,
                Key::Backslash => 0x1c,
                Key::CloseBracket => 0x1d,
                _ => {
                    let name = key.name();
                    let c = name.bytes().next()?;
                    if name.len() != 1 || !c.is_ascii_alphabetic() {
                        return None;
                    }
                    c.to_ascii_uppercase() & 0x1f
                }
            };
            // Alt on top of Ctrl is an escape prefix, as for plain text.
            return Some(if m.alt { vec![0x1b, byte] } else { vec![byte] });
        }
        _ => return None,
    };
    Some(s.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn grid_check(text: &str, size: f32, scale: f32) {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        ctx.set_pixels_per_point(scale);
        let mut parser = vt100::Parser::new(2, 160, 0);
        parser.process(text.as_bytes());
        let cursor_col = parser.screen().cursor_position().1;
        parser.process(b"|");
        let font = FontId::monospace(size);
        let look = TermLook { font_size: size, line_height: 1.0, cursor: CursorShape::Block,
            cursor_blink: false, bold_bright: true, copy_on_select: false };
        let theme = Theme::load("signal");
        let _ = ctx.run(Default::default(), |ctx| {
            let pixels_per_point = ctx.pixels_per_point();
            assert!((pixels_per_point - scale).abs() < 0.001);
            ctx.fonts(|fonts| {
                let cw = cell_width(fonts, &font, pixels_per_point);
                let galley = fonts.layout_job(row_job(fonts, parser.screen(), 0, 160, &theme, &look, &font, cw, pixels_per_point));
                let marker = galley.rows[0].glyphs.iter().find(|glyph| glyph.chr == '|').unwrap();
                let cursor_x = cursor_col as f32 * cw;
                assert!((marker.pos.x - cursor_x).abs() <= 0.51 / pixels_per_point,
                    "text drift at font {size}, scale {scale}: marker x {}, cursor x {cursor_x}, cell {cw}", marker.pos.x);
            });
        });
    }

    #[test]
    fn grid_aligns_long_ascii_input_with_cursor_at_display_scales() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for size in [14.0, 16.0, 18.0] { grid_check(&"M".repeat(100), size, scale); }
        }
    }

    #[test]
    fn grid_preserves_columns_after_wide_and_combining_characters() {
        for scale in [1.0, 1.25, 2.0] {
            grid_check("prompt 中文 🚀 e\u{301} tail ", 14.0, scale);
            grid_check("\x1b[31mred\x1b[0m ⚙ \u{e0b0} next ", 16.0, scale);
        }
    }

    fn frame(tab: &mut TermTab, ctx: &egui::Context, rect: Rect, events: Vec<Event>, focused: bool, smooth: f32) -> bool {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))), events, ..Default::default() };
        let look = TermLook { font_size: 14.0, line_height: 1.0, cursor: CursorShape::Block,
            cursor_blink: false, bold_bright: true, copy_on_select: false };
        let theme = Theme::load("signal");
        let mut hovered = false;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.interact(rect, egui::Id::new("test-terminal"), egui::Sense::click_and_drag());
                hovered = response.hovered();
                tab.handle_events(&ui.input(|i| i.events.clone()), false, false, ctx);
                // Simulate another widget consuming scroll, or a leftover smooth tail.
                ui.input_mut(|input| input.smooth_scroll_delta.y = smooth);
                tab.show(ui, rect, &theme, &response, &look, focused);
            });
        });
        hovered
    }

    fn terminal_frame_setup() -> (TermTab, egui::Context, Rect, Pos2) {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let _ = ctx.run(Default::default(), |_| {});
        let (cw, ch) = ctx.fonts(|fonts| {
            let font = FontId::monospace(14.0);
            (cell_width(fonts, &font, 1.0), fonts.row_height(&font).round())
        });
        let rect = Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(GUTTER + cw * 40.0, ch * 3.0));
        let pos = rect.min + Vec2::new(GUTTER + cw * 2.5, ch * 1.5);
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), ctx.clone(), 100).unwrap();
        tab.resize(3, 40);
        tab.parser.process(b"first\r\nsecond\r\nthird\r\nfourth\r\nfifth\r\nsixth\r\n");
        for _ in 0..2 { frame(&mut tab, &ctx, rect, vec![Event::PointerMoved(pos)], false, 0.0); }
        (tab, ctx, rect, pos)
    }

    #[test]
    fn dragging_keeps_the_original_press_cell() {
        let (mut tab, ctx, rect, _) = terminal_frame_setup();
        tab.parser.process(b"\x1b[2J\x1b[Hhttps://example.org");
        let start = rect.min + Vec2::new(GUTTER + 1.0, 5.0);
        let end = start + Vec2::new(90.0, 0.0);
        frame(&mut tab, &ctx, rect, vec![Event::PointerMoved(start), Event::PointerButton {
            pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE,
        }], true, 0.0);
        frame(&mut tab, &ctx, rect, vec![Event::PointerMoved(end)], true, 0.0);
        assert_eq!(tab.selection.unwrap().0, (0, 0));
        assert!(tab.selected_text().unwrap().starts_with("https://"));
    }

    #[test]
    fn ctrl_click_emits_url_and_plain_click_does_not() {
        let (mut tab, ctx, rect, _) = terminal_frame_setup();
        tab.parser.process(b"\x1b[2J\x1b[Hhttps://example.org");
        let pos = rect.min + Vec2::new(GUTTER + 30.0, 5.0);
        let look = TermLook { font_size: 14.0, line_height: 1.0, cursor: CursorShape::Block,
            cursor_blink: false, bold_bright: true, copy_on_select: false };
        let theme = Theme::load("signal");
        for ctrl in [false, true] {
            let modifiers = Modifiers { ctrl, ..Modifiers::NONE };
            for pressed in [true, false] {
                let input = egui::RawInput { modifiers,
                    events: vec![Event::PointerMoved(pos), Event::PointerButton {
                        pos, button: egui::PointerButton::Primary, pressed, modifiers }],
                    ..Default::default() };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.interact(rect, egui::Id::new("test-terminal"), egui::Sense::click_and_drag());
                        tab.show(ui, rect, &theme, &response, &look, true);
                    });
                });
                let urls: Vec<_> = output.platform_output.commands.iter().filter_map(|command| {
                    if let egui::OutputCommand::OpenUrl(url) = command { Some(url.url.as_str()) } else { None }
                }).collect();
                if ctrl && !pressed {
                    assert_eq!(urls, vec!["https://example.org"]);
                } else { assert!(urls.is_empty()); }
            }
        }
    }

    #[test]
    fn wheel_scrolls_hovered_unfocused_pane_even_after_smooth_delta_is_consumed() {
        let (mut tab, ctx, rect, pos) = terminal_frame_setup();
        let events = vec![Event::PointerMoved(pos), Event::MouseWheel { unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 1.0), modifiers: Modifiers::NONE }];
        assert!(frame(&mut tab, &ctx, rect, events, false, 0.0));
        assert_eq!(tab.parser.screen().scrollback(), 1);
        let events = vec![Event::PointerMoved(Pos2::new(10.0, 10.0)), Event::MouseWheel { unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 1.0), modifiers: Modifiers::NONE }];
        assert!(!frame(&mut tab, &ctx, rect, events, false, 0.0));
        assert_eq!(tab.parser.screen().scrollback(), 1);
    }

    #[test]
    fn typing_returns_to_live_grid_and_cannot_be_undone_by_scroll_tail() {
        let (mut tab, ctx, rect, pos) = terminal_frame_setup();
        tab.parser.screen_mut().set_scrollback(2);
        frame(&mut tab, &ctx, rect, vec![Event::PointerMoved(pos), Event::Text("x".into())], true, 80.0);
        assert_eq!(tab.parser.screen().scrollback(), 0);
        frame(&mut tab, &ctx, rect, vec![], true, 60.0);
        assert_eq!(tab.parser.screen().scrollback(), 0);
        let events = vec![Event::PointerMoved(pos), Event::MouseWheel { unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 1.0), modifiers: Modifiers::NONE }];
        frame(&mut tab, &ctx, rect, events, true, 0.0);
        assert_eq!(tab.parser.screen().scrollback(), 0);
    }

    #[test]
    fn hovered_application_receives_wheel_with_actual_grid_coordinates() {
        use std::sync::{Arc, Mutex};
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let (mut tab, ctx, rect, pos) = terminal_frame_setup();
        tab.parser.process(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
        let written = Arc::new(Mutex::new(Vec::new()));
        tab.writer = Box::new(Capture(written.clone()));
        let events = vec![Event::PointerMoved(pos), Event::MouseWheel { unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 1.0), modifiers: Modifiers::NONE }];
        assert!(frame(&mut tab, &ctx, rect, events, false, 0.0));
        assert_eq!(&*written.lock().unwrap(), b"\x1b[<64;3;2M");
    }

    #[test]
    fn noisy_output_yields_each_frame_and_keeps_order() {
        let ctx = egui::Context::default();
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), ctx.clone(), 100).unwrap();
        // Control the input boundary while retaining a real PTY and parser.
        let (tx, rx) = std::sync::mpsc::channel();
        tab.rx = rx;
        let mut expected = vt100::Parser::new(24, 80, 100);
        for i in 0..32 {
            let mut bytes = vec![b'a' + i % 26; 8192];
            bytes.extend_from_slice(format!("\x1b[Hchunk-{i:02}").as_bytes());
            expected.process(&bytes);
            tx.send(bytes).unwrap();
        }
        drop(tx);
        let _ = ctx.run(Default::default(), |_| tab.pump());
        assert!(!tab.exited, "one frame drained the entire noisy output queue");
        assert_ne!(tab.parser.screen().contents(), expected.screen().contents());
        assert!(ctx.has_requested_repaint(), "pending output must schedule another frame");
        for _ in 0..100 {
            tab.pump();
            if tab.exited {
                break;
            }
        }
        assert!(tab.exited, "output never finished draining");
        assert_eq!(tab.parser.screen().contents_formatted(), expected.screen().contents_formatted());
    }

    #[test]
    fn stalled_ui_backpressures_noisy_shell() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("done");
        let mut tab = TermTab::spawn("/bin/sh", dir.path(), egui::Context::default(), 0).unwrap();
        tab.write(b"dd if=/dev/zero bs=8192 count=512 2>/dev/null; printf done > done\r");
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            assert!(!done.exists(), "shell wrote 4 MiB while UI consumed no output");
            std::thread::sleep(Duration::from_millis(5));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done.exists() {
            tab.pump();
            assert!(Instant::now() < deadline, "backpressure never released");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(std::fs::read(done).unwrap(), b"done");
    }

    #[test]
    fn pump_limits_parser_bytes_even_for_one_large_chunk() {
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), egui::Context::default(), 0).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        tab.rx = rx;
        tx.send(vec![b'x'; 128 * 1024]).unwrap();
        tab.pump();
        assert!(tab.pending_offset > 0);
        assert!(tab.pending_offset <= 32 * 1024, "parser exceeded the frame byte budget");
        assert!(tab.pending_offset < tab.pending_output.len(), "partial chunk was discarded");
    }

    #[test]
    fn exited_child_does_not_discard_buffered_output() {
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), egui::Context::default(), 0).unwrap();
        tab.write(b"exit\r");
        let deadline = Instant::now() + Duration::from_secs(5);
        while tab.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "child did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        tab.rx = rx;
        let mut expected = vt100::Parser::new(24, 80, 0);
        for i in 0..16 {
            let mut bytes = vec![b'x'; 8192];
            bytes.extend_from_slice(format!("\x1b[Hfinal-{i:02}").as_bytes());
            expected.process(&bytes);
            tx.send(bytes).unwrap();
        }
        drop(tx);
        tab.pump();
        assert!(!tab.exited, "tab closed with buffered output still unread");
        for _ in 0..100 {
            tab.pump();
            if tab.exited {
                break;
            }
        }
        assert!(tab.exited, "drained terminal did not finish");
        assert_eq!(tab.parser.screen().contents_formatted(), expected.screen().contents_formatted());
    }

    #[test]
    fn ime_composition_blocks_navigation_across_frames() {
        let (mut tab, ctx, _, _) = terminal_frame_setup();
        tab.open_search();
        tab.search.query = "i".into();
        tab.search.restart(tab.parser.screen());
        while tab.search.pending() { tab.search.scan(); }
        assert!(tab.search.matches.len() > 1);
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        tab.handle_events(&[Event::Ime(egui::ImeEvent::Preedit("文".into()))], false, false, &ctx);
        tab.handle_events(&[enter.clone()], false, false, &ctx);
        assert_eq!(tab.search.current, 0);
        tab.handle_events(&[Event::Ime(egui::ImeEvent::Commit("文".into())), enter.clone()], false, false, &ctx);
        assert_eq!(tab.search.current, 0);
        tab.handle_events(&[enter], false, false, &ctx);
        assert_eq!(tab.search.current, 1);
    }

    #[test]
    fn find_field_edits_unicode_and_keeps_focus_after_enter() {
        let (mut tab, ctx, rect, _) = terminal_frame_setup();
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        frame(&mut tab, &ctx, rect, vec![key(Key::F, Modifiers::CTRL | Modifiers::SHIFT)], true, 0.0);
        frame(&mut tab, &ctx, rect, vec![Event::Text("first".into())], true, 0.0);
        assert_eq!(tab.search.query, "first");
        assert_eq!(tab.search.matches.len(), 1);
        frame(&mut tab, &ctx, rect, vec![key(Key::Enter, Modifiers::NONE)], true, 0.0);
        frame(&mut tab, &ctx, rect, vec![Event::Text("文".into())], true, 0.0);
        assert_eq!(tab.search.query, "first文");
        assert!(tab.search.matches.is_empty());
    }

    #[test]
    fn search_isolates_shortcut_text_enter_escape_and_ime_from_pty() {
        use std::sync::{Arc, Mutex};
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let ctx = egui::Context::default();
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), ctx.clone(), 100).unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        tab.writer = Box::new(Capture(written.clone()));
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        tab.handle_events(&[key(Key::F, Modifiers::CTRL | Modifiers::SHIFT), Event::Text("needle".into()), Event::Ime(egui::ImeEvent::Commit("文".into())), key(Key::Enter, Modifiers::NONE)], false, false, &ctx);
        assert!(tab.searching());
        tab.handle_events(&[key(Key::Escape, Modifiers::NONE), Event::Text("same-frame".into())], false, false, &ctx);
        assert!(!tab.searching());
        assert!(written.lock().unwrap().is_empty());
        tab.handle_events(&[Event::Text("live".into())], false, false, &ctx);
        assert_eq!(&*written.lock().unwrap(), b"live");
    }

    #[test]
    fn encodes_keys() {
        let none = Modifiers::NONE;
        assert_eq!(key_bytes(Key::Enter, none, false).unwrap(), b"\r");
        assert_eq!(key_bytes(Key::ArrowUp, none, false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(Key::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(key_bytes(Key::D, Modifiers::CTRL, false).unwrap(), [0x04]);
        assert_eq!(key_bytes(Key::A, none, false), None);
        assert_eq!(key_bytes(Key::Enter, Modifiers::SHIFT, false).unwrap(), b"\x1b\r");
        assert_eq!(key_bytes(Key::ArrowLeft, Modifiers::ALT, true).unwrap(), b"\x1b[1;3D");
        assert_eq!(key_bytes(Key::ArrowLeft, Modifiers::CTRL, false).unwrap(), b"\x1b[1;5D");
        assert_eq!(key_bytes(Key::ArrowUp, Modifiers::SHIFT, false).unwrap(), b"\x1b[1;2A");
        assert_eq!(key_bytes(Key::Delete, Modifiers::CTRL, false).unwrap(), b"\x1b[3;5~");
        assert_eq!(key_bytes(Key::F5, none, false).unwrap(), b"\x1b[15~");
        assert_eq!(key_bytes(Key::F1, Modifiers::SHIFT, false).unwrap(), b"\x1b[1;2P");
        assert_eq!(key_bytes(Key::B, Modifiers::CTRL | Modifiers::ALT, false).unwrap(), [0x1b, 0x02]);
    }

    #[test]
    fn shell_output_reaches_the_grid() {
        let ctx = egui::Context::default();
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), ctx, 100).unwrap();
        assert_eq!(tab.cwd(), Some(PathBuf::from("/tmp")));
        tab.write(b"echo he''llo-pty\r");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            tab.pump();
            if tab.parser.screen().contents().contains("hello-pty") {
                break;
            }
            assert!(Instant::now() < deadline, "no output from shell");
            std::thread::sleep(Duration::from_millis(20));
        }
        let (_, cols) = tab.parser.screen().size();
        // The prompt may or may not precede the output, depending on timing.
        let (row, col) = tab
            .parser
            .screen()
            .rows(0, cols)
            .enumerate()
            .find_map(|(row, line)| Some((row as u16, line.find("hello-pty")? as u16)))
            .unwrap();
        tab.selection = Some(((row, col + 8), (row, col)));
        assert_eq!(tab.selected_text().as_deref(), Some("hello-pty"));

        tab.write(b"exit\r");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !tab.exited {
            tab.pump();
            assert!(Instant::now() < deadline, "shell did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn blocks_frame_setup() -> (TermTab, egui::Context, Rect) {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let _ = ctx.run(Default::default(), |_| {});
        let (cw, ch) = ctx.fonts(|fonts| {
            let font = FontId::monospace(14.0);
            (cell_width(fonts, &font, 1.0), fonts.row_height(&font).round())
        });
        let rect = Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(GUTTER + cw * 40.0, ch * 3.0));
        let mut tab = TermTab::spawn("/bin/sh", Path::new("/tmp"), ctx.clone(), 100).unwrap();
        tab.resize(3, 40);
        // egui attributes a press to the widgets of the frame before it.
        let away = Pos2::new(10.0, 10.0);
        for _ in 0..2 { frame(&mut tab, &ctx, rect, vec![Event::PointerMoved(away)], false, 0.0); }
        (tab, ctx, rect)
    }

    fn top_row(tab: &TermTab) -> String {
        tab.parser.screen().rows(0, 40).next().unwrap_or_default()
    }

    #[test]
    fn ctrl_shift_arrows_walk_the_prompts_without_reaching_the_pty() {
        use std::sync::{Arc, Mutex};
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let (mut tab, ctx, rect) = blocks_frame_setup();
        let written = Arc::new(Mutex::new(Vec::new()));
        tab.writer = Box::new(Capture(written.clone()));
        for n in 0..3 {
            tab.blocks.feed(&mut tab.parser, format!("\x1b]133;A\x07$ cmd{n}\r\n\x1b]133;C\x07out{n}\r\n\x1b]133;D;{n}\x07").as_bytes());
        }
        tab.blocks.feed(&mut tab.parser, b"\x1b]133;A\x07$ ");
        assert!(top_row(&tab).starts_with("$ cmd2"));
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let walk = Modifiers::CTRL | Modifiers::SHIFT;
        frame(&mut tab, &ctx, rect, vec![key(Key::ArrowUp, walk)], true, 0.0);
        assert!(top_row(&tab).starts_with("$ cmd1"), "{:?}", top_row(&tab));
        frame(&mut tab, &ctx, rect, vec![key(Key::ArrowUp, walk)], true, 0.0);
        assert!(top_row(&tab).starts_with("$ cmd0"));
        frame(&mut tab, &ctx, rect, vec![key(Key::ArrowUp, walk)], true, 0.0);
        assert!(top_row(&tab).starts_with("$ cmd0"), "the first prompt stays put");
        frame(&mut tab, &ctx, rect, vec![key(Key::ArrowDown, walk)], true, 0.0);
        assert!(top_row(&tab).starts_with("$ cmd1"));
        assert!(written.lock().unwrap().is_empty(), "prompt walking reached the shell");
        frame(&mut tab, &ctx, rect, vec![key(Key::ArrowUp, Modifiers::SHIFT)], true, 0.0);
        assert_eq!(&*written.lock().unwrap(), b"\x1b[1;2A");
        assert_eq!(tab.parser.screen().scrollback(), 0, "typing returns to the live grid");
    }

    #[test]
    fn a_click_on_a_prompts_mark_selects_its_output_and_the_last_output_copies() {
        let (mut tab, ctx, rect) = blocks_frame_setup();
        tab.blocks.feed(&mut tab.parser, b"\x1b]133;A\x07$ cmd0\r\n\x1b]133;C\x07alpha\r\nbeta\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ cmd1\r\n\x1b]133;C\x07gamma\r\n\x1b]133;D;2\x07\x1b]133;A\x07$ ");
        assert_eq!(tab.copy_last_output().as_deref(), Some("gamma\n"));
        assert_eq!(tab.blocks.at_prompt(3).unwrap().status(), Some(2));
        let click = |tab: &mut TermTab, pos: Pos2| {
            let press = Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
            let release = Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
            frame(tab, &ctx, rect, vec![Event::PointerMoved(pos), press], true, 0.0);
            frame(tab, &ctx, rect, vec![release], true, 0.0);
        };
        // The gutter beside the top row, which shows the second prompt.
        click(&mut tab, rect.min + Vec2::new(3.0, 5.0));
        assert_eq!(tab.block_selection, Some((4, 5)));
        assert_eq!(tab.selected_text().as_deref(), Some("gamma\n"));
        // A click in the grid lets it go; the first command's output spans two lines.
        click(&mut tab, rect.min + Vec2::new(GUTTER + 30.0, 5.0));
        assert_eq!(tab.block_selection, None);
        tab.jump_prompt(true);
        tab.jump_prompt(true);
        assert!(top_row(&tab).starts_with("$ cmd0"));
        click(&mut tab, rect.min + Vec2::new(3.0, 5.0));
        assert_eq!(tab.selected_text().as_deref(), Some("alpha\nbeta\n"));
        tab.input(b"x");
        assert_eq!(tab.block_selection, None);
    }

    #[test]
    fn bash_marks_its_prompts_and_exit_statuses() {
        let bash = Path::new("/usr/bin/bash");
        if !bash.exists() || std::env::var_os(integration::OPT_OUT).is_some_and(|v| v == "0") {
            eprintln!("bash integration is not checked here");
            return;
        }
        // An empty home keeps this machine's own shell files out of the check.
        let home = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut tab = TermTab::spawn_shell("/usr/bin/bash", home.path(), ctx, 1000, &[("HOME", home.path().as_os_str())]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let wait = |tab: &mut TermTab, done: &dyn Fn(&TermTab) -> bool, what: &str| loop {
            tab.pump();
            if done(tab) { break; }
            assert!(Instant::now() < deadline, "{what}: {:?}", tab.parser.screen().contents());
            std::thread::sleep(Duration::from_millis(20));
        };
        wait(&mut tab, &|t| t.blocks.iter().next().is_some(), "no prompt mark from bash");
        tab.write(b"false\r");
        wait(&mut tab, &|t| t.blocks.iter().any(|b| b.status() == Some(1)), "no exit status for false");
        tab.write(b"echo out-marker\r");
        wait(&mut tab, &|t| t.blocks.iter().filter(|b| b.end.is_some()).count() >= 2, "echo did not finish");
        let output = tab.copy_last_output().unwrap_or_default();
        assert!(output.contains("out-marker") && !output.contains("echo"), "{output:?}");
        tab.write(b"exit\r");
        wait(&mut tab, &|t| t.exited, "bash did not exit");
    }
}
