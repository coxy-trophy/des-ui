//! The Signal interface language: its fixed colours, its two typefaces, and the small
//! parts every panel is built from. Shared glass surfaces, fine lines, soft controls; the
//! theme's accent marks only what is live, selected or has the keyboard.

use crate::theme::Theme;
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Galley, Layout,
    Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, text::LayoutJob, text::TextFormat,
    vec2,
};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

pub const GROUND: Color32 = Color32::from_rgb(0x06, 0x08, 0x0B);
pub const PANEL: Color32 = Color32::from_rgb(0x0A, 0x0E, 0x12);
pub const TRACK: Color32 = Color32::from_rgb(0x14, 0x1B, 0x22);
pub const LINE: Color32 = Color32::from_rgb(0x1A, 0x23, 0x2B);
pub const LINE_STRONG: Color32 = Color32::from_rgb(0x2A, 0x37, 0x42);
pub const LABEL: Color32 = Color32::from_rgb(0x6F, 0x84, 0x92);
pub const SECONDARY: Color32 = Color32::from_rgb(0x8F, 0xA3, 0xB0);
pub const TEXT: Color32 = Color32::from_rgb(0xDC, 0xE7, 0xEC);
pub const CAUTION: Color32 = Color32::from_rgb(0xF5, 0xB8, 0x5A);

/// Shared geometry for controls; terminal cells and workspace layout stay independent.
pub const SPACE: f32 = 8.0;
pub const CONTROL_HEIGHT: f32 = 30.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const SURFACE_RADIUS: f32 = 10.0;

/// Above this share of its range, a meter turns to the caution colour.
pub const CAUTION_ABOVE: f32 = 0.75;

const CHAKRA_MEDIUM: &[u8] = include_bytes!("../assets/fonts/ChakraPetch-Medium.ttf");
const CHAKRA_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/ChakraPetch-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");

/// The typeface that names things: clock, titles, tabs, section labels.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}

pub fn display_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display-bold".into()))
}

/// The typeface for everything measured or typed.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// The monospace fonts installed on this machine: family name and file, by name.
pub fn monospace_fonts() -> Vec<(String, std::path::PathBuf)> {
    let Ok(output) = std::process::Command::new("fc-list")
        .args([":spacing=100", "--format=%{family[0]}\t%{style[0]}\t%{file}\n"])
        .output()
    else {
        return Vec::new();
    };
    let mut fonts: Vec<(String, std::path::PathBuf)> = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split('\t');
        let (Some(family), Some(style), Some(file)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        // One upright face per family, and only formats the renderer reads.
        let usable = [".ttf", ".otf"].iter().any(|ext| file.to_lowercase().ends_with(ext));
        let text_face = !["Emoji", "SignWriting", "Symbols"].iter().any(|word| family.contains(word));
        let upright = matches!(style, "Regular" | "Book" | "Medium" | "Roman");
        if usable && text_face && upright && !fonts.iter().any(|(known, _)| known == family) {
            fonts.push((family.to_string(), file.into()));
        }
    }
    fonts.sort_by_key(|(family, _)| family.to_lowercase());
    fonts
}

/// Installs the typefaces. `symbols` is an installed font with prompt glyphs (a Nerd
/// Font), used for what the monospace face lacks. `terminal` replaces the bundled
/// monospace face with a font of the user's choosing.
pub fn install_fonts(ctx: &egui::Context, symbols: Option<&Path>, terminal: Option<&Path>) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, data: FontData| {
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
    };
    add("chakra-medium", FontData::from_static(CHAKRA_MEDIUM));
    add("chakra-semibold", FontData::from_static(CHAKRA_SEMIBOLD));
    add("jetbrains-mono", FontData::from_static(JETBRAINS_MONO));

    let mut mono_stack = Vec::new();
    if let Some(path) = terminal {
        match std::fs::read(path) {
            Ok(bytes) => {
                add("terminal", FontData::from_owned(bytes));
                mono_stack.push("terminal".to_owned());
            }
            Err(e) => eprintln!("des-ui: cannot read font {}: {e}", path.display()),
        }
    }
    mono_stack.push("jetbrains-mono".to_owned());
    if let Some(path) = symbols {
        match std::fs::read(path) {
            Ok(bytes) => {
                add("symbols", FontData::from_owned(bytes));
                mono_stack.push("symbols".to_owned());
            }
            Err(e) => eprintln!("des-ui: cannot read font {}: {e}", path.display()),
        }
    }
    // egui's own fonts stay last, for emoji and anything else still missing.
    let fallback = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    mono_stack.extend(fallback);

    let with_fallback = |first: &str| {
        let mut stack = vec![first.to_owned()];
        stack.extend(mono_stack.iter().cloned());
        stack
    };
    fonts
        .families
        .insert(FontFamily::Name("display".into()), with_fallback("chakra-medium"));
    fonts
        .families
        .insert(FontFamily::Name("display-bold".into()), with_fallback("chakra-semibold"));
    fonts.families.insert(FontFamily::Proportional, mono_stack.clone());
    fonts.families.insert(FontFamily::Monospace, mono_stack);
    ctx.set_fonts(fonts);
}

/// Lays out one line of text. `tracking` is extra space between letters, which the
/// upper-case labels use.
pub fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, tracking: f32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: tracking,
            ..Default::default()
        },
    );
    ui.fonts(|fonts| fonts.layout_job(job))
}

/// Adds one line of text as a widget.
/// A text field's placeholder. The style's override colour would otherwise paint it
/// as bright as typed text, so it carries its own quieter colour.
pub fn hint(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(LABEL)
}

pub fn text(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Response {
    tracked(ui, text, font, color, 0.0)
}

/// Text never pushes its container wider: what does not fit is cut with an ellipsis.
pub fn tracked(ui: &mut Ui, text: &str, font: FontId, color: Color32, tracking: f32) -> Response {
    let mut job = LayoutJob::default();
    job.wrap = egui::text::TextWrapping {
        max_width: ui.available_width().max(8.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: tracking,
            ..Default::default()
        },
    );
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response
}

/// Groups everything `add` draws into cards: each [`section`] heading inside opens a
/// card that runs to the next heading or the end. The card's background is reserved
/// before its contents and sized afterwards to what was really drawn, so spacing left
/// after a group never stretches it. Outside this, headings stay plain rules.
pub fn cards(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    if ui.data(|data| data.get_temp::<Cards>(cards_id()).is_some()) {
        add(ui);
        return;
    }
    let pad = if ui.available_width() < 480.0 { 10.0 } else { 16.0 };
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(pad as i8, 0)).show(ui, |ui| {
        let inner = ui.max_rect();
        let cards = Cards { painter: ui.painter().clone(), left: inner.left() - pad, right: inner.right() + pad, pad, open: None };
        ui.data_mut(|data| data.insert_temp(cards_id(), cards));
        add(ui);
        if let Some(mut cards) = ui.data(|data| data.get_temp::<Cards>(cards_id())) {
            if let Some(bottom) = close_card(ui, &mut cards) {
                // Room below the last card, which may sit at the foot of a scroll.
                let below = bottom + pad - ui.cursor().top();
                if below > 0.0 { ui.add_space(below); }
            }
        }
        ui.data_mut(|data| data.remove::<Cards>(cards_id()));
    });
}

