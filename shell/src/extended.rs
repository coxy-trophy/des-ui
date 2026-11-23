//! What an extended display (any but the main one) shows of its own: the idle view
//! when nothing is open on it, the strip along its top when applications are, and
//! what it shows while the screen is locked.
//!
//! The applications on an extended display sit below its strip; the compositor keeps
//! that room (`des_common::frame::extended_workspace`).
use crate::sysmon::SysMon;
use crate::theme::Theme;
use crate::ui;
use des_common::ipc::{Command, DesktopState, Place, WORKSPACES};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};

/// What a click on an extended display asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Command(Command),
    /// Open the dock on this display, to start or find an application there.
    OpenApps,
    /// Settings, at the displays.
    Displays,
}

/// The applications in view on `screen` on the workspace in view.
pub fn shown_on(state: &DesktopState, screen: usize) -> Vec<usize> {
    state.apps.iter().enumerate()
        .filter(|(_, app)| app.screen == screen && app.workspace == state.workspace && app.place != Place::Hidden && !app.dialog)
        .map(|(index, _)| index).collect()
}

/// The application in use on `screen`: the one with the keyboard when it is there,
/// else the one in the main place, else the first in view.
pub fn in_use_on(state: &DesktopState, screen: usize) -> Option<usize> {
    let shown = shown_on(state, screen);
    state.focus.filter(|focus| shown.contains(focus))
        .or_else(|| shown.iter().copied().find(|&index| state.apps[index].place == Place::Main))
        .or_else(|| shown.first().copied())
}

/// Whether moving the application at `index` (to the next display, as Super O does)
/// brings it to `screen`.
pub fn moves_here(state: &DesktopState, index: usize, screen: usize) -> bool {
    let Some(app) = state.apps.get(index) else { return false };
    state.screens > 1 && app.screen != screen && (app.screen + 1) % state.screens == screen
}

/// The applications elsewhere that Super O would bring to `screen`.
pub fn bring_here(state: &DesktopState, screen: usize) -> Vec<usize> {
    (0..state.apps.len()).filter(|&index| !state.apps[index].dialog && moves_here(state, index, screen)).collect()
}

fn scale_for(rect: Rect) -> f32 {
    (rect.width() / 1920.0).min(rect.height() / 1080.0).max(rect.width().min(rect.height()) / 1080.0).clamp(0.55, 2.0)
}

/// A soft dark pool behind the middle, so text reads over any wallpaper.
fn scrim(painter: &egui::Painter, rect: Rect, center: Pos2, radius: egui::Vec2, inner: f32, outer: f32, tint: Color32) {
    let color = |alpha: f32| Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color(inner));
    let steps = 72;
    for step in 0..=steps {
        let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + vec2(angle.cos() * radius.x, angle.sin() * radius.y), color(outer));
    }
    for step in 0..steps as u32 { mesh.add_triangle(0, step + 1, step + 2); }
    painter.with_clip_rect(rect).add(mesh);
}

/// A band fading from `alpha` at one edge to nothing.
fn fade(painter: &egui::Painter, band: Rect, alpha: f32, from_top: bool) {
    let dark = Color32::from_rgba_unmultiplied(6, 8, 11, (alpha * 255.0) as u8);
    let (top, bottom) = if from_top { (dark, Color32::TRANSPARENT) } else { (Color32::TRANSPARENT, dark) };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(band.left_top(), top);
    mesh.colored_vertex(band.right_top(), top);
    mesh.colored_vertex(band.right_bottom(), bottom);
    mesh.colored_vertex(band.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

fn glass() -> Color32 { Color32::from_rgba_unmultiplied(10, 14, 18, 200) }

/// A rounded, outlined surface; `lit` draws it in the accent.
fn surface(painter: &egui::Painter, rect: Rect, radius: f32, theme: &Theme, lit: bool, fill: Color32) {
    painter.rect_filled(rect, radius, if lit { theme.main.gamma_multiply(0.14) } else { fill });
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, if lit { theme.main } else { ui::LINE_STRONG }), StrokeKind::Inside);
}

