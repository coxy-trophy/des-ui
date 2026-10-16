//! The Signal interface language: its fixed colours, its two typefaces, and the small
//! parts every panel is built from. Flat fills, one-pixel lines, square corners; the
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

/// A section's heading: its name, a rule, and whatever `meta` adds at the right.
pub fn section(ui: &mut Ui, title: &str, meta: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        tracked(ui, title, display_bold(12.0), SECONDARY, 3.0);
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
    let (fill, line) = if active {
        (theme.tint(), theme.main)
    } else {
        (GROUND, LINE)
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            add(ui)
        })
        .inner
}

/// A tile holding one labelled value.
pub fn value_tile(ui: &mut Ui, theme: &Theme, label: &str, value: &str) {
    tile(ui, theme, false, |ui| {
        tracked(ui, label, display(10.0), LABEL, 2.0);
        text(ui, value, mono(12.0), TEXT);
    });
}

/// A key's name in a box. Returns the space it took.
pub fn keycap(ui: &mut Ui, key: &str) -> Response {
    let galley = galley(ui, key, mono(11.0), TEXT, 0.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::hover());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
    ui.painter().galley(rect.min + vec2(8.0, 4.0), galley, TEXT);
    response
}

/// What a key does, with the key at the right.
pub fn key_row(ui: &mut Ui, action: &str, key: &str) {
    ui.horizontal(|ui| {
        text(ui, action, mono(12.0), SECONDARY);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| keycap(ui, key));
    });
}

/// A bordered text button, filled with the accent while what it controls is engaged.
pub fn button(ui: &mut Ui, theme: &Theme, label: &str, hint: Option<&str>, engaged: bool) -> Response {
    let (ink, hint_ink) = if engaged {
        (GROUND, theme.muted_on_accent())
    } else {
        (TEXT, LABEL)
    };
    let font = if engaged { display_bold(12.0) } else { display(12.0) };
    let label = galley(ui, label, font, ink, 2.0);
    let hint = hint.map(|hint| galley(ui, hint, mono(10.0), hint_ink, 0.0));
    let width = 24.0 + label.size().x + hint.as_ref().map_or(0.0, |hint| hint.size().x + 10.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let painter = ui.painter();
    if engaged {
        painter.rect_filled(rect, 0.0, theme.main);
    } else {
        let line = if response.hovered() { SECONDARY } else { LINE_STRONG };
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    let mut x = rect.left() + 12.0;
    let label_width = label.size().x;
    painter.galley(pos2(x, rect.center().y - label.size().y / 2.0), label, ink);
    x += label_width + 10.0;
    if let Some(hint) = hint {
        painter.galley(pos2(x, rect.center().y - hint.size().y / 2.0), hint, hint_ink);
    }
    response
}

/// An on/off switch. Returns its response; a click means "flip it".
pub fn toggle(ui: &mut Ui, theme: &Theme, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
    let painter = ui.painter();
    let line = if on { theme.main } else if response.hovered() { SECONDARY } else { LINE_STRONG };
    painter.rect_filled(rect, 0.0, if on { theme.tint() } else { GROUND });
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), StrokeKind::Inside);
    let knob = Rect::from_min_size(
        pos2(if on { rect.right() - 18.0 } else { rect.left() + 4.0 }, rect.top() + 4.0),
        vec2(14.0, 12.0),
    );
    painter.rect_filled(knob, 0.0, if on { theme.main } else { LABEL });
    response
}

/// A slider over 0..=1. Returns the new value while it is being dragged or clicked.
pub fn slider(ui: &mut Ui, theme: &Theme, width: f32, value: f32) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    let track = Rect::from_min_max(
        pos2(rect.left() + 5.0, rect.center().y - 3.0),
        pos2(rect.right() - 5.0, rect.center().y + 3.0),
    );
    let picked = response
        .interact_pointer_pos()
        .filter(|_| response.dragged() || response.clicked())
        .map(|pos| ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0));
    let value = picked.unwrap_or(value).clamp(0.0, 1.0);
    let painter = ui.painter();
    painter.rect_filled(track, 0.0, TRACK);
    let mut filled = track;
    filled.set_width(track.width() * value);
    painter.rect_filled(filled, 0.0, theme.main);
    let knob = Rect::from_center_size(pos2(filled.right(), rect.center().y), vec2(10.0, 18.0));
    let ink = if response.hovered() || response.dragged() { TEXT } else { theme.main };
    painter.rect_filled(knob, 0.0, GROUND);
    painter.rect_stroke(knob, 0.0, Stroke::new(1.5, ink), StrokeKind::Inside);
    picked
}