const CARD_GAP: f32 = 16.0;

#[derive(Clone)]
struct Cards {
    painter: egui::Painter,
    left: f32,
    right: f32,
    pad: f32,
    /// The open card: its reserved background and its top.
    open: Option<(egui::layers::ShapeIdx, f32)>,
}

fn cards_id() -> egui::Id {
    egui::Id::new("des-ui-cards")
}

/// Paints the open card behind what was drawn in it, and returns its bottom.
fn close_card(ui: &Ui, cards: &mut Cards) -> Option<f32> {
    let (idx, top) = cards.open.take()?;
    let layer = cards.painter.layer_id();
    let content = ui.ctx().graphics(|graphics| {
        graphics.get(layer).map_or(top, |list| {
            list.all_entries().skip(idx.0 + 1)
                .map(|clipped| clipped.shape.visual_bounding_rect())
                .filter(|rect| rect.is_finite() && rect.max.y > top)
                .fold(top, |bottom, rect| bottom.max(rect.max.y))
        })
    });
    let rect = Rect::from_min_max(pos2(cards.left, top), pos2(cards.right, content + cards.pad));
    cards.painter.set(idx, egui::epaint::RectShape::new(rect, SURFACE_RADIUS, PANEL, Stroke::new(1.0, LINE), StrokeKind::Inside));
    Some(rect.bottom())
}

/// Inside [`cards`]: closes the card before and opens one for this heading.
fn open_card(ui: &mut Ui) -> bool {
    let Some(mut cards) = ui.data(|data| data.get_temp::<Cards>(cards_id())) else { return false };
    if let Some(bottom) = close_card(ui, &mut cards) {
        let gap = ui.cursor().top() - bottom;
        if gap < CARD_GAP { ui.add_space(CARD_GAP - gap); }
    }
    let top = ui.cursor().top();
    let idx = cards.painter.add(egui::Shape::Noop);
    cards.open = Some((idx, top));
    let pad = cards.pad;
    ui.data_mut(|data| data.insert_temp(cards_id(), cards));
    ui.add_space(pad);
    true
}

/// A section's heading: its name, a rule, and whatever `meta` adds at the right.
/// Inside [`cards`] it starts a card instead, and the name heads it without a rule.
pub fn section(ui: &mut Ui, title: &str, meta: impl FnOnce(&mut Ui)) {
    if open_card(ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            tracked(ui, title, display_bold(12.0), TEXT, 1.0);
            ui.with_layout(Layout::right_to_left(Align::Center), meta);
        });
        ui.add_space(4.0);
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        tracked(ui, title, display_bold(13.0), SECONDARY, 0.8);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            meta(ui);
            let rest = ui.available_rect_before_wrap();
            if rest.width() > 4.0 {
                ui.painter().hline(rest.x_range(), rest.center().y, Stroke::new(1.0, LINE));
            }
        });
    });
    ui.add_space(4.0);
}

/// Small text for the right end of a section heading.
pub fn meta(ui: &mut Ui, value: &str, color: Color32) {
    text(ui, value, mono(11.0), color);
}

/// A labelled bar with its reading at the right. Above the caution threshold both the
/// bar and the reading change colour.
pub fn meter(ui: &mut Ui, theme: &Theme, label: &str, label_width: f32, frac: f32, reading: &str) {
    let frac = frac.clamp(0.0, 1.0);
    let color = if frac > CAUTION_ABOVE { CAUTION } else { theme.main };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let painter = ui.painter();
    painter.text(rect.left_center(), Align2::LEFT_CENTER, label, mono(12.0), LABEL);
    let reading_color = if frac > CAUTION_ABOVE { CAUTION } else { TEXT };
    painter.text(rect.right_center(), Align2::RIGHT_CENTER, reading, mono(12.0), reading_color);
    let track = Rect::from_min_max(
        pos2(rect.left() + label_width + 10.0, rect.center().y - 3.0),
        pos2(rect.right() - 40.0, rect.center().y + 3.0),
    );
    if track.width() > 0.0 {
        painter.rect_filled(track, 0.0, TRACK);
        let mut filled = track;
        filled.set_width(track.width() * frac);
        painter.rect_filled(filled, 0.0, color);
    }
}

/// A plain bar, optionally with a second, muted segment after the first.
pub fn bar(ui: &mut Ui, theme: &Theme, height: f32, frac: f32, second: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, TRACK);
    let frac = frac.clamp(0.0, 1.0);
    let mut first = rect;
    first.set_width(rect.width() * frac);
    painter.rect_filled(first, 0.0, if frac > CAUTION_ABOVE { CAUTION } else { theme.main });
    let second = second.clamp(0.0, 1.0 - frac);
    if second > 0.0 {
        let rest = Rect::from_min_size(first.right_top(), vec2(rect.width() * second, height));
        painter.rect_filled(rest, 0.0, theme.muted());
    }
}

/// A bordered box, the container for tiles. `active` draws it in the accent.
pub fn tile<R>(ui: &mut Ui, theme: &Theme, active: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    crate::material::surface(ui, egui::Frame::new()
        .corner_radius(CONTROL_RADIUS)
        .stroke(Stroke::new(1.0, if active { theme.main } else { LINE }))
        .inner_margin(egui::Margin::symmetric(12, 10)), crate::material::Role::Panel, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui)
        }).inner
}

/// A key's name in a box. Returns the space it took.
pub fn keycap(ui: &mut Ui, key: &str) -> Response {
    let galley = galley(ui, key, mono(11.0), TEXT, 0.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::hover());
    ui.painter()
        .rect_stroke(rect, CONTROL_RADIUS, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
    ui.painter().galley(rect.min + vec2(8.0, 4.0), galley, TEXT);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, key));
    response
}

/// What a key does, with the key at the right.
pub fn key_row(ui: &mut Ui, action: &str, key: &str) {
    ui.horizontal(|ui| {
        text(ui, action, display(13.0), SECONDARY);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| keycap(ui, key));
    });
}

