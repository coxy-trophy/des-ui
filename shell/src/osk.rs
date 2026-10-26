//! The on-screen keyboard: keys sized for a finger along the foot of the shell, above
//! the status strip. A press becomes the same `egui` events a real key would, pushed
//! into the next frame's input, so the terminal and every text field read it as
//! typing. It types only into the shell; other applications would need the
//! compositor's virtual-keyboard protocol.

use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Event, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// Height of one key, and of one row; tall enough for a fingertip.
pub const KEY: f32 = 44.0;
const GAP: f32 = 4.0;
const ROWS: usize = 5;
const MARGIN: f32 = 8.0;

/// What a key does when pressed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Cap {
    /// A character, and the one it gives with Shift.
    Char(char, char),
    /// A key with a name rather than a character.
    Named(Key, &'static str),
    Space,
    Shift,
    Ctrl,
    Alt,
    /// Between the letters and the symbols.
    Layer,
}

impl Cap {
    /// How wide, in key widths.
    fn span(self) -> f32 {
        match self {
            Cap::Space => 5.0,
            Cap::Named(Key::Backspace | Key::Enter | Key::Tab, _) | Cap::Shift => 1.5,
            _ => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Layer {
    Letters,
    Symbols,
}

const fn c(plain: char, shifted: char) -> Cap {
    Cap::Char(plain, shifted)
}

const LETTERS: [&[Cap]; ROWS] = [
    &[c('1', '!'), c('2', '@'), c('3', '#'), c('4', '$'), c('5', '%'), c('6', '^'), c('7', '&'), c('8', '*'), c('9', '('), c('0', ')'), Cap::Named(Key::Backspace, "BKSP")],
    &[Cap::Named(Key::Tab, "TAB"), c('q', 'Q'), c('w', 'W'), c('e', 'E'), c('r', 'R'), c('t', 'T'), c('y', 'Y'), c('u', 'U'), c('i', 'I'), c('o', 'O'), c('p', 'P')],
    &[Cap::Named(Key::Escape, "ESC"), c('a', 'A'), c('s', 'S'), c('d', 'D'), c('f', 'F'), c('g', 'G'), c('h', 'H'), c('j', 'J'), c('k', 'K'), c('l', 'L'), Cap::Named(Key::Enter, "ENTER")],
    &[Cap::Shift, c('z', 'Z'), c('x', 'X'), c('c', 'C'), c('v', 'V'), c('b', 'B'), c('n', 'N'), c('m', 'M'), c(',', '<'), c('.', '>'), c('/', '?'), Cap::Named(Key::ArrowUp, "↑")],
    &[Cap::Ctrl, Cap::Alt, Cap::Layer, Cap::Space, Cap::Named(Key::ArrowLeft, "←"), Cap::Named(Key::ArrowDown, "↓"), Cap::Named(Key::ArrowRight, "→")],
];

const SYMBOLS: [&[Cap]; ROWS] = [
    &[c('1', '!'), c('2', '@'), c('3', '#'), c('4', '$'), c('5', '%'), c('6', '^'), c('7', '&'), c('8', '*'), c('9', '('), c('0', ')'), Cap::Named(Key::Backspace, "BKSP")],
    &[Cap::Named(Key::Tab, "TAB"), c('`', '~'), c('-', '_'), c('=', '+'), c('[', '{'), c(']', '}'), c('\\', '|'), c(';', ':'), c('\'', '"'), c('<', '<'), c('>', '>')],
    &[Cap::Named(Key::Escape, "ESC"), c('!', '!'), c('@', '@'), c('#', '#'), c('$', '$'), c('%', '%'), c('^', '^'), c('&', '&'), c('*', '*'), c('|', '|'), Cap::Named(Key::Enter, "ENTER")],
    &[Cap::Shift, c('(', '('), c(')', ')'), c('{', '{'), c('}', '}'), c('"', '"'), c('~', '~'), c('_', '_'), c('+', '+'), c('?', '?'), c(':', ':'), Cap::Named(Key::ArrowUp, "↑")],
    &[Cap::Ctrl, Cap::Alt, Cap::Layer, Cap::Space, Cap::Named(Key::ArrowLeft, "←"), Cap::Named(Key::ArrowDown, "↓"), Cap::Named(Key::ArrowRight, "→")],
];

pub struct Osk {
    pub open: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    layer: Layer,
    /// What the last presses produced, for the start of the next frame.
    queued: Vec<Event>,
    /// The text field that had the keyboard before a key was touched.
    last_focus: Option<Id>,
    refocus: Option<Id>,
}

impl Default for Osk {
    fn default() -> Self {
        Osk { open: false, shift: false, ctrl: false, alt: false, layer: Layer::Letters, queued: Vec::new(), last_focus: None, refocus: None }
    }
}

/// Fit every row to the actual card, including narrow framed workspaces.
fn key_layout(rect: Rect, rows: [&[Cap]; ROWS]) -> Vec<(usize, usize, Cap, Rect)> {
    let inset = MARGIN.min(rect.width()/8.0).min(rect.height()/8.0);
    let inner = rect.shrink(inset);
    let gap = GAP.min(inner.width()/40.0).min(inner.height()/12.0);
    let key_height = ((inner.height()-(ROWS-1) as f32*gap)/ROWS as f32).clamp(1.0,KEY);
    let unit = rows.iter().map(|row| {
        let spans: f32 = row.iter().map(|cap|cap.span()).sum();
        (inner.width()-(row.len()-1) as f32*gap)/spans
    }).fold(KEY*2.0,f32::min).max(1.0);
    let mut keys = Vec::new();
    for (r,row) in rows.iter().enumerate() {
        let width: f32 = row.iter().map(|cap|cap.span()*unit).sum::<f32>()+(row.len()-1) as f32*gap;
        let mut x = inner.center().x-width/2.0;
        for (k,&cap) in row.iter().enumerate() {
            let key = Rect::from_min_size(pos2(x,inner.top()+r as f32*(key_height+gap)),vec2(cap.span()*unit,key_height));
            x += key.width()+gap;
            keys.push((r,k,cap,key));
        }
    }
    keys
}

impl Osk {
    /// The room it takes at the foot of the shell, the same whatever is latched; 0 shut.
    pub fn height(&self) -> f32 {
        if self.open { ROWS as f32 * KEY + (ROWS - 1) as f32 * GAP + 2.0 * MARGIN } else { 0.0 }
    }

    /// Keep a usable workspace above the keyboard on shorter displays.
    pub fn height_for(&self, viewport_height: f32) -> f32 {
        self.height().min((viewport_height * 0.4).max(0.0))
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: false, command: self.ctrl }
    }

    /// The events one press stands for. Shift, Ctrl and Alt hold for the next key only.
    fn press(&mut self, cap: Cap) -> Vec<Event> {
        let key_event = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let events = match cap {
            Cap::Shift => { self.shift = !self.shift; return Vec::new(); }
            Cap::Ctrl => { self.ctrl = !self.ctrl; return Vec::new(); }
            Cap::Alt => { self.alt = !self.alt; return Vec::new(); }
            Cap::Layer => {
                self.layer = if self.layer == Layer::Letters { Layer::Symbols } else { Layer::Letters };
                return Vec::new();
            }
            Cap::Named(key, _) => vec![key_event(key, self.modifiers())],
            Cap::Space if self.ctrl || self.alt => vec![key_event(Key::Space, self.modifiers())],
            Cap::Space => vec![Event::Text(" ".into())],
            Cap::Char(plain, shifted) if self.ctrl || self.alt => {
                // A chord is a key, as Ctrl+C is; the terminal turns it into its byte.
                match Key::from_name(&plain.to_string()) {
                    Some(key) => vec![key_event(key, self.modifiers())],
                    None => vec![Event::Text(if self.shift { shifted } else { plain }.to_string())],
                }
            }
            Cap::Char(plain, shifted) => vec![Event::Text(if self.shift { shifted } else { plain }.to_string())],
        };
        self.shift = false;
        self.ctrl = false;
        self.alt = false;
        events
    }

    /// Call first in each frame, before anything reads input: hands over what was
    /// pressed, giving the keyboard back to the field that had it.
    pub fn inject(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.refocus.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        if !self.queued.is_empty() {
            let events = std::mem::take(&mut self.queued);
            ctx.input_mut(|input| input.events.extend(events));
        }
        if let Some(id) = ctx.memory(|memory| memory.focused()) {
            self.last_focus = Some(id);
        }
    }

    /// Draws the keys into `rect`, the strip `height()` tall that the parent set aside.
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect) {
        if !self.open {
            return;
        }
        let painter = ui.painter_at(rect);
        crate::material::paint(ui.ctx(), &painter, rect, des_common::frame::RADIUS, crate::material::Role::Panel);
        painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0, ui::LINE));
        let rows = if self.layer == Layer::Letters { LETTERS } else { SYMBOLS };
        let mut pressed = None;
        for (r,k,cap,key_rect) in key_layout(rect, rows) {
            let key_height = key_rect.height();
            // Not focusable: touching a key must not take the keyboard from the field.
            let response = ui.interact(key_rect, ui.id().with(("osk", r, k)), Sense::CLICK);
            let lit = match cap {
                Cap::Shift => self.shift,
                Cap::Ctrl => self.ctrl,
                Cap::Alt => self.alt,
                Cap::Layer => self.layer == Layer::Symbols,
                _ => false,
            };
            let (fill, ink) = if lit {
                (theme.main, ui::GROUND)
            } else if response.is_pointer_button_down_on() {
                (theme.tint(), ui::TEXT)
            } else {
                (ui::GROUND, ui::TEXT)
            };
            painter.rect_filled(key_rect, ui::CONTROL_RADIUS, fill);
            let edge = if response.hovered() { ui::SECONDARY } else { ui::LINE_STRONG };
            painter.rect_stroke(key_rect, ui::CONTROL_RADIUS, Stroke::new(1.0, edge), StrokeKind::Inside);
            let label = self.label(cap);
            let mut font = if label.chars().count() > 1 { ui::display(12.0_f32.min(key_height * 0.45)) } else { ui::mono(17.0_f32.min(key_height * 0.45)) };
            let mut galley = painter.layout_no_wrap(label.clone(), font.clone(), ink);
            let available = (key_rect.width()-4.0_f32.min(key_rect.width()*0.2)).max(1.0);
            if galley.size().x > available {
                font.size *= available/galley.size().x;
                galley = painter.layout_no_wrap(label, font, ink);
            }
            painter.galley(key_rect.center()-galley.size()/2.0, galley, ink);
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, lit, self.spoken(cap)));
            if response.clicked() {
                pressed = Some(cap);
            }
        }
        if let Some(cap) = pressed {
            let events = self.press(cap);
            if !events.is_empty() {
                self.queued.extend(events);
                self.refocus = self.last_focus;
                ui.ctx().request_repaint();
            }
        }
    }

    fn label(&self, cap: Cap) -> String {
        match cap {
            Cap::Char(plain, shifted) => if self.shift { shifted } else { plain }.to_string(),
            Cap::Named(_, name) => name.to_string(),
            Cap::Space => String::new(),
            Cap::Shift => "SHIFT".into(),
            Cap::Ctrl => "CTRL".into(),
            Cap::Alt => "ALT".into(),
            Cap::Layer => if self.layer == Layer::Letters { "123" } else { "ABC" }.into(),
        }
    }

    /// What a screen reader says for a key.
    fn spoken(&self, cap: Cap) -> String {
        match cap {
            Cap::Named(key, _) => key.name().to_string(),
            Cap::Space => "Space".into(),
            Cap::Layer => if self.layer == Layer::Letters { "Symbols" } else { "Letters" }.into(),
            _ => self.label(cap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(events: &[Event]) -> Vec<String> {
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.clone()) } else { None }).collect()
    }

    #[test]
    fn letters_type_and_shift_holds_for_one_key() {
        let mut osk = Osk::default();
        assert_eq!(text(&osk.press(c('a', 'A'))), ["a"]);
        osk.press(Cap::Shift);
        assert_eq!(text(&osk.press(c('a', 'A'))), ["A"]);
        assert_eq!(text(&osk.press(c('a', 'A'))), ["a"], "shift latches for one key only");
        osk.press(Cap::Shift);
        assert_eq!(text(&osk.press(c('1', '!'))), ["!"]);
        assert_eq!(text(&osk.press(Cap::Space)), [" "]);
    }

    #[test]
    fn ctrl_c_is_a_key_with_its_modifiers() {
        let mut osk = Osk::default();
        osk.press(Cap::Ctrl);
        let events = osk.press(c('c', 'C'));
        assert!(matches!(events.as_slice(), [Event::Key { key: Key::C, pressed: true, modifiers, .. }] if modifiers.ctrl && !modifiers.shift));
        assert!(!osk.ctrl, "ctrl is released after the chord");
        assert_eq!(text(&osk.press(c('c', 'C'))), ["c"]);
    }

    #[test]
    fn named_keys_carry_latched_modifiers() {
        let mut osk = Osk::default();
        assert!(matches!(osk.press(Cap::Named(Key::Enter, "ENTER")).as_slice(), [Event::Key { key: Key::Enter, modifiers, .. }] if modifiers.is_none()));
        osk.press(Cap::Shift);
        osk.press(Cap::Alt);
        assert!(matches!(osk.press(Cap::Named(Key::Tab, "TAB")).as_slice(), [Event::Key { key: Key::Tab, modifiers, .. }] if modifiers.shift && modifiers.alt));
        assert!(!osk.shift && !osk.alt);
    }

    #[test]
    fn modifiers_and_layers_produce_nothing_by_themselves() {
        let mut osk = Osk::default();
        for cap in [Cap::Shift, Cap::Ctrl, Cap::Alt, Cap::Layer] {
            assert!(osk.press(cap).is_empty());
        }
        assert_eq!(osk.layer, Layer::Symbols);
        osk.press(Cap::Layer);
        assert_eq!(osk.layer, Layer::Letters);
        // Pressing a latch twice lets it go.
        osk.press(Cap::Shift);
        assert!(!osk.shift && osk.ctrl && osk.alt);
    }

    #[test]
    fn keys_fit_their_card_at_small_and_wide_frame_sizes() {
        for size in [vec2(64.0,66.0),vec2(280.0,160.0),vec2(640.0,252.0),vec2(1280.0,252.0)] {
            let rect = Rect::from_min_size(pos2(20.0,400.0),size);
            for rows in [LETTERS,SYMBOLS] {
                let keys = key_layout(rect,rows);
                assert_eq!(keys.len(),rows.iter().map(|row|row.len()).sum::<usize>());
                for (_,_,_,key) in keys { assert!(rect.contains_rect(key), "{key:?} outside {rect:?}"); }
            }
        }
    }

    #[test]
    fn height_is_stable_and_zero_when_shut() {
        let mut osk = Osk::default();
        assert_eq!(osk.height(), 0.0);
        osk.open = true;
        let open = osk.height();
        assert!(open >= ROWS as f32 * KEY);
        osk.press(Cap::Shift);
        osk.press(Cap::Layer);
        assert_eq!(osk.height(), open);
        assert!(LETTERS.iter().chain(SYMBOLS.iter()).all(|row| !row.is_empty()));
    }

    #[test]
    fn pressing_a_key_reaches_the_next_frame_as_input() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let mut osk = Osk { open: true, ..Osk::default() };
        let theme = Theme::load("signal");
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let strip = Rect::from_min_max(pos2(0.0, 600.0 - osk.height()), screen.max);
        let run = |osk: &mut Osk, events: Vec<Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut seen = Vec::new();
            let _ = ctx.run(input, |ctx| {
                osk.inject(ctx);
                seen = ctx.input(|i| i.events.clone());
                egui::CentralPanel::default().show(ctx, |ui| osk.show(ui, &theme, strip));
            });
            seen
        };
        run(&mut osk, Vec::new());
        // Find the q key by asking where the second row's second key is drawn.
        let inner = strip.shrink(MARGIN);
        let widest: f32 = LETTERS.iter().map(|row| row.iter().map(|cap| cap.span()).sum::<f32>()).fold(0.0, f32::max);
        let unit = ((inner.width() - (widest - 1.0) * GAP) / widest).clamp(KEY * 0.8, KEY * 2.0);
        let row = LETTERS[1];
        let width: f32 = row.iter().map(|cap| cap.span() * unit).sum::<f32>() + (row.len() as f32 - 1.0) * GAP;
        let x = inner.center().x - width / 2.0 + row[0].span() * unit + GAP + unit / 2.0;
        let q = pos2(x, inner.top() + KEY + GAP + KEY / 2.0);
        let click = |pressed| Event::PointerButton { pos: q, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        run(&mut osk, vec![Event::PointerMoved(q), click(true)]);
        run(&mut osk, vec![click(false)]);
        let seen = run(&mut osk, Vec::new());
        assert!(seen.iter().any(|e| matches!(e, Event::Text(t) if t == "q")), "{seen:?}");
    }
}