/// A clickable area with its hover, focus and accessible name.
fn press(ui: &mut Ui, rect: Rect, id: egui::Id, name: &str, enabled: bool) -> egui::Response {
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    if enabled { response.clone().on_hover_cursor(CursorIcon::PointingHand); }
    response
}

/// A key as printed on a keycap, at `right` (its right edge), centred on `y`.
fn keycap(painter: &egui::Painter, ui: &Ui, right: Pos2, text: &str, size: f32) -> f32 {
    let galley = ui::galley(ui, text, ui::mono(size), ui::TEXT, 0.4);
    let cap = Rect::from_min_size(pos2(right.x - galley.size().x - size * 1.2, right.y - size * 0.95), vec2(galley.size().x + size * 1.2, size * 1.9));
    painter.rect_filled(cap, 5.0, Color32::from_rgb(0x0D, 0x13, 0x18));
    painter.rect_stroke(cap, 5.0, Stroke::new(1.0_f32, Color32::from_rgb(0x33, 0x42, 0x4E)), StrokeKind::Inside);
    painter.galley(cap.center() - galley.size() / 2.0, galley, ui::TEXT);
    cap.width()
}

/// The workspace buttons, left to right from `left`, centred on `y`.
fn workspaces(ui: &mut Ui, theme: &Theme, state: &DesktopState, screen: usize, left: f32, y: f32, size: f32, act: &mut Option<Act>) -> f32 {
    let painter = ui.painter().clone();
    let gap = size * 0.18;
    for workspace in 0..WORKSPACES {
        let rect = Rect::from_min_size(pos2(left + workspace as f32 * (size + gap), y - size / 2.0), vec2(size, size));
        let current = workspace == state.workspace;
        let response = press(ui, rect, egui::Id::new(("extended-workspace", screen, workspace)), &format!("Workspace {}", workspace + 1), true);
        surface(&painter, rect, size * 0.27, theme, current, if response.hovered() { theme.tint() } else { glass() });
        painter.text(rect.center(), Align2::CENTER_CENTER, (workspace + 1).to_string(), ui::display_bold(size * 0.36), if current { theme.main } else { ui::SECONDARY });
        if response.clicked() && !current { *act = Some(Act::Command(Command::Workspace(workspace))); }
    }
    WORKSPACES as f32 * (size + gap) - gap
}

/// Small numbered boxes, one per display, this one in the accent.
fn displays(painter: &egui::Painter, theme: &Theme, right: Pos2, count: usize, screen: usize, s: f32) -> f32 {
    let (w, h, gap) = (34.0 * s, 22.0 * s, 4.0 * s);
    let total = count.min(6) as f32 * (w + gap) - gap + 16.0 * s;
    let frame = Rect::from_min_size(pos2(right.x - total, right.y - h / 2.0 - 7.0 * s), vec2(total, h + 14.0 * s));
    painter.rect_filled(frame, 10.0 * s, glass());
    painter.rect_stroke(frame, 10.0 * s, Stroke::new(1.0_f32, ui::LINE_STRONG), StrokeKind::Inside);
    for n in 0..count.min(6) {
        let rect = Rect::from_min_size(pos2(frame.left() + 8.0 * s + n as f32 * (w + gap), right.y - h / 2.0), vec2(w, h));
        let this = n == screen;
        if this { painter.rect_filled(rect, 3.0, theme.main.gamma_multiply(0.14)); }
        painter.rect_stroke(rect, 3.0, Stroke::new(if this { 1.5_f32 } else { 1.0 }, if this { theme.main } else { ui::LABEL }), StrokeKind::Inside);
        painter.text(rect.center(), Align2::CENTER_CENTER, (n + 1).to_string(), ui::mono(10.0 * s), if this { theme.main } else { ui::SECONDARY });
    }
    total
}