fn control_painter(ui: &Ui) -> egui::Painter {
    let mut painter = ui.painter().clone();
    if !ui.is_enabled() { painter.set_opacity(0.4); }
    painter
}
fn focus_outline(painter: &egui::Painter, response: &Response, theme: &Theme) {
    if response.has_focus() {
        painter.rect_stroke(response.rect.shrink(1.0), CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    }
}

/// A bordered text button, filled with the accent while what it controls is engaged.
pub fn button(ui: &mut Ui, theme: &Theme, label: &str, hint: Option<&str>, engaged: bool) -> Response {
    let name = label.to_string();
    let (ink, hint_ink) = if engaged {
        (TEXT, SECONDARY)
    } else {
        (TEXT, LABEL)
    };
    let font = if engaged { display_bold(12.0) } else { display(12.0) };
    let label = galley(ui, label, font, ink, 0.5);
    let hint = hint.map(|hint| galley(ui, hint, mono(10.0), hint_ink, 0.0));
    let width = 24.0 + label.size().x + hint.as_ref().map_or(0.0, |hint| hint.size().x + 10.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
    let painter = control_painter(ui);
    if engaged {
        painter.rect_filled(rect, CONTROL_RADIUS, theme.tint());
        painter.rect_stroke(rect, CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    } else {
        if response.hovered() { painter.rect_filled(rect, CONTROL_RADIUS, TRACK); }
        let line = if response.hovered() { SECONDARY } else { LINE_STRONG };
        painter.rect_stroke(rect, CONTROL_RADIUS, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    let mut x = rect.left() + 12.0;
    let label_width = label.size().x;
    painter.galley(pos2(x, rect.center().y - label.size().y / 2.0), label, ink);
    x += label_width + 10.0;
    if let Some(hint) = hint {
        painter.galley(pos2(x, rect.center().y - hint.size().y / 2.0), hint, hint_ink);
    }
    focus_outline(&painter, &response, theme);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), engaged, &name));
    response
}

/// An on/off switch. Returns its response; a click means "flip it".
pub fn toggle(ui: &mut Ui, theme: &Theme, on: bool) -> Response {
    let (hit_rect, response) = ui.allocate_exact_size(vec2(40.0, CONTROL_HEIGHT), Sense::click());
    let rect = Rect::from_center_size(hit_rect.center(), vec2(40.0, 20.0));
    let painter = control_painter(ui);
    let line = if on { theme.main } else if response.hovered() { SECONDARY } else { LINE_STRONG };
    painter.rect_filled(rect, 10.0, if on { theme.tint() } else { GROUND });
    painter.rect_stroke(rect, 10.0, Stroke::new(1.0, line), StrokeKind::Inside);
    let knob = Rect::from_min_size(
        pos2(if on { rect.right() - 18.0 } else { rect.left() + 4.0 }, rect.top() + 4.0),
        vec2(14.0, 12.0),
    );
    painter.rect_filled(knob, 6.0, if on { theme.main } else { LABEL });
    focus_outline(&painter, &response, theme);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), on, "Switch"));
    response
}

/// A slider over 0..=1. Returns the new value while it is being dragged or clicked.
pub fn slider(ui: &mut Ui, theme: &Theme, width: f32, value: f32, name: &str) -> Option<f32> {
    slider_sized(ui, theme, width, value, name, CONTROL_HEIGHT)
}

/// A larger slider for primary controls, with the same keyboard and focus behavior.
pub fn slider_sized(ui: &mut Ui, theme: &Theme, width: f32, value: f32, name: &str, height: f32) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    if response.clicked() || response.drag_started() { response.request_focus(); }
    let track_half_height = if height > CONTROL_HEIGHT { 4.0 } else { 3.0 };
    let inset = if height > CONTROL_HEIGHT { 7.0 } else { 5.0 };
    let track = Rect::from_min_max(
        pos2(rect.left() + inset, rect.center().y - track_half_height),
        pos2(rect.right() - inset, rect.center().y + track_half_height),
    );
    let mut picked = response
        .interact_pointer_pos()
        .filter(|_| response.dragged() || response.clicked())
        .map(|pos| ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0));
    if response.has_focus() && ui.is_enabled() {
        let modifiers = ui.input(|input| input.modifiers);
        if !modifiers.alt && !modifiers.ctrl && !modifiers.command {
            let step = if modifiers.shift { 0.1 } else { 0.01 };
            let adjustment = ui.input_mut(|input| {
                if input.consume_key(modifiers, egui::Key::Home) { Some(0.0) }
                else if input.consume_key(modifiers, egui::Key::End) { Some(1.0) }
                else if input.consume_key(modifiers, egui::Key::ArrowRight) || input.consume_key(modifiers, egui::Key::ArrowUp) { Some((value + step).clamp(0.0, 1.0)) }
                else if input.consume_key(modifiers, egui::Key::ArrowLeft) || input.consume_key(modifiers, egui::Key::ArrowDown) { Some((value - step).clamp(0.0, 1.0)) }
                else { None }
            });
            picked = adjustment.or(picked);
        }
    }
    let value = picked.unwrap_or(value).clamp(0.0, 1.0);
    let painter = control_painter(ui);
    painter.rect_filled(track, 3.0, TRACK);
    let mut filled = track;
    filled.set_width(track.width() * value);
    painter.rect_filled(filled, 3.0, theme.main);
    let knob_size = if height > CONTROL_HEIGHT { vec2(14.0, 24.0) } else { vec2(10.0, 18.0) };
    let knob = Rect::from_center_size(pos2(filled.right(), rect.center().y), knob_size);
    let ink = if response.hovered() || response.dragged() { TEXT } else { theme.main };
    painter.rect_filled(knob, 4.0, GROUND);
    painter.rect_stroke(knob, 4.0, Stroke::new(1.5, ink), StrokeKind::Inside);
    focus_outline(&painter, &response, theme);
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value as f64, name));
    picked
}

/// One choice among several: a bordered label, filled with the accent when chosen.
/// `swatch` shows a colour beside the label.
pub fn chip(ui: &mut Ui, theme: &Theme, label: &str, swatch: Option<Color32>, chosen: bool) -> Response {
    let name = label.to_string();
    let ink = TEXT;
    let font = if chosen { display_bold(12.0) } else { display(12.0) };
    let label = galley(ui, label, font, ink, 0.5);
    let lead = if swatch.is_some() { 20.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(label.size().x + 24.0 + lead, 30.0), Sense::click());
    let painter = control_painter(ui);
    if chosen {
        painter.rect_filled(rect, CONTROL_RADIUS, theme.tint());
        painter.rect_stroke(rect, CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    } else {
        if response.hovered() { painter.rect_filled(rect, CONTROL_RADIUS, TRACK); }
        let line = if response.hovered() { SECONDARY } else { LINE_STRONG };
        painter.rect_stroke(rect, CONTROL_RADIUS, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    if let Some(color) = swatch {
        let square = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(12.0, 12.0));
        painter.rect_filled(square, 0.0, color);
        painter.rect_stroke(square, 0.0, Stroke::new(1.0, GROUND), StrokeKind::Inside);
    }
    painter.galley(pos2(rect.left() + 12.0 + lead, rect.center().y - label.size().y / 2.0), label, ink);
    focus_outline(&painter, &response, theme);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), chosen, &name));
    response
}

/// What an [`icon_button`] shows.
#[derive(Clone, Copy)]
pub enum Icon {
    Search,
    Close,
    Settings,
    Music,
    Power,
    Notifications,
    Apps,
    Fullscreen,
    Volume,
    Muted,
    Brightness,
    Wifi,
    Download,
    Upload,
    Display,
    PanelLeft,
    PanelBottom,
    PanelRight,
    /// A frame divided down the middle: split the terminal.
    SplitPane,
    /// Two sheets, one behind the other: copy.
    Copy,
    /// The four ways the applications in view share the workspace: one window, two
    /// side by side, three columns, and a master with others stacked beside it.
    LayoutSingle,
    LayoutSplit,
    LayoutColumns,
    LayoutMaster,
    /// A keyboard's outline with three rows of keys: the on-screen keyboard.
    Keyboard,
    /// Three sliders: the quick settings.
    Quick,
    Bluetooth,
    /// A crescent: do not disturb.
    Moon,
    /// A padlock.
    Lock,
}

