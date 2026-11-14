//! The calculator: a small card in the workspace, drawn by the shell itself, so it
//! costs no process of its own. Arithmetic is worked out by the launcher's own
//! evaluator (`commands::evaluate_explained`); this keeps what is typed, applies the
//! one-touch functions, and remembers the last few answers.
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, Key, Layout, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Answers kept below the keys.
const HISTORY: usize = 5;
/// Characters an expression may grow to.
const LONGEST: usize = 160;

/// A key on the calculator, or its keyboard equivalent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Press {
    Digit(char),
    Dot,
    /// `+ - * / ^`
    Op(char),
    Open,
    Close,
    Equals,
    Back,
    Clear,
    Negate,
    Percent,
    Sqrt,
    Square,
    Inverse,
}

#[derive(Default)]
pub struct Calculator {
    pub open: bool,
    /// What is typed, with ASCII operators (`*`, `/`, `-`); shown with `×`, `÷`, `−`.
    entry: String,
    /// The entry is an answer: a digit starts afresh, an operator carries on from it.
    answered: bool,
    /// Why the last `=` gave no answer.
    error: Option<&'static str>,
    /// Expression and answer, newest first.
    history: VecDeque<(String, String)>,
    copied: Option<Instant>,
}

impl Calculator {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// What the entry works out to now, for the line under it.
    pub fn preview(&self) -> Option<String> {
        if self.entry.is_empty() || self.answered { return None; }
        crate::commands::evaluate_explained(&closed(&self.entry)).ok().map(format)
    }

    /// What is shown large: the answer, or what is being typed.
    pub fn shown(&self) -> String {
        if self.entry.is_empty() { "0".into() } else { pretty(&self.entry) }
    }

    pub fn press(&mut self, press: Press) {
        self.error = None;
        match press {
            Press::Digit(_) | Press::Dot | Press::Open if self.answered => {
                self.entry.clear();
                self.answered = false;
                self.press(press);
            }
            Press::Digit(digit) => self.push(digit),
            Press::Dot => {
                // One point to a number.
                let number = trailing_number(&self.entry);
                if !self.entry[number..].contains('.') {
                    if number == self.entry.len() { self.push('0'); }
                    self.push('.');
                }
            }
            Press::Op(op) => {
                self.answered = false;
                // A second operator replaces the first; a minus may start a number.
                if let Some(last) = self.entry.chars().last().filter(|c| "+-*/^".contains(*c)) {
                    if !(op == '-' && last != '-') {
                        self.entry.pop();
                    }
                } else if self.entry.is_empty() && op != '-' {
                    self.entry.push('0');
                }
                self.push(op);
            }
            Press::Open => self.push('('),
            Press::Close => {
                let depth = self.entry.matches('(').count() as i64 - self.entry.matches(')').count() as i64;
                if depth > 0 { self.push(')'); }
            }
            Press::Back => {
                self.answered = false;
                if self.entry.ends_with("sqrt(") { self.entry.truncate(self.entry.len() - 5); } else { self.entry.pop(); }
            }
            Press::Clear => {
                self.entry.clear();
                self.answered = false;
            }
            Press::Negate => {
                // `(-n)` unwraps back to `n`.
                if let Some(inner) = self.entry.strip_suffix(')') {
                    let at = trailing_number(inner);
                    if at < inner.len() && inner[..at].ends_with("(-") {
                        self.entry = format!("{}{}", &inner[..at - 2], &inner[at..]);
                    }
                    return;
                }
                let at = trailing_number(&self.entry);
                if at == self.entry.len() { return; }
                let number = self.entry[at..].to_string();
                self.entry.truncate(at);
                if let Some(stripped) = number.strip_prefix('-') {
                    self.entry.push_str(stripped);
                } else if self.entry.is_empty() && self.answered {
                    self.entry.push('-');
                    self.entry.push_str(&number);
                } else {
                    self.entry.push_str("(-");
                    self.entry.push_str(&number);
                    self.entry.push(')');
                }
            }
            Press::Percent => {
                // The number just typed, as a part of a hundred.
                let at = trailing_number(&self.entry);
                if let Ok(value) = self.entry[at..].parse::<f64>() {
                    self.entry.truncate(at);
                    self.entry.push_str(&format(value / 100.0));
                }
            }
            Press::Sqrt => self.apply(|e| format!("sqrt({e})")),
            Press::Square => self.apply(|e| format!("({e})^2")),
            Press::Inverse => self.apply(|e| format!("1/({e})")),
            Press::Equals => self.equals(),
        }
    }