/// One of the idle view's actions: an icon-free button with its key at the right.
fn action(ui: &mut Ui, theme: &Theme, rect: Rect, id: egui::Id, label: &str, key: Option<&str>, enabled: bool, quiet: bool, s: f32) -> bool {
    let painter = ui.painter().clone();
    let response = press(ui, rect, id, label, enabled);
    let fill = Color32::from_rgba_unmultiplied(10, 14, 18, if quiet { 130 } else { 215 });
    painter.rect_filled(rect, 14.0 * s, if response.hovered() { theme.tint() } else { fill });
    painter.rect_stroke(rect, 14.0 * s, Stroke::new(1.0_f32, if response.has_focus() { theme.main } else { ui::LINE_STRONG }), StrokeKind::Inside);
    let color = if enabled { ui::TEXT } else { ui::LABEL };
    let galley = ui::galley(ui, label, ui::display(15.0 * s), color, 0.4);
    painter.galley(pos2(rect.left() + 20.0 * s, rect.center().y - galley.size().y / 2.0), galley, color);
    if let Some(key) = key { keycap(&painter, ui, pos2(rect.right() - 16.0 * s, rect.center().y), key, 11.5 * s); }
    response.clicked()
}

fn action_width(ui: &Ui, label: &str, key: Option<&str>, s: f32) -> f32 {
    let text = ui::galley(ui, label, ui::display(15.0 * s), ui::TEXT, 0.4).size().x;
    let cap = key.map_or(0.0, |key| ui::galley(ui, key, ui::mono(11.5 * s), ui::TEXT, 0.4).size().x + 11.5 * s * 1.2 + 14.0 * s);
    text + cap + 40.0 * s
}

/// Four readings, side by side or two by two.
fn readings(ui: &mut Ui, theme: &Theme, mon: &SysMon, area: Rect, columns: usize, s: f32) {
    let painter = ui.painter().clone();
    let load = mon.cpu_hist.back().copied().unwrap_or(0.0);
    let memory = mon.mem_used as f32 / mon.mem_total.max(1) as f32;
    let down = mon.rx_hist.back().copied().unwrap_or(0.0);
    let up = mon.tx_hist.back().copied().unwrap_or(0.0);
    let peak = mon.rx_hist.iter().fold(1024.0_f32, |a, b| a.max(*b));
    let caution = |frac: f32| frac > ui::CAUTION_ABOVE;
    let battery = mon.power.as_ref();
    let tiles: [(&str, String, String, f32, Option<&std::collections::VecDeque<f32>>, bool); 4] = [
        ("PROCESSOR", format!("{load:.0}%"), if caution(load / 100.0) { "Busy".into() } else { "Load across all cores".into() }, load / 100.0, Some(&mon.cpu_hist), caution(load / 100.0)),
        ("MEMORY", format!("{:.1} GiB", mon.mem_used as f64 / crate::panels::GIB), format!("of {:.1} GiB", mon.mem_total as f64 / crate::panels::GIB), memory, None, caution(memory)),
        ("NETWORK", format!("{}/s", crate::panels::human_bytes(down as f64)), format!("↓ {}/s · ↑ {}/s", crate::panels::human_bytes(down as f64), crate::panels::human_bytes(up as f64)), down / peak, Some(&mon.rx_hist), false),
        ("BATTERY", mon.battery.map_or("AC".to_string(), |b| format!("{b}%")),
            match battery { Some(b) if b.plugged => "Plugged in".into(), Some(_) => "On battery".into(), None => "No battery".into() },
            mon.battery.map_or(1.0, |b| b as f32 / 100.0), None, false),
    ];
    let rows = 4_usize.div_ceil(columns);
    let gap = 16.0 * s;
    let width = (area.width() - gap * (columns as f32 - 1.0)) / columns as f32;
    let height = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
    for (i, (label, reading, detail, frac, history, warm)) in tiles.into_iter().enumerate() {
        let (column, row) = (i % columns, i / columns);
        let tile = Rect::from_min_size(pos2(area.left() + column as f32 * (width + gap), area.top() + row as f32 * (height + gap)), vec2(width, height));
        painter.rect_filled(tile, 14.0 * s, Color32::from_rgba_unmultiplied(8, 11, 15, 220));
        painter.rect_stroke(tile, 14.0 * s, Stroke::new(1.0_f32, ui::LINE), StrokeKind::Inside);
        let inner = tile.shrink2(vec2(18.0 * s, 15.0 * s));
        let color = if warm { ui::CAUTION } else { theme.main };
        let name = ui::galley(ui, label, ui::display_bold(12.0 * s), ui::SECONDARY, 2.4 * s);
        painter.galley(inner.left_top(), name, ui::SECONDARY);
        painter.text(inner.right_top() + vec2(0.0, -2.0 * s), Align2::RIGHT_TOP, &reading, ui::mono(19.0 * s), if warm { ui::CAUTION } else { ui::TEXT });
        let graph = Rect::from_min_max(pos2(inner.left(), inner.top() + 30.0 * s), pos2(inner.right(), inner.bottom() - 20.0 * s));
        match history.filter(|history| history.len() > 1 && graph.height() > 8.0) {
            Some(history) => {
                let max = if label == "PROCESSOR" { 100.0 } else { history.iter().fold(1024.0_f32, |a, b| a.max(*b)) };
                let n = history.len();
                let points: Vec<Pos2> = history.iter().enumerate().map(|(k, v)| pos2(
                    graph.left() + graph.width() * k as f32 / (n - 1) as f32,
                    graph.bottom() - graph.height() * (v / max).clamp(0.0, 1.0))).collect();
                painter.add(egui::Shape::line(points, Stroke::new(2.0_f32, color)));
            }
            None => {
                let track = Rect::from_center_size(graph.center(), vec2(graph.width(), 8.0 * s));
                painter.rect_filled(track, 4.0 * s, ui::TRACK);
                let mut filled = track;
                filled.set_width(track.width() * frac.clamp(0.0, 1.0));
                painter.rect_filled(filled, 4.0 * s, color);
            }
        }
        painter.text(pos2(inner.left(), inner.bottom()), Align2::LEFT_BOTTOM, &detail, ui::mono(12.0 * s), ui::SECONDARY);
    }
}