impl Icon {
    /// The picture for a layout.
    pub fn of_layout(kind: des_common::ipc::LayoutKind) -> Icon {
        use des_common::ipc::LayoutKind;
        match kind {
            LayoutKind::Single => Icon::LayoutSingle,
            LayoutKind::Split => Icon::LayoutSplit,
            LayoutKind::Columns => Icon::LayoutColumns,
            LayoutKind::Master => Icon::LayoutMaster,
        }
    }
}

/// Left-aligned list action with the same height and focus treatment as controls.
pub fn list_button(ui: &mut Ui, theme: &Theme, label: &str, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
    let painter = control_painter(ui);
    if response.hovered() || response.has_focus() { painter.rect_filled(rect, CONTROL_RADIUS, TRACK); }
    focus_outline(&painter, &response, theme);
    let text = egui::RichText::new(label).font(display(12.0)).color(TEXT);
    ui.put(rect.shrink2(vec2(6.0, 0.0)), egui::Label::new(text).truncate().halign(Align::Min));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    response.on_hover_text(label)
}

/// Clearable field with a full-height clear target and stable focus identity.
pub fn search_field(ui: &mut Ui, theme: &Theme, value: &mut String, id: egui::Id, hint: &str) -> Response {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - CONTROL_HEIGHT - ui.spacing().item_spacing.x).max(20.0);
        let response = ui.add_sized(vec2(width, CONTROL_HEIGHT), egui::TextEdit::singleline(value).margin(eframe::egui::vec2(10.0, 4.0)).id(id).hint_text(self::hint(hint)).vertical_align(Align::Center));
        let clear = ui.add_enabled_ui(!value.is_empty(), |ui| icon_button(ui, theme, Icon::Close, CONTROL_HEIGHT, false)).inner.on_hover_text("Clear search");
        clear.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !value.is_empty(), "Clear search"));
        if clear.clicked() { value.clear(); response.request_focus(); }
        response
    }).inner
}

/// A small square button drawn with lines. `lit` draws it in the accent.
pub fn icon_button(ui: &mut Ui, theme: &Theme, icon: Icon, size: f32, lit: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let color = if lit {
        theme.main
    } else if response.hovered() || response.has_focus() {
        TEXT
    } else {
        SECONDARY
    };
    let painter = control_painter(ui);
    if lit || response.hovered() || response.has_focus() {
        painter.rect_filled(rect.shrink(1.0), CONTROL_RADIUS, if lit { theme.tint() } else { TRACK });
    }
    focus_outline(&painter, &response, theme);
    paint_icon(&painter, rect, icon, color, lit);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), icon.label()));
    response
}

/// `icon_button` in a circle, for the floating status islands.
pub fn round_icon_button(ui: &mut Ui, theme: &Theme, icon: Icon, size: f32, lit: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let active = response.hovered() || response.has_focus();
    let color = if lit { theme.main } else if active { TEXT } else { SECONDARY };
    let painter = control_painter(ui);
    if lit || active {
        painter.circle_filled(rect.center(), size / 2.0 - 1.0, if lit { theme.tint() } else { TRACK });
    }
    if response.has_focus() {
        painter.circle_stroke(rect.center(), size / 2.0 - 1.0, Stroke::new(1.0, theme.main));
    }
    paint_icon(&painter, rect, icon, color, lit);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), icon.label()));
    response
}

/// How much of a status glyph is lit. The unlit parts stay, dimmed, so the shape
/// never changes when the level does.
pub const UNLIT: f32 = 0.28;

/// Wi-Fi parts lit for a signal in percent: the wedge alone, then one and two arcs.
pub fn wifi_bars(percent: u8) -> u8 {
    match percent { 67.. => 3, 34..=66 => 2, _ => 1 }
}

/// Speaker waves lit for a volume in percent: none at zero, then 1 to 3.
pub fn volume_waves(percent: u8) -> u8 {
    match percent { 0 => 0, 1..=33 => 1, 34..=66 => 2, _ => 3 }
}

/// The battery's fill in tenths, so it moves in steps rather than every percent;
/// anything above empty shows at least one step.
pub fn battery_tenths(percent: u8) -> u8 {
    if percent == 0 { 0 } else { percent.min(100).div_ceil(10) }
}

fn arc(center: egui::Pos2, radius: f32, from: f32, to: f32) -> Vec<egui::Pos2> {
    (0..=16).map(|i| {
        let a = from + (to - from) * i as f32 / 16.0;
        center + vec2(a.cos(), a.sin()) * radius
    }).collect()
}

/// A stroked arc with round ends, which egui's lines do not draw themselves.
fn round_arc(painter: &egui::Painter, center: egui::Pos2, radius: f32, from: f32, to: f32, width: f32, color: Color32) {
    let points = arc(center, radius, from, to);
    painter.circle_filled(points[0], width / 2.0, color);
    painter.circle_filled(points[points.len() - 1], width / 2.0, color);
    painter.line(points, Stroke::new(width, color));
}

/// Wi-Fi as one 90-degree fan: a solid wedge under two thick round-ended arcs,
/// `lit` of the three parts lit from the bottom. `scale` 1 is about 20 by 16.
pub fn paint_wifi(painter: &egui::Painter, c: egui::Pos2, color: Color32, lit: u8, scale: f32) {
    use std::f32::consts::PI;
    let (from, to) = (-PI * 0.75, -PI * 0.25);
    let apex = c + vec2(0.0, 7.4) * scale;
    let ink = |part: u8| if lit > part { color } else { color.gamma_multiply(UNLIT) };
    // A small wedge and thin arcs with clear gaps, so it stays light at strip size.
    let mut wedge = vec![apex];
    wedge.extend(arc(apex, 4.4 * scale, from, to));
    painter.add(egui::Shape::convex_polygon(wedge, ink(0), Stroke::NONE));
    round_arc(painter, apex, 8.6 * scale, from, to, 1.7 * scale, ink(1));
    round_arc(painter, apex, 12.8 * scale, from, to, 1.7 * scale, ink(2));
}

/// A wired connection, shown in place of Wi-Fi.
pub fn paint_wired(painter: &egui::Painter, c: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, -4.0), vec2(9.0, 6.0)), 1.5, stroke, StrokeKind::Inside);
    painter.line_segment([c + vec2(0.0, -1.0), c + vec2(0.0, 2.5)], stroke);
    painter.line_segment([c + vec2(-6.0, 2.5), c + vec2(6.0, 2.5)], stroke);
    for x in [-6.0, 0.0, 6.0] { painter.line_segment([c + vec2(x, 2.5), c + vec2(x, 7.0)], stroke); }
}