/// One choice among several: a bordered label, filled with the accent when chosen.
/// `swatch` shows a colour beside the label.
pub fn chip(ui: &mut Ui, theme: &Theme, label: &str, swatch: Option<Color32>, chosen: bool) -> Response {
    let ink = if chosen { GROUND } else { TEXT };
    let font = if chosen { display_bold(12.0) } else { display(12.0) };
    let label = galley(ui, label, font, ink, 1.5);
    let lead = if swatch.is_some() { 20.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(label.size().x + 24.0 + lead, 30.0), Sense::click());
    let painter = ui.painter();
    if chosen {
        painter.rect_filled(rect, 0.0, theme.main);
    } else {
        let line = if response.hovered() { SECONDARY } else { LINE_STRONG };
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    if let Some(color) = swatch {
        let square = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(12.0, 12.0));
        painter.rect_filled(square, 0.0, color);
        painter.rect_stroke(square, 0.0, Stroke::new(1.0, GROUND), StrokeKind::Inside);
    }
    painter.galley(pos2(rect.left() + 12.0 + lead, rect.center().y - label.size().y / 2.0), label, ink);
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
    Battery,
    Charging,
    Wifi,
    Download,
    Upload,
    Display,
    WindowSplit,
    /// An arrow to the right: send to the next display.
    Send,
    PanelLeft,
    PanelBottom,
    PanelRight,
    /// A frame divided down the middle: split the terminal.
    SplitPane,
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
    paint_icon(ui.painter(), rect, icon, color, lit);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), icon.label()));
    response
}

impl Icon {
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Find in terminal",
            Self::Close => "Close", Self::Send => "Send to next display",
            Self::PanelLeft => "Left panel", Self::PanelBottom => "File panel", Self::PanelRight => "Right panel",
            Self::SplitPane => "Split terminal", Self::WindowSplit => "Split application workspace",
            Self::Settings => "Settings", Self::Music => "Music", Self::Power => "Power",
            Self::Notifications => "Notifications", Self::Apps => "Applications", Self::Fullscreen => "Full screen",
            Self::Volume => "Volume", Self::Muted => "Muted", Self::Brightness => "Brightness",
            Self::Battery => "Battery", Self::Charging => "Charging", Self::Wifi => "Wi-Fi signal",
            Self::Download => "Download", Self::Upload => "Upload", Self::Display => "Displays",
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

pub fn icon_value(ui: &mut Ui, symbol: Icon, value: &str, color: Color32, hint: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        icon(ui, symbol, 16.0, color);
        text(ui, value, mono(11.0), color);
    }).response.on_hover_text(hint);
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
        Icon::Music => {
            path(&[(-3.0,4.0),(-3.0,-5.0),(5.0,-7.0),(5.0,2.0)]);
            line((-3.0,-2.0),(5.0,-4.0));
            painter.circle_filled(c + vec2(-5.0,4.0), 2.2, color);
            painter.circle_filled(c + vec2(3.0,2.0), 2.2, color);
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
        Icon::Battery | Icon::Charging => {
            painter.rect_stroke(Rect::from_center_size(c - vec2(1.0,0.0), vec2(12.0,8.0)), 0.0, stroke, StrokeKind::Inside);
            line((6.0,-1.0),(6.0,1.0));
            if matches!(icon, Icon::Charging) { path(&[(0.0,-3.0),(-2.0,0.5),(1.0,0.5),(-1.0,3.0)]); }
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
        Icon::WindowSplit => {
            for x in [-7.0, 1.0] {
                painter.rect_stroke(Rect::from_min_size(c + vec2(x,-5.0),vec2(6.0,10.0)),0.0,stroke,StrokeKind::Inside);
                line((x+1.0,-2.0),(x+5.0,-2.0));
            }
        }
        Icon::Close => {
            painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], stroke);
            painter.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], stroke);
        }
        Icon::Send => {
            painter.line_segment([c + vec2(-6.0, 0.0), c + vec2(5.0, 0.0)], stroke);
            painter.line_segment([c + vec2(1.0, -4.0), c + vec2(5.0, 0.0)], stroke);
            painter.line_segment([c + vec2(1.0, 4.0), c + vec2(5.0, 0.0)], stroke);
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