/// The idle view: nothing of this display's is open on the workspace in view.
pub fn idle(ui: &mut Ui, theme: &Theme, mon: &SysMon, state: &DesktopState, screen: usize) -> Option<Act> {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let s = scale_for(rect);
    let portrait = rect.height() > rect.width();
    let mut act = None;
    crate::material::paint(ui.ctx(), &painter, rect, 0.0, crate::material::Role::Background);
    fade(&painter, Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.2)), 0.7, true);
    fade(&painter, Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * 0.28), rect.max), 0.85, false);
    let middle = pos2(rect.center().x, rect.top() + rect.height() * if portrait { 0.4 } else { 0.46 });
    scrim(&painter, rect, middle, vec2(900.0 * s, 560.0 * s), 0.78, 0.0, ui::GROUND);

    // Along the top, in the band the strip takes when applications are open.
    let band = des_common::frame::extended_strip(rect.width(), rect.height());
    let y = rect.top() + band.y + band.height / 2.0;
    let left = rect.left() + 48.0 * s;
    let brand = ui::galley(ui, "DES-UI", ui::display_bold(15.0 * s), theme.main, 4.5 * s);
    let brand_width = brand.size().x;
    painter.galley(pos2(left, y - brand.size().y / 2.0), brand, theme.main);
    let chip_text = ui::galley(ui, &format!("DISPLAY {} · EXTENDED", screen + 1), ui::mono(12.0 * s), ui::TEXT, 1.4 * s);
    let chip = Rect::from_min_size(pos2(left + brand_width + 16.0 * s, y - 14.0 * s), vec2(chip_text.size().x + 24.0 * s, 28.0 * s));
    painter.rect_filled(chip, 14.0 * s, glass());
    painter.rect_stroke(chip, 14.0 * s, Stroke::new(1.0_f32, ui::LINE_STRONG), StrokeKind::Inside);
    painter.galley(chip.center() - chip_text.size() / 2.0, chip_text, ui::TEXT);
    let right = rect.right() - 48.0 * s;
    let map = displays(&painter, theme, pos2(right, y), state.screens.max(screen + 1), screen, s);
    if !portrait {
        painter.text(pos2(right - map - 16.0 * s, y), Align2::RIGHT_CENTER, format!("{:.0} × {:.0}", rect.width(), rect.height()), ui::mono(13.0 * s), ui::SECONDARY);
    }
    let size = 44.0 * s;
    let row = WORKSPACES as f32 * (size + size * 0.18) - size * 0.18;
    let (workspace_left, workspace_y) = if portrait { (rect.center().x - row / 2.0, y + 64.0 * s) } else { (rect.center().x - row / 2.0, y) };
    workspaces(ui, theme, state, screen, workspace_left, workspace_y, size, &mut act);

    // The time.
    let now = chrono::Local::now();
    let clock = (rect.height() * 0.19).min(rect.width() * if portrait { 0.18 } else { 0.11 });
    let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display(clock), Color32::from_rgb(0xF2, 0xF7, 0xF9), 1.0);
    let seconds = ui::galley(ui, &now.format("%S").to_string(), ui::display(clock * 0.29), theme.main, 0.0);
    let total = time.size().x + clock * 0.07 + seconds.size().x;
    let top = middle.y - time.size().y * 0.78;
    let start = rect.center().x - total / 2.0;
    let time_size = time.size();
    painter.galley(pos2(start, top), time, ui::TEXT);
    painter.galley(pos2(start + time_size.x + clock * 0.07, top + time_size.y * 0.84 - seconds.size().y), seconds, theme.main);
    let date = ui::galley(ui, &now.format("%A %d %B").to_string().to_uppercase(), ui::display(clock * 0.106), ui::TEXT, clock * 0.032);
    let date_bottom = top + time_size.y + date.size().y;
    painter.galley(pos2(rect.center().x - date.size().x / 2.0, top + time_size.y), date, ui::TEXT);

    // What can be done with an empty display.
    let here = state.focus.filter(|&focus| moves_here(state, focus, screen));
    let actions: [(&str, Option<&str>, bool, bool, Act); 3] = [
        ("Bring the app in use here", Some("SUPER O"), here.is_some(), false, Act::Command(Command::Move(here.unwrap_or(0)))),
        ("Open an app on this display", Some("SUPER SPACE"), true, false, Act::OpenApps),
        ("Arrange displays", None, true, true, Act::Displays),
    ];
    let note_y = date_bottom + 40.0 * s;
    painter.text(pos2(rect.center().x, note_y), Align2::CENTER_TOP, "Nothing is open on this display.", ui::mono(13.0 * s), ui::TEXT);
    let height = 56.0 * s;
    let gap = 14.0 * s;
    let first = note_y + 36.0 * s;
    if portrait {
        let width = (rect.width() - 2.0 * 120.0 * s).clamp(320.0 * s, 560.0 * s);
        for (n, (label, key, enabled, quiet, wanted)) in actions.into_iter().enumerate() {
            let button = Rect::from_min_size(pos2(rect.center().x - width / 2.0, first + n as f32 * (height + gap)), vec2(width, height));
            if action(ui, theme, button, egui::Id::new(("extended-action", screen, n)), label, key, enabled, quiet, s) { act = Some(wanted); }
        }
    } else {
        let widths: Vec<f32> = actions.iter().map(|(label, key, ..)| action_width(ui, label, *key, s)).collect();
        let total = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0);
        let mut x = rect.center().x - total / 2.0;
        for (n, ((label, key, enabled, quiet, wanted), width)) in actions.into_iter().zip(widths).enumerate() {
            let button = Rect::from_min_size(pos2(x, first), vec2(width, height));
            if action(ui, theme, button, egui::Id::new(("extended-action", screen, n)), label, key, enabled, quiet, s) { act = Some(wanted); }
            x += width + gap;
        }
    }

    // Four readings along the bottom.
    let columns = if portrait || rect.width() < 1100.0 { 2 } else { 4 };
    let tile_height = 96.0 * s;
    let rows = 4 / columns;
    let area_height = tile_height * rows as f32 + 16.0 * s * (rows as f32 - 1.0);
    let area = Rect::from_min_max(pos2(rect.left() + 48.0 * s, rect.bottom() - 40.0 * s - area_height), pos2(rect.right() - 48.0 * s, rect.bottom() - 40.0 * s));
    readings(ui, theme, mon, area, columns, s);
    act
}