/// A speaker with `waves` of three lit, or crossed out when muted.
pub fn paint_volume(painter: &egui::Painter, c: egui::Pos2, color: Color32, waves: u8, muted: bool) {
    use std::f32::consts::PI;
    let body = [(-9.0, -2.5), (-5.5, -2.5), (-1.5, -6.0), (-1.5, 6.0), (-5.5, 2.5), (-9.0, 2.5)];
    painter.add(egui::Shape::convex_polygon(body.iter().map(|&(x, y)| c + vec2(x, y)).collect(), color, Stroke::NONE));
    if muted {
        let stroke = Stroke::new(1.6, color);
        painter.line_segment([c + vec2(3.0, -3.0), c + vec2(9.0, 3.0)], stroke);
        painter.line_segment([c + vec2(3.0, 3.0), c + vec2(9.0, -3.0)], stroke);
        return;
    }
    for (index, radius) in [3.5_f32, 7.0, 10.5].into_iter().enumerate() {
        let ink = if waves as usize > index { color } else { color.gamma_multiply(UNLIT) };
        painter.line(arc(c + vec2(-1.5, 0.0), radius, -PI * 0.24, PI * 0.24), Stroke::new(1.6, ink));
    }
}

/// A battery: a thin outline with its own cap, filled to the nearest tenth with
/// square-cornered fill, a bolt over it while charging and a plug when full on
/// mains. 27 by 13 at `scale` 1.
#[allow(clippy::too_many_arguments)]
pub fn paint_battery(painter: &egui::Painter, c: egui::Pos2, color: Color32, percent: u8, charging: bool, full_on_mains: bool, scale: f32) {
    let at = |x: f32, y: f32| c + vec2(x - 13.5, y - 6.5) * scale;
    let frame = color.gamma_multiply(if color == TEXT { 0.45 } else { 0.6 });
    let body = Rect::from_min_max(at(0.6, 0.6), at(23.4, 12.4));
    painter.rect_stroke(body, 3.4 * scale, Stroke::new(1.2 * scale, frame), StrokeKind::Middle);
    // The cap: a short rounded stub against the body, not a separate stroke.
    painter.rect_filled(Rect::from_min_max(at(24.0, 4.3), at(25.9, 8.7)), 1.0 * scale, frame);
    let tenths = battery_tenths(percent);
    if tenths > 0 {
        let width = (18.8 * tenths as f32 / 10.0).max(2.6);
        painter.rect_filled(Rect::from_min_max(at(2.6, 2.6), at(2.6 + width, 10.4)), 1.4 * scale, color);
    }
    if charging {
        // The bolt in two convex halves, light on the fill, edged in the ground.
        let outline = vec![at(13.2, 1.2), at(8.6, 7.0), at(11.8, 7.0), at(10.5, 11.8), at(15.5, 5.5), at(12.2, 5.5)];
        painter.add(egui::Shape::closed_line(outline, Stroke::new(1.8 * scale, GROUND)));
        painter.add(egui::Shape::convex_polygon(vec![at(13.2, 1.2), at(8.6, 7.0), at(11.8, 7.0), at(12.4, 5.5)], TEXT, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![at(12.2, 5.5), at(15.5, 5.5), at(10.5, 11.8), at(11.8, 7.0)], TEXT, Stroke::NONE));
    } else if full_on_mains {
        let stroke = Stroke::new(1.1 * scale, GROUND);
        painter.line_segment([at(10.4, 3.4), at(10.4, 5.0)], stroke);
        painter.line_segment([at(13.6, 3.4), at(13.6, 5.0)], stroke);
        painter.line(vec![at(9.2, 5.0), at(14.8, 5.0), at(14.5, 6.6), at(12.0, 8.4), at(9.5, 6.6), at(9.2, 5.0)], stroke);
        painter.line_segment([at(12.0, 8.4), at(12.0, 9.6)], stroke);
    }
}

/// A slider drawn as a filled pill with its icon inside, at the left of the fill.
/// Returns a level picked by pointer or keys, and whether the icon itself was
/// clicked when `icon_toggles` (as a mute switch).
#[allow(clippy::too_many_arguments)]
pub fn pill_slider(ui: &mut Ui, theme: &Theme, width: f32, value: f32, name: &str, height: f32, icon_toggles: bool, icon: &dyn Fn(&egui::Painter, egui::Pos2, Color32)) -> (Option<f32>, bool) {
    // A taller target than what is drawn, so it is easy to grab.
    let target = (height + 14.0).max(30.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, target), Sense::click_and_drag());
    let pill = Rect::from_center_size(rect.center(), vec2(width, height));
    let icon_zone = pill.left() + height + 6.0;
    if response.clicked() || response.drag_started() { response.request_focus(); }
    let pressed_icon = icon_toggles && response.clicked() && response.interact_pointer_pos().is_some_and(|pos| pos.x <= icon_zone);
    let mut picked = response.interact_pointer_pos()
        .filter(|_| !pressed_icon && (response.dragged() || response.clicked()))
        .map(|pos| ((pos.x - pill.left()) / pill.width()).clamp(0.0, 1.0));
    if response.has_focus() && ui.is_enabled() {
        let modifiers = ui.input(|input| input.modifiers);
        if !modifiers.alt && !modifiers.ctrl && !modifiers.command {
            let step = if modifiers.shift { 0.1 } else { 0.01 };
            let adjustment = ui.input_mut(|input| {
                if input.consume_key(modifiers, egui::Key::Home) { Some(0.0) }
                else if input.consume_key(modifiers, egui::Key::End) { Some(1.0) }
                else if input.consume_key(modifiers, egui::Key::ArrowRight) || input.consume_key(modifiers, egui::Key::ArrowUp) { Some((value + step).clamp(0.0, 1.0)) }
                else if input.consume_key(modifiers, egui::Key::ArrowLeft) || input.consume_key(modifiers, egui::Key::ArrowDown) { Some((value - step).clamp(0.0, 1.0)) }
                else { None }
            });
            picked = adjustment.or(picked);
        }
    }
    let shown = picked.unwrap_or(value).clamp(0.0, 1.0);
    let painter = control_painter(ui);
    let radius = height / 2.0;
    painter.rect_filled(pill, radius, LINE);
    if shown > 0.0 {
        // Never narrower than its own round end, so a low level still reads as a fill.
        let fill = Rect::from_min_size(pill.min, vec2((pill.width() * shown).max(height), height));
        painter.rect_filled(fill, radius, theme.main);
    }
    let centre = pos2(pill.left() + radius + 3.0, pill.center().y);
    icon(&painter, centre, if shown > 0.0 { GROUND } else { SECONDARY });
    if response.has_focus() {
        painter.rect_stroke(pill.expand(2.0), radius + 2.0, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), shown as f64, name));
    (picked, pressed_icon)
}