    /// A function of the whole entry, worked out at once.
    fn apply(&mut self, wrap: impl Fn(&str) -> String) {
        if self.entry.is_empty() { return; }
        self.entry = wrap(&self.entry);
        self.answered = false;
        self.equals();
    }

    fn equals(&mut self) {
        if self.entry.is_empty() || self.answered { return; }
        let expression = closed(&self.entry);
        match crate::commands::evaluate_explained(&expression) {
            Ok(value) => {
                let answer = format(value);
                self.history.push_front((pretty(&expression), answer.clone()));
                self.history.truncate(HISTORY);
                self.entry = answer;
                self.answered = true;
            }
            Err(why) => self.error = Some(why),
        }
    }

    fn push(&mut self, c: char) {
        if self.entry.len() < LONGEST { self.entry.push(c); }
    }

    /// The answer worth copying: the last one, or what the entry works out to.
    fn answer(&self) -> Option<String> {
        if self.answered { Some(self.entry.clone()) } else { self.preview() }
    }

    /// The keys typed this frame, as presses. Escape clears, then closes.
    fn keyboard(&mut self, ui: &Ui) {
        let events = ui.input(|input| input.events.clone());
        let mut presses = Vec::new();
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    for c in text.chars() {
                        presses.push(match c {
                            '0'..='9' => Press::Digit(c),
                            '.' | ',' => Press::Dot,
                            '+' | '-' | '*' | '/' | '^' => Press::Op(c),
                            'x' | 'X' | '×' => Press::Op('*'),
                            '÷' => Press::Op('/'),
                            '(' => Press::Open,
                            ')' => Press::Close,
                            '%' => Press::Percent,
                            '=' => Press::Equals,
                            _ => continue,
                        });
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.command => match key {
                    Key::Enter => presses.push(Press::Equals),
                    Key::Backspace => presses.push(Press::Back),
                    Key::Delete => presses.push(Press::Clear),
                    Key::Escape => {
                        if self.entry.is_empty() { self.open = false; } else { presses.push(Press::Clear); }
                    }
                    _ => {}
                },
                egui::Event::Copy => self.copy(ui.ctx()),
                _ => {}
            }
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::C)) { self.copy(ui.ctx()); }
        for press in presses { self.press(press); }
    }

    fn copy(&mut self, ctx: &egui::Context) {
        if let Some(answer) = self.answer() {
            ctx.copy_text(answer);
            self.copied = Some(Instant::now());
        }
    }

    /// Draws the card in the middle of `rect`.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        self.keyboard(ui);
        if !self.open { return; }
        const KEY: egui::Vec2 = egui::Vec2 { x: 60.0, y: 46.0 };
        const GAP: f32 = 7.0;
        let width = 5.0 * KEY.x + 4.0 * GAP + 2.0 * 22.0;
        let history_height = if self.history.is_empty() { 0.0 } else { 34.0 + self.history.len() as f32 * 26.0 };
        let height = 36.0 + 30.0 + 14.0 + 104.0 + 18.0 + 5.0 * KEY.y + 4.0 * GAP + 18.0 + history_height + 12.0;
        let card = Rect::from_center_size(rect.center(), vec2(width, height.min(rect.height() - 8.0)));
        let radius = 18.0;
        let mut shapes = vec![egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(120) }.as_shape(card, radius).into()];
        shapes.extend(crate::material::shapes(ui.ctx(), card, radius, crate::material::Role::Panel));
        shapes.push(egui::Shape::rect_stroke(card, radius, Stroke::new(1.0, ui::LINE), StrokeKind::Inside));
        ui.painter().extend(shapes);
        let inner = card.shrink2(vec2(22.0, 18.0));
        let mut ui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        ui.set_clip_rect(card.intersect(ui.clip_rect()));

        // The head: the name, and Done.
        ui.allocate_ui_with_layout(vec2(inner.width(), 30.0), Layout::left_to_right(Align::Center), |ui| {
            ui::tracked(ui, "CALCULATOR", ui::display_bold(15.0), ui::TEXT, 3.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui::button(ui, theme, "DONE", Some("ESC"), false).clicked() { self.open = false; }
            });
        });
        ui.add_space(14.0);

        // The display: what is typed, and its answer or why there is none.
        let (screen, _) = ui.allocate_exact_size(vec2(inner.width(), 104.0), Sense::hover());
        ui.painter().rect_filled(screen, 12.0, ui::GROUND.gamma_multiply(0.7));
        ui.painter().rect_stroke(screen, 12.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let line = match (&self.error, self.preview(), self.answered) {
            (Some(why), _, _) => (why.to_string(), ui::CAUTION),
            (None, Some(preview), _) => (format!("= {preview}"), ui::SECONDARY),
            (None, None, true) => (self.history.front().map(|(e, _)| e.clone()).unwrap_or_default(), ui::LABEL),
            _ => (String::new(), ui::LABEL),
        };
        let painter = ui.painter();
        painter.text(screen.right_top() + vec2(-14.0, 14.0), Align2::RIGHT_TOP, &line.0, ui::mono(13.0), line.1);
        let shown = self.shown();
        let size = if shown.chars().count() > 14 { 26.0 } else { 40.0 };
        let galley = painter.layout_no_wrap(shown, ui::display_bold(size), ui::TEXT);
        let at = pos2((screen.right() - 14.0 - galley.size().x).max(screen.left() + 14.0), screen.bottom() - 12.0 - galley.size().y);
        painter.with_clip_rect(screen.shrink(4.0)).galley(at, galley, ui::TEXT);
        // Copy the answer, from the display's corner.
        let copy = Rect::from_min_size(screen.left_top() + vec2(10.0, 10.0), vec2(26.0, 26.0));
        let response = ui.interact(copy, ui.id().with("calculator-copy"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, self.answer().is_some(), "Copy the answer"));
        let lit = self.copied.is_some_and(|at| at.elapsed() < Duration::from_millis(1200));
        ui::paint_icon(ui.painter(), copy.shrink(5.0), ui::Icon::Copy, if lit { theme.main } else if response.hovered() { ui::TEXT } else { ui::LABEL }, false);
        if response.on_hover_text(if lit { "Copied" } else { "Copy the answer · Ctrl+C" }).clicked() { self.copy(ui.ctx()); }
        if lit { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
        ui.add_space(18.0);

        // The keys.
        const KEYS: [[(&str, Press); 5]; 5] = [
            [("C", Press::Clear), ("(", Press::Open), (")", Press::Close), ("%", Press::Percent), ("÷", Press::Op('/'))],
            [("√", Press::Sqrt), ("7", Press::Digit('7')), ("8", Press::Digit('8')), ("9", Press::Digit('9')), ("×", Press::Op('*'))],
            [("x²", Press::Square), ("4", Press::Digit('4')), ("5", Press::Digit('5')), ("6", Press::Digit('6')), ("−", Press::Op('-'))],
            [("1/x", Press::Inverse), ("1", Press::Digit('1')), ("2", Press::Digit('2')), ("3", Press::Digit('3')), ("+", Press::Op('+'))],
            [("±", Press::Negate), ("0", Press::Digit('0')), (".", Press::Dot), ("DEL", Press::Back), ("=", Press::Equals)],
        ];
        let top = ui.cursor().top();
        let mut pressed = None;
        for (row, keys) in KEYS.iter().enumerate() {
            for (column, (label, press)) in keys.iter().enumerate() {
                let place = Rect::from_min_size(pos2(inner.left() + column as f32 * (KEY.x + GAP), top + row as f32 * (KEY.y + GAP)), KEY);
                let response = ui.interact(place, ui.id().with(("calculator-key", row, column)), Sense::click());
                let spoken = match label { &"√" => "Square root", &"x²" => "Square", &"1/x" => "One over", &"±" => "Change sign", &"DEL" => "Delete", &"C" => "Clear", other => other };
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken));
                let equals = *press == Press::Equals;
                let operator = matches!(press, Press::Op(_));
                let function = column == 0 || matches!(press, Press::Open | Press::Close | Press::Percent | Press::Back);
                let hot = response.hovered() || response.has_focus();
                let fill = if equals { if hot { theme.main.gamma_multiply(0.85) } else { theme.main } }
                    else if hot { ui::LINE } else if operator { theme.main.gamma_multiply(0.14) } else { ui::TRACK };
                let ink = if equals { ui::GROUND } else if operator { theme.main } else if function { ui::SECONDARY } else { ui::TEXT };
                let painter = ui.painter();
                painter.rect_filled(place, 12.0, fill);
                if response.has_focus() { painter.rect_stroke(place, 12.0, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
                let font = if function && !matches!(press, Press::Open | Press::Close) { ui::display(16.0) } else { ui::display_bold(20.0) };
                painter.text(place.center(), Align2::CENTER_CENTER, *label, font, ink);
                if response.clicked() { pressed = Some(*press); }
            }
        }
        if let Some(press) = pressed { self.press(press); }
        ui.add_space(5.0 * KEY.y + 4.0 * GAP + 18.0);

        // The last few answers; a click brings one back.
        if !self.history.is_empty() {
            ui::tracked(&mut ui, "EARLIER", ui::display_bold(10.0), ui::LABEL, 2.0);
            ui.add_space(4.0);
            let mut reuse = None;
            for (index, (expression, answer)) in self.history.iter().enumerate() {
                let (row, response) = ui.allocate_exact_size(vec2(inner.width(), 22.0), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{expression} = {answer}")));
                if response.hovered() { ui.painter().rect_filled(row, 6.0, ui::TRACK); }
                let painter = ui.painter().with_clip_rect(row);
                painter.text(row.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, expression, ui::mono(12.0), ui::LABEL);
                painter.text(row.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, format!("= {answer}"), ui::mono(12.0), if index == 0 { ui::TEXT } else { ui::SECONDARY });
                if response.on_hover_text("Use this answer").clicked() { reuse = Some(answer.clone()); }
            }
            if let Some(answer) = reuse {
                self.entry = answer;
                self.answered = true;
                self.error = None;
            }
        }
    }
}

/// Where the number at the end of `entry` starts (its length when there is none).
fn trailing_number(entry: &str) -> usize {
    let start = entry.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').len();
    // A minus belongs to the number when nothing it could subtract from comes before it.
    if entry[..start].ends_with('-') {
        let before = entry[..start - 1].chars().last();
        if before.is_none() && start < entry.len() { return start - 1; }
    }
    start
}

/// The entry with any brackets left open closed.
fn closed(entry: &str) -> String {
    let open = entry.matches('(').count().saturating_sub(entry.matches(')').count());
    format!("{entry}{}", ")".repeat(open))
}

/// An expression as it is shown: `×`, `÷`, `−` and `√`.
fn pretty(entry: &str) -> String {
    entry.replace("sqrt", "√").replace('*', "×").replace('/', "÷").replace('-', "−")
}

/// A number written plainly: no trailing zeros, ten places at most, and powers of ten
/// only when it is very large or very small.
pub fn format(value: f64) -> String {
    if value == 0.0 { return "0".into(); }
    let magnitude = value.abs();
    if !(1.0e-9..1.0e15).contains(&magnitude) {
        let text = format!("{value:.9e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exponent}");
    }
    let text = format!("{value:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> Calculator {
        let mut calculator = Calculator::default();
        for c in keys.chars() {
            calculator.press(match c {
                '0'..='9' => Press::Digit(c),
                '.' => Press::Dot,
                '(' => Press::Open,
                ')' => Press::Close,
                '=' => Press::Equals,
                '<' => Press::Back,
                '~' => Press::Negate,
                '%' => Press::Percent,
                'r' => Press::Sqrt,
                's' => Press::Square,
                'i' => Press::Inverse,
                op => Press::Op(op),
            });
        }
        calculator
    }

    /// Draws the calculator to `DESUI_CALCULATOR_PREVIEW` (a PNG), for a look at it.
    #[test]
    #[ignore]
    fn native_calculator_preview() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        struct Preview { calculator: Calculator, frames: usize, output: std::path::PathBuf }
        impl eframe::App for Preview {
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                for event in ctx.input(|i| i.events.clone()) {
                    if let egui::Event::Screenshot { image, .. } = event {
                        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        image::save_buffer(&self.output, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).unwrap();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                let theme = Theme::load("signal");
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui::GROUND)).show(ctx, |ui| {
                    let rect = ui.max_rect();
                    self.calculator.show(ui, rect, &theme);
                });
                self.frames += 1;
                if self.frames == 4 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
                ctx.request_repaint();
            }
        }
        let output = std::path::PathBuf::from(std::env::var("DESUI_CALCULATOR_PREVIEW").expect("preview output"));
        let mut calculator = typed("1/3=2^10=12*(3+4");
        calculator.open = true;
        let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 820.0]), event_loop_builder: Some(Box::new(|builder| { builder.with_x11().with_any_thread(true); })), ..Default::default() };
        eframe::run_native("calculator preview", options, Box::new(move |cc| {
            ui::install_fonts(&cc.egui_ctx, None, None);
            crate::apply_style(&cc.egui_ctx, &Theme::load("signal"));
            Ok(Box::new(Preview { calculator, frames: 0, output }))
        })).unwrap();
    }

    #[test]
    fn precedence_and_brackets() {
        assert_eq!(typed("2+3*4=").shown(), "14");
        assert_eq!(typed("(2+3)*4=").shown(), "20");
        assert_eq!(typed("2^10=").shown(), "1024");
        // Brackets left open are closed by =.
        assert_eq!(typed("2*(3+4=").shown(), "14");
        assert_eq!(typed("7/2").preview().as_deref(), Some("3.5"));
    }

    #[test]
    fn answers_are_written_without_noise() {
        assert_eq!(typed(".1+.2=").shown(), "0.3");
        assert_eq!(typed("1/3=").shown(), "0.3333333333");
        assert_eq!(format(2.0e20), "2e20");
        assert_eq!(format(-1.5e-12), "-1.5e-12");
        assert_eq!(format(-0.0), "0");
        assert_eq!(format(1234567.25), "1234567.25");
    }

    #[test]
    fn dividing_by_zero_says_so_and_keeps_the_entry() {
        assert_eq!(typed("12*(3+4").preview().as_deref(), Some("84"), "an open bracket is closed for the preview");
        let calculator = typed("5/0=");
        assert_eq!(calculator.error, Some("Can't divide by zero"));
        assert_eq!(calculator.shown(), "5÷0");
        assert!(calculator.history.is_empty());
    }

    #[test]
    fn functions_signs_and_percent() {
        assert_eq!(typed("9r").shown(), "3");
        assert_eq!(typed("12s").shown(), "144");
        assert_eq!(typed("4i").shown(), "0.25");
        assert_eq!(typed("5~").shown(), "(−5)");
        assert_eq!(typed("5~~").shown(), "5");
        assert_eq!(typed("3*5~=").shown(), "−15");
        assert_eq!(typed("3*5~=").answer().as_deref(), Some("-15"), "what is copied is plain ASCII");
        assert_eq!(typed("200*15%=").shown(), "30");
        // An answer changes sign in place.
        assert_eq!(typed("2+3=~").shown(), "−5");
    }

    #[test]
    fn editing_typing_after_an_answer_and_history() {
        assert_eq!(typed("12<3").shown(), "13");
        assert_eq!(typed("1..5").shown(), "1.5");
        assert_eq!(typed(".5").shown(), "0.5");
        assert_eq!(typed("2+*3=").shown(), "6", "a second operator replaces the first");
        // A digit after an answer starts afresh; an operator carries on from it.
        assert_eq!(typed("2+2=7").shown(), "7");
        assert_eq!(typed("2+2=*3=").shown(), "12");
        let calculator = typed("1+1=2+2=3+3=4+4=5+5=6+6=");
        assert_eq!(calculator.history.len(), HISTORY);
        assert_eq!(calculator.history.front().unwrap(), &("6+6".to_string(), "12".to_string()));
        assert_eq!(calculator.answer().as_deref(), Some("12"));
    }
}