/// The strip along the top while applications are open on this display.
pub fn strip(ui: &mut Ui, theme: &Theme, state: &DesktopState, screen: usize) -> Option<Act> {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    crate::material::paint(ui.ctx(), &painter, rect, 0.0, crate::material::Role::Background);
    painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(6, 8, 11, 140));
    let band = des_common::frame::extended_strip(rect.width(), rect.height());
    let bar = Rect::from_min_size(rect.min + vec2(band.x, band.y), vec2(band.width, band.height));
    painter.rect_filled(bar, 14.0, Color32::from_rgba_unmultiplied(8, 11, 15, 232));
    painter.rect_stroke(bar, 14.0, Stroke::new(1.0_f32, ui::LINE), StrokeKind::Inside);
    let mut act = None;
    let y = bar.center().y;
    let mut x = bar.left() + 16.0;
    let tag = ui::galley(ui, &format!("D{}", screen + 1), ui::display_bold(13.0), theme.main, 3.0);
    let tag_width = tag.size().x;
    painter.galley(pos2(x, y - tag.size().y / 2.0), tag, theme.main);
    x += tag_width + 14.0;
    x += workspaces(ui, theme, state, screen, x, y, 34.0, &mut act) + 14.0;
    painter.vline(x, (y - 12.0)..=(y + 12.0), Stroke::new(1.0_f32, ui::LINE_STRONG));
    x += 14.0;

    // From the right: the time, then the display's own actions.
    let mut right = bar.right() - 16.0;
    let now = chrono::Local::now();
    let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display(18.0), ui::TEXT, 0.6);
    right -= time.size().x;
    painter.galley(pos2(right, y - time.size().y / 2.0), time, ui::TEXT);
    right -= 14.0;
    painter.vline(right, (y - 12.0)..=(y + 12.0), Stroke::new(1.0_f32, ui::LINE_STRONG));
    right -= 8.0;
    let in_use = in_use_on(state, screen);
    let icon = |ui: &mut Ui, right: f32, n: usize, name: &str, enabled: bool, draw: &dyn Fn(&egui::Painter, Rect, Color32)| -> (bool, f32) {
        let rect = Rect::from_min_size(pos2(right - 36.0, y - 18.0), vec2(36.0, 36.0));
        let response = press(ui, rect, egui::Id::new(("extended-strip", screen, n)), name, enabled);
        let response = response.on_hover_text(name);
        if response.hovered() { painter.rect_filled(rect, 10.0, theme.tint()); }
        draw(&painter, rect.shrink(9.0), if enabled { ui::SECONDARY } else { ui::LABEL });
        (response.clicked(), rect.width())
    };
    let (open, width) = icon(ui, right, 0, "Open an app on this display (Super Space)", true, &|painter, r, color| {
        painter.hline(r.x_range(), r.center().y, Stroke::new(2.0_f32, color));
        painter.vline(r.center().x, r.y_range(), Stroke::new(2.0_f32, color));
    });
    if open { act = Some(Act::OpenApps); }
    right -= width + 4.0;
    let next = state.screens.max(1);
    let (send, width) = icon(ui, right, 1, &format!("Send the app in use to display {} (Super O)", (screen + 1) % next + 1), in_use.is_some() && state.screens > 1, &|painter, r, color| {
        painter.rect_stroke(Rect::from_min_max(pos2(r.left() + r.width() * 0.35, r.top() + 2.0), r.right_bottom() - vec2(0.0, 3.0)), 2.0, Stroke::new(2.0_f32, color), StrokeKind::Inside);
        let mid = r.center().y;
        painter.line_segment([pos2(r.left() + r.width() * 0.28, mid), pos2(r.left(), mid)], Stroke::new(2.0_f32, color));
        painter.line_segment([pos2(r.left(), mid), pos2(r.left() + 4.0, mid - 4.0)], Stroke::new(2.0_f32, color));
        painter.line_segment([pos2(r.left(), mid), pos2(r.left() + 4.0, mid + 4.0)], Stroke::new(2.0_f32, color));
    });
    if send { if let Some(index) = in_use { act = Some(Act::Command(Command::Move(index))); } }
    right -= width + 14.0;
    let shown = shown_on(state, screen).len();
    let count = ui::galley(ui, &format!("{shown} APP{} ON THIS DISPLAY", if shown == 1 { "" } else { "S" }), ui::mono(12.0), ui::SECONDARY, 0.8);
    right -= count.size().x;
    let count_left = right;
    painter.galley(pos2(right, y - count.size().y / 2.0), count, ui::SECONDARY);

    // The application in use, in what room is left.
    if let Some(index) = in_use {
        let app = &state.apps[index];
        let room = Rect::from_min_max(pos2(x, bar.top()), pos2(count_left - 20.0, bar.bottom()));
        if room.width() > 60.0 {
            let mark = Rect::from_center_size(pos2(x + 11.0, y), vec2(22.0, 22.0));
            painter.rect_filled(mark, 6.0, ui::TRACK);
            painter.rect_stroke(mark, 6.0, Stroke::new(1.0_f32, if state.focus == Some(index) { theme.main } else { ui::LINE_STRONG }), StrokeKind::Inside);
            let title = if app.title.trim().is_empty() { app.app_id.as_str() } else { app.title.as_str() };
            let galley = ui.painter().layout_no_wrap(title.to_string(), ui::display(15.0), ui::TEXT);
            painter.with_clip_rect(room.shrink2(vec2(0.0, 4.0))).galley(pos2(x + 32.0, y - galley.size().y / 2.0), galley, ui::TEXT);
        }
    }
    act
}