impl Icon {
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Find in terminal",
            Self::Close => "Close",
            Self::PanelLeft => "Left panel", Self::PanelBottom => "File panel", Self::PanelRight => "Right panel",
            Self::SplitPane => "Split terminal",
            Self::Settings => "Settings", Self::Music => "Music", Self::Power => "Power",
            Self::Notifications => "Notifications", Self::Apps => "Applications", Self::Fullscreen => "Full screen",
            Self::Volume => "Volume", Self::Muted => "Muted", Self::Brightness => "Brightness",
            Self::Wifi => "Wi-Fi signal",
            Self::Download => "Download", Self::Upload => "Upload", Self::Display => "Displays",
            Self::Copy => "Copy last command output",
            Self::LayoutSingle => "One application", Self::LayoutSplit => "Two side by side",
            Self::LayoutColumns => "Three columns", Self::LayoutMaster => "Master and stack",
            Self::Keyboard => "On-screen keyboard", Self::Quick => "Quick settings",
            Self::Bluetooth => "Bluetooth", Self::Moon => "Do not disturb", Self::Lock => "Lock",
        }
    }
}

/// Noninteractive icon, also used beside readings so values remain visible.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_icon(ui.painter(), rect, icon, color, false);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, ui.is_enabled(), icon.label()));
    response.on_hover_text(icon.label())
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32, lit: bool) {
    let stroke = Stroke::new(1.4, color);
    let c = rect.center();
    let line = |a: (f32, f32), b: (f32, f32)| { painter.line_segment([c + vec2(a.0, a.1), c + vec2(b.0, b.1)], stroke); };
    let path = |points: &[(f32, f32)]| { painter.line(points.iter().map(|&(x,y)| c + vec2(x,y)).collect(), stroke); };
    match icon {
        Icon::Settings => {
            painter.circle_stroke(c, 4.0, stroke);
            painter.circle_stroke(c, 1.4, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let v = vec2(a.cos(), a.sin());
                painter.line_segment([c + v * 4.0, c + v * 6.5], stroke);
            }
        }
        Icon::Search => {painter.circle_stroke(c+vec2(-2.0,-2.0),4.5,stroke);line((1.0,1.0),(6.0,6.0));}
        Icon::LayoutSingle | Icon::LayoutSplit | Icon::LayoutColumns | Icon::LayoutMaster => {
            // A screen outline, divided as the layout divides the workspace.
            painter.rect_stroke(Rect::from_min_max(c + vec2(-7.0, -5.0), c + vec2(7.0, 5.0)), 0.0, stroke, StrokeKind::Middle);
            match icon {
                Icon::LayoutSplit => line((0.0, -5.0), (0.0, 5.0)),
                Icon::LayoutColumns => {
                    line((-2.3, -5.0), (-2.3, 5.0));
                    line((2.3, -5.0), (2.3, 5.0));
                }
                Icon::LayoutMaster => {
                    line((0.0, -5.0), (0.0, 5.0));
                    line((0.0, -1.7), (7.0, -1.7));
                    line((0.0, 1.7), (7.0, 1.7));
                }
                _ => {}
            }
        }
        Icon::Keyboard => {
            painter.rect_stroke(Rect::from_min_max(c + vec2(-7.0, -4.5), c + vec2(7.0, 4.5)), 1.0, stroke, StrokeKind::Middle);
            for (y, from, to) in [(-2.0, -4.5, 4.5), (0.5, -4.5, 4.5)] {
                let mut x = from;
                while x <= to + 0.01 {
                    painter.circle_filled(c + vec2(x, y), 0.7, color);
                    x += 3.0;
                }
            }
            line((-3.0, 2.8), (3.0, 2.8));
        }
        Icon::Quick => {
            for (y, knob) in [(-4.0, 2.5), (0.0, -2.5), (4.0, 0.5)] {
                line((-6.0, y), (6.0, y));
                painter.circle_filled(c + vec2(knob, y), 1.8, color);
            }
        }
        Icon::Copy => {
            painter.rect_stroke(Rect::from_min_max(c + vec2(-6.0, -6.0), c + vec2(2.0, 2.0)), 1.0, stroke, StrokeKind::Middle);
            painter.rect_stroke(Rect::from_min_max(c + vec2(-2.0, -2.0), c + vec2(6.0, 6.0)), 1.0, stroke, StrokeKind::Middle);
        }
        Icon::Music => {
            path(&[(-3.0,4.0),(-3.0,-5.0),(5.0,-7.0),(5.0,2.0)]);
            line((-3.0,-2.0),(5.0,-4.0));
            painter.circle_filled(c + vec2(-5.0,4.0), 2.2, color);
            painter.circle_filled(c + vec2(3.0,2.0), 2.2, color);
        }
        Icon::Lock => {
            painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, 2.5), vec2(12.0, 9.0)), 2.0, stroke, StrokeKind::Inside);
            let points = (0..=16).map(|i| {
                let a = std::f32::consts::PI + i as f32 / 16.0 * std::f32::consts::PI;
                c + vec2(0.0, -2.0) + vec2(a.cos() * 3.6, a.sin() * 3.6)
            }).collect::<Vec<_>>();
            painter.line(points, stroke);
            line((-3.6,-2.0),(-3.6,-1.8)); line((3.6,-2.0),(3.6,-1.8));
            painter.circle_filled(c + vec2(0.0, 2.5), 1.1, color);
        }
        Icon::Power => {
            let points = (0..=24).map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + 0.7 + i as f32 / 24.0 * (std::f32::consts::TAU - 1.4);
                c + vec2(a.cos(), a.sin()) * 6.0
            }).collect();
            painter.line(points, stroke);
            line((0.0,-7.0),(0.0,0.0));
        }
        Icon::Notifications => {
            path(&[(-6.0,4.0),(-4.0,1.0),(-4.0,-2.0),(-3.0,-5.0),(0.0,-6.0),(3.0,-5.0),(4.0,-2.0),(4.0,1.0),(6.0,4.0),(-6.0,4.0)]);
            painter.circle_filled(c + vec2(0.0,6.0), 1.2, color);
        }
        Icon::Apps => {
            for x in [-5.0, 2.0] { for y in [-5.0, 2.0] {
                painter.rect_stroke(Rect::from_min_size(c + vec2(x,y), vec2(4.0,4.0)), 0.0, stroke, StrokeKind::Inside);
            }}
        }
        Icon::Fullscreen => {
            for (x,y) in [(-1.0,-1.0),(1.0,-1.0),(-1.0,1.0),(1.0,1.0)] {
                path(&[(x*2.0,y*6.0),(x*6.0,y*6.0),(x*6.0,y*2.0)]);
            }
        }
        Icon::Volume | Icon::Muted => {
            path(&[(-6.0,-2.0),(-3.0,-2.0),(1.0,-5.0),(1.0,5.0),(-3.0,2.0),(-6.0,2.0),(-6.0,-2.0)]);
            if matches!(icon, Icon::Muted) {
                line((3.0,-2.0),(7.0,2.0)); line((3.0,2.0),(7.0,-2.0));
            } else {
                path(&[(3.0,-3.0),(5.0,-1.0),(5.0,1.0),(3.0,3.0)]);
            }
        }
        Icon::Brightness => {
            painter.circle_stroke(c, 2.8, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let v = vec2(a.cos(), a.sin());
                painter.line_segment([c + v * 4.8, c + v * 6.8], stroke);
            }
        }
        Icon::Bluetooth => {
            path(&[(-4.5,-3.5),(4.5,3.5),(0.0,7.0),(0.0,-7.0),(4.5,-3.5),(-4.5,3.5)]);
        }
        Icon::Moon => {
            // A crescent: the part of a disc not covered by a second one up and to the left.
            let (outer, inner, shift) = (6.5_f32, 5.2_f32, vec2(-3.4, -2.6));
            let outside = |a: f32| (vec2(a.cos(), a.sin()) * outer - shift).length() > inner;
            let steps = 240;
            let angle = |i: usize| i as f32 * std::f32::consts::TAU / steps as f32;
            // The visible stretch of the outer edge starts where the covered stretch ends.
            let start = (0..steps).find(|&i| !outside(angle(i)) && outside(angle((i + 1) % steps))).map_or(0, |i| i + 1);
            let lit_arc: Vec<f32> = (0..steps).map(|k| angle((start + k) % steps) + if start + k >= steps { std::f32::consts::TAU } else { 0.0 }).take_while(|&a| outside(a)).collect();
            if lit_arc.len() >= 2 {
                let (first, last) = (c + vec2(lit_arc[0].cos(), lit_arc[0].sin()) * outer, c + vec2(lit_arc[lit_arc.len() - 1].cos(), lit_arc[lit_arc.len() - 1].sin()) * outer);
                let toward = |p: egui::Pos2| (p - (c + shift)).angle();
                let (from, mut to) = (toward(last), toward(first));
                while to < from { to += std::f32::consts::TAU; }
                // The hollow edge lies inside the disc; going round the other way it would not.
                let middle = (from + to) / 2.0;
                if (shift + vec2(middle.cos(), middle.sin()) * inner).length() > outer { to -= std::f32::consts::TAU; }
                let n = lit_arc.len();
                let rim: Vec<egui::Pos2> = lit_arc.iter().map(|a| c + vec2(a.cos(), a.sin()) * outer).collect();
                // The inner edge runs back from the last rim point to the first.
                let hollow: Vec<egui::Pos2> = (0..n).map(|k| { let a = from + (to - from) * k as f32 / (n - 1) as f32; c + shift + vec2(a.cos(), a.sin()) * inner }).collect();
                if lit {
                    let mut mesh = egui::Mesh::default();
                    for k in 0..n { mesh.colored_vertex(rim[k], color); mesh.colored_vertex(hollow[n - 1 - k], color); }
                    for k in 0..n as u32 - 1 { mesh.add_triangle(2 * k, 2 * k + 1, 2 * k + 2); mesh.add_triangle(2 * k + 1, 2 * k + 3, 2 * k + 2); }
                    painter.add(egui::Shape::mesh(mesh));
                }
                let mut outline = rim; outline.extend(hollow); outline.push(outline[0]);
                painter.line(outline, stroke);
            }
        }
        Icon::Wifi => {
            path(&[(-7.0,-2.0),(-4.0,-5.0),(0.0,-6.0),(4.0,-5.0),(7.0,-2.0)]);
            path(&[(-4.0,1.0),(-2.0,-1.0),(0.0,-2.0),(2.0,-1.0),(4.0,1.0)]);
            painter.circle_filled(c + vec2(0.0,4.0), 1.4, color);
        }
        Icon::Download | Icon::Upload => {
            let d = if matches!(icon, Icon::Download) { 1.0 } else { -1.0 };
            line((0.0,-6.0*d),(0.0,5.0*d));
            path(&[(-4.0,1.0*d),(0.0,5.0*d),(4.0,1.0*d)]);
        }
        Icon::Display => {
            painter.rect_stroke(Rect::from_center_size(c - vec2(0.0,1.0), vec2(14.0,9.0)), 0.0, stroke, StrokeKind::Inside);
            line((0.0,4.0),(0.0,6.0)); line((-4.0,6.0),(4.0,6.0));
        }
        Icon::Close => {
            painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], stroke);
            painter.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], stroke);
        }
        Icon::SplitPane => {
            let frame = Rect::from_center_size(c, vec2(14.0, 10.0));
            painter.rect_stroke(frame, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            painter.vline(c.x, frame.y_range(), Stroke::new(1.0, color));
        }
        Icon::PanelLeft | Icon::PanelBottom | Icon::PanelRight => {
            // A screen outline with the panel's region filled when it is open.
            let frame = Rect::from_center_size(c, vec2(14.0, 10.0));
            painter.rect_stroke(frame, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            let region = match icon {
                Icon::PanelLeft => Rect::from_min_size(frame.min, vec2(4.0, 10.0)),
                Icon::PanelRight => Rect::from_min_size(frame.right_top() - vec2(4.0, 0.0), vec2(4.0, 10.0)),
                _ => Rect::from_min_size(frame.left_bottom() - vec2(0.0, 4.0), vec2(14.0, 4.0)),
            };
            if lit {
                painter.rect_filled(region, 0.0, color);
            } else {
                painter.rect_stroke(region, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            }
        }
    }
}

/// A line graph of `values` scaled against `max`, in a box of the given size.
pub fn sparkline(ui: &mut Ui, values: &VecDeque<f32>, max: f32, size: egui::Vec2, color: Color32, boxed: bool) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    if boxed {
        painter.rect_filled(rect, 0.0, GROUND);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));
    }
    if values.len() < 2 || max <= 0.0 {
        return;
    }
    let step = rect.width() / (values.len() - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let frac = (value / max).clamp(0.0, 1.0);
            pos2(
                rect.left() + i as f32 * step,
                rect.bottom() - 2.0 - frac * (rect.height() - 4.0),
            )
        })
        .collect();
    painter.line(points, Stroke::new(1.5, color));
}

/// Corner marks around `rect`, as on a viewfinder.
pub fn corners(painter: &egui::Painter, rect: Rect, arm: f32, stroke: Stroke) {
    for (corner, dx, dy) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment([corner, corner + vec2(arm * dx, 0.0)], stroke);
        painter.line_segment([corner, corner + vec2(0.0, arm * dy)], stroke);
    }
}

/// Lays `count` bars across `width` in rows: how many to a row, how many rows, and
/// each bar's width. A bar is at least 5 points wide and at most 18, with 2 between.
pub fn bar_grid_layout(count: usize, width: f32) -> (usize, usize, f32) {
    const GAP: f32 = 2.0;
    const NARROWEST: f32 = 5.0;
    const WIDEST: f32 = 18.0;
    if count == 0 || width < NARROWEST {
        return (0, 0, 0.0);
    }
    let per_row = (((width + GAP) / (NARROWEST + GAP)).floor() as usize).clamp(1, count);
    let rows = count.div_ceil(per_row);
    let bar = ((width - GAP * (per_row - 1) as f32) / per_row as f32).min(WIDEST);
    (per_row, rows, bar)
}

/// One bar per value in rows as wide as the column, each against `max`; the bar
/// under the pointer names its reading through `name`.
pub fn bar_grid(ui: &mut Ui, theme: &Theme, values: &[f32], max: f32, row_height: f32, name: impl Fn(usize, f32) -> String) {
    const GAP: f32 = 2.0;
    let (per_row, rows, bar) = bar_grid_layout(values.len(), ui.available_width());
    if rows == 0 {
        return;
    }
    let height = rows as f32 * row_height + (rows - 1) as f32 * GAP;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let pointer = response.hover_pos();
    let painter = ui.painter();
    let mut hovered = None;
    for (i, value) in values.iter().enumerate() {
        let (row, column) = (i / per_row, i % per_row);
        let left = rect.left() + column as f32 * (bar + GAP);
        let top = rect.top() + row as f32 * (row_height + GAP);
        let track = Rect::from_min_size(pos2(left, top), vec2(bar, row_height));
        painter.rect_filled(track, 0.0, TRACK);
        let frac = if max > 0.0 { (value / max).clamp(0.0, 1.0) } else { 0.0 };
        let filled = Rect::from_min_max(pos2(left, track.bottom() - row_height * frac), track.right_bottom());
        painter.rect_filled(filled, 0.0, if frac > CAUTION_ABOVE { CAUTION } else { theme.main });
        if pointer.is_some_and(|p| track.contains(p)) {
            hovered = Some((i, *value));
        }
    }
    if let Some((i, value)) = hovered {
        response.on_hover_text(name(i, value));
    }
}