/// What an extended display shows while the screen is locked: the time and where to
/// unlock, and nothing of the session.
pub fn locked(ui: &mut Ui, theme: &Theme, screen: usize) {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let s = scale_for(rect);
    painter.rect_filled(rect, 0.0, ui::GROUND);
    let middle = pos2(rect.center().x, rect.top() + rect.height() * 0.44);
    scrim(&painter, rect, middle, vec2(760.0 * s, 520.0 * s), 0.11, 0.0, theme.main);
    let y = rect.top() + 52.0 * s;
    let left = rect.left() + 64.0 * s;
    let brand = ui::galley(ui, "DES-UI", ui::display_bold(15.0 * s), ui::SECONDARY, 4.5 * s);
    let brand_width = brand.size().x;
    painter.galley(pos2(left, y - brand.size().y / 2.0), brand, ui::SECONDARY);
    let chip_text = ui::galley(ui, "LOCKED", ui::mono(12.0 * s), ui::TEXT, 1.4 * s);
    let chip = Rect::from_min_size(pos2(left + brand_width + 16.0 * s, y - 14.0 * s), vec2(chip_text.size().x + 24.0 * s, 28.0 * s));
    painter.rect_stroke(chip, 14.0 * s, Stroke::new(1.0_f32, ui::LINE_STRONG), StrokeKind::Inside);
    painter.galley(chip.center() - chip_text.size() / 2.0, chip_text, ui::TEXT);
    painter.text(pos2(rect.right() - 64.0 * s, y), Align2::RIGHT_CENTER, format!("DISPLAY {}", screen + 1), ui::mono(13.0 * s), ui::SECONDARY);

    let now = chrono::Local::now();
    let clock = (rect.height() * 0.155).min(rect.width() * 0.15);
    let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display(clock), Color32::from_rgb(0xF2, 0xF7, 0xF9), 1.0);
    let top = rect.top() + rect.height() * 0.25;
    let time_size = time.size();
    painter.galley(pos2(rect.center().x - time_size.x / 2.0, top), time, ui::TEXT);
    let date = ui::galley(ui, &now.format("%A %d %B").to_string().to_uppercase(), ui::display(20.0 * s), ui::SECONDARY, 6.0 * s);
    let date_top = top + time_size.y + 8.0 * s;
    let date_height = date.size().y;
    painter.galley(pos2(rect.center().x - date.size().x / 2.0, date_top), date, ui::SECONDARY);
    let ring = Rect::from_center_size(pos2(rect.center().x, date_top + date_height + 100.0 * s), vec2(88.0 * s, 88.0 * s));
    painter.circle_stroke(ring.center(), ring.width() / 2.0, Stroke::new(2.0_f32, theme.main));
    ui::paint_icon(&painter, ring.shrink(29.0 * s), ui::Icon::Lock, theme.main, true);
    let line_y = ring.bottom() + 34.0 * s;
    let label = ui::galley(ui, "Unlock on display 1", ui::display_bold(22.0 * s), ui::TEXT, 0.4);
    let arrow = 30.0 * s;
    let width = arrow + label.size().x;
    let start = rect.center().x - width / 2.0;
    let mid = line_y + label.size().y / 2.0;
    let stroke = Stroke::new(2.2_f32 * s, theme.main);
    painter.line_segment([pos2(start + 20.0 * s, mid), pos2(start, mid)], stroke);
    painter.line_segment([pos2(start, mid), pos2(start + 7.0 * s, mid - 7.0 * s)], stroke);
    painter.line_segment([pos2(start, mid), pos2(start + 7.0 * s, mid + 7.0 * s)], stroke);
    let label_height = label.size().y;
    painter.galley(pos2(start + arrow, line_y), label, ui::TEXT);
    painter.text(pos2(rect.center().x, line_y + label_height + 16.0 * s), Align2::CENTER_TOP,
        "Type your password on display 1. Nothing from your session is shown here.", ui::mono(13.0 * s), ui::SECONDARY);
}