/// Two histories about one axis, both against `max`: `above` rises from it and
/// `below` hangs from it, so that traffic in and out, or reads and writes, can be
/// seen against each other.
pub fn waterfall(ui: &mut Ui, above: &VecDeque<f32>, below: &VecDeque<f32>, max: f32, size: egui::Vec2, above_color: Color32, below_color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, GROUND);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let axis = rect.center().y;
    painter.hline(rect.x_range(), axis, Stroke::new(1.0, LINE_STRONG));
    if max <= 0.0 {
        return;
    }
    let reach = rect.height() / 2.0 - 2.0;
    let draw = |values: &VecDeque<f32>, sign: f32, color: Color32| {
        if values.len() < 2 {
            return;
        }
        let step = rect.width() / (values.len() - 1) as f32;
        let points: Vec<Pos2> = values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let frac = (value / max).clamp(0.0, 1.0);
                pos2(rect.left() + i as f32 * step, axis - sign * frac * reach)
            })
            .collect();
        // The area under the line, as fine strokes: the instrument's texture.
        let fill = color.gamma_multiply(0.35);
        for point in &points {
            painter.line_segment([pos2(point.x, axis), *point], Stroke::new(1.0, fill));
        }
        painter.line(points, Stroke::new(1.5, color));
    };
    draw(above, 1.0, above_color);
    draw(below, -1.0, below_color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_system_native_controls_share_type_geometry_and_focus() {
        let ctx = egui::Context::default();
        crate::apply_style(&ctx, &Theme::load("signal"));
        let style = ctx.style();
        assert_eq!(style.override_font_id, Some(display(13.0)));
        assert_eq!(style.spacing.interact_size.y, 30.0);
        assert!(style.visuals.widgets.hovered.corner_radius.nw >= 6);
        // Scroll bars are a quiet rail, never a bright handle over a black box.
        assert!(style.spacing.scroll.bar_width <= 6.0);
        assert_eq!(style.spacing.scroll.interact_background_opacity, 0.0);
        assert!(style.spacing.scroll.interact_handle_opacity < 0.5);
    }

    #[test]
    fn design_system_icon_buttons_show_keyboard_focus_without_hover() {
        let ctx = egui::Context::default(); install_fonts(&ctx, None, None);
        let theme = Theme::load("signal"); let mut id = egui::Id::NULL;
        let _ = ctx.run(Default::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            id = icon_button(ui, &theme, Icon::Search, 30.0, false).id;
        }); });
        ctx.memory_mut(|memory| memory.request_focus(id));
        let output = ctx.run(Default::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            assert!(icon_button(ui, &theme, Icon::Search, 30.0, false).has_focus());
        }); });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == theme.main)), "focused icon needs a visible focus outline");
    }

    #[test]
    fn design_system_sliders_support_focused_keyboard_adjustment_only() {
        let ctx = egui::Context::default(); install_fonts(&ctx, None, None);
        let theme = Theme::load("signal"); let mut id = egui::Id::NULL;
        let _ = ctx.run(Default::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            id = ui.next_auto_id(); assert_eq!(slider(ui, &theme, 150.0, 0.5, "Test level"), None);
        }); });
        ctx.memory_mut(|memory| memory.request_focus(id));
        let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            assert_eq!(slider(ui, &theme, 150.0, 0.5, "Test level"), Some(0.51));
            assert!(!ui.input(|i| i.key_pressed(egui::Key::ArrowRight)), "focused control must consume the adjustment");
        }); });
        ctx.memory_mut(|memory| memory.surrender_focus(id));
        let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            assert_eq!(slider(ui, &theme, 150.0, 0.5, "Test level"), None);
            assert!(ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)), "unfocused control must leave terminal/navigation keys alone");
        }); });
    }

    #[test]
    fn clicking_slider_gives_it_keyboard_focus() {
        let ctx = egui::Context::default(); install_fonts(&ctx, None, None);
        let theme = Theme::load("signal"); let mut bounds = Rect::NOTHING;
        let _ = ctx.run(Default::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            let start = ui.cursor().min; slider(ui, &theme, 150.0, 0.5, "Volume");
            bounds = Rect::from_min_size(start, vec2(150.0, CONTROL_HEIGHT));
        }); });
        let at = bounds.center();
        for pressed in [true, false] {
            let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }], ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| { slider(ui, &theme, 150.0, 0.5, "Volume"); });
            });
        }
        let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
            assert_eq!(slider(ui, &theme, 150.0, 0.5, "Volume"), Some(1.0));
        }); });
    }

    #[test]
    fn new_icons_say_what_they_are() {
        assert_eq!(Icon::Keyboard.label(), "On-screen keyboard");
        assert_eq!(Icon::Quick.label(), "Quick settings");
    }

    #[test]
    fn bar_grids_fit_their_column() {
        assert_eq!(bar_grid_layout(0, 200.0), (0, 0, 0.0));
        assert_eq!(bar_grid_layout(4, 1.0), (0, 0, 0.0));
        // Four cores in a wide column: one row of bars no wider than the widest.
        assert_eq!(bar_grid_layout(4, 272.0), (4, 1, 18.0));
        // Sixty-four cores in a narrow column wrap into rows, each bar at the narrowest.
        let (per_row, rows, bar) = bar_grid_layout(64, 100.0);
        assert_eq!(per_row, 14);
        assert_eq!(rows, 5);
        assert!(bar >= 5.0 && per_row as f32 * bar + (per_row - 1) as f32 * 2.0 <= 100.0);
        // A row never asks for more width than it has.
        for (count, width) in [(3, 40.0), (12, 90.0), (128, 272.0)] {
            let (per_row, rows, bar) = bar_grid_layout(count, width);
            assert!(per_row as f32 * bar + (per_row - 1) as f32 * 2.0 <= width + 0.01);
            assert!(per_row * rows >= count && per_row * (rows - 1) < count);
        }
    }
}