#[cfg(test)]
mod tests {
    use super::*;
    use des_common::ipc::AppInfo;

    fn app(screen: usize, workspace: usize, place: Place) -> AppInfo {
        AppInfo { app_id: "app".into(), title: "App".into(), screen, place, workspace, pinned: false, urgent: false, dialog: false }
    }

    #[test]
    fn only_applications_in_view_on_the_display_count_as_open_there() {
        let state = DesktopState { screens: 2, apps: vec![app(1, 0, Place::Main), app(1, 1, Place::Hidden), app(0, 0, Place::Main)], ..Default::default() };
        assert_eq!(shown_on(&state, 1), vec![0]);
        assert_eq!(shown_on(&state, 0), vec![2]);
        let other = DesktopState { workspace: 1, ..state.clone() };
        assert!(shown_on(&other, 1).is_empty(), "an application on another workspace leaves the display idle");
    }

    #[test]
    fn the_app_in_use_prefers_the_keyboard_then_the_main_place() {
        let mut state = DesktopState { screens: 2, apps: vec![app(1, 0, Place::Side), app(1, 0, Place::Main), app(0, 0, Place::Main)], ..Default::default() };
        assert_eq!(in_use_on(&state, 1), Some(1));
        state.focus = Some(0);
        assert_eq!(in_use_on(&state, 1), Some(0));
        state.focus = Some(2);
        assert_eq!(in_use_on(&state, 1), Some(1), "the keyboard on another display does not count");
    }

    #[test]
    fn bringing_here_follows_the_next_display_rule() {
        let state = DesktopState { screens: 3, apps: vec![app(0, 0, Place::Main), app(1, 0, Place::Main), app(2, 0, Place::Main)], ..Default::default() };
        assert!(moves_here(&state, 0, 1));
        assert!(!moves_here(&state, 0, 2), "Super O sends display 1's apps to display 2 only");
        assert!(moves_here(&state, 2, 0));
        assert_eq!(bring_here(&state, 2), vec![1]);
        let one = DesktopState { screens: 1, ..state };
        assert!(!moves_here(&one, 0, 0));
    }
}
