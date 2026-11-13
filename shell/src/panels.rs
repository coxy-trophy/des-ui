//! The frame around the workspace: the two monitoring columns, the file browser, the
//! status strip, and what a further display shows while it is idle.

use crate::geo::Snapshot;
use crate::powerup::{self, Part};
use crate::globe;
use crate::sysmon::SysMon;
use crate::theme::Theme;
use crate::ui::{self, Icon};
use des_common::{
    Panel, Panels,
    ipc::{DesktopState, Place},
};
use eframe::egui::{
    Align, Align2, Layout, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// What the left column keeps for the process list and the companion, so that the
/// optional instrument sections above them never crowd them out.
const LEFT_RESERVE: f32 = CARD_CHROME + 3.0 * 26.0 + 30.0;

/// How many of the optional sections, in order and each of the given height, fit in
/// `available` while leaving `reserve` for what must follow.
fn sections_that_fit(available: f32, reserve: f32, heights: &[f32]) -> usize {
    let mut left = available - reserve;
    heights
        .iter()
        .take_while(|height| {
            left -= **height;
            left >= 0.0
        })
        .count()
}

fn human_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// Lays two equal tiles side by side.
/// A rounded card around a group of readings, as wide as the column.
pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = eframe::egui::Frame::new()
        .fill(ui::PANEL)
        .stroke(Stroke::new(1.0, ui::LINE))
        .corner_radius(ui::SURFACE_RADIUS)
        .inner_margin(eframe::egui::Margin::same(CARD_PAD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 7.0;
            add(ui)
        })
        .inner;
    ui.add_space(CARD_GAP);
    inner
}

/// Inside a card around a panel's readings.
const CARD_PAD: f32 = 12.0;
/// Between cards.
const CARD_GAP: f32 = 8.0;
/// A card's frame and heading, before its readings.
const CARD_CHROME: f32 = CARD_PAD * 2.0 + 18.0 + 7.0 + CARD_GAP;

/// A card's heading: its name at the left, and whatever `right` draws at the right.
/// Fixed height, so the column's budget can count on it.
pub(crate) fn card_head(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let title = ui::galley(ui, title, ui::display_bold(12.5), ui::SECONDARY, 1.2);
    ui.painter().galley(pos2(rect.left(), rect.center().y - title.size().y / 2.0), title.clone(), ui::SECONDARY);
    let mut end = ui.new_child(eframe::egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(rect.left() + title.size().x + 8.0, rect.top()), rect.max)).layout(Layout::right_to_left(Align::Center)));
    end.spacing_mut().item_spacing.x = 6.0;
    right(&mut end);
}

/// A small rounded label: a state in a word or two.
pub(crate) fn pill(ui: &mut Ui, text: &str, color: eframe::egui::Color32) -> eframe::egui::Response {
    let galley = ui::galley(ui, text, ui::mono(10.0), color, 0.6);
    let size = galley.size() + vec2(14.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_stroke(rect, size.y / 2.0, Stroke::new(1.0, color.gamma_multiply(0.7)), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

/// A labelled reading in a fixed-height row: name at the left, value at the right.
fn reading_row(ui: &mut Ui, label: &str, value: &str, color: eframe::egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, ui::mono(11.5), ui::LABEL);
    ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, value, ui::mono(11.5), color);
}

fn clock(ui: &mut Ui, theme: &Theme, size: f32) {
    let now = chrono::Local::now();
    let main = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display(size), ui::TEXT, 1.0);
    let seconds = ui::galley(ui, &now.format("%S").to_string(), ui::display(size * 0.35), theme.main, 0.0);
    let width = main.size().x + size * 0.12 + seconds.size().x;
    let height = main.size().y * 0.84;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    // A click opens the calendar below the clock.
    if response.on_hover_cursor(eframe::egui::CursorIcon::PointingHand).on_hover_text("Calendar").clicked() {
        crate::agenda::request(ui.ctx(), pos2(rect.left(), rect.bottom() + 40.0));
    }
    let baseline = rect.bottom();
    let seconds_pos = pos2(rect.left() + main.size().x + size * 0.12, baseline - seconds.size().y * 0.95);
    ui.painter().galley(pos2(rect.left(), baseline - main.size().y * 0.92), main, ui::TEXT);
    ui.painter().galley(seconds_pos, seconds, theme.main);
}

/// The date and how long the machine has been up, on one line; the uptime goes
/// when the column is too narrow for both.
fn date_line(ui: &mut Ui, mon: &SysMon, font: eframe::egui::FontId) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let date = ui::galley(ui, &chrono::Local::now().format("%a %d %b %Y").to_string().to_uppercase(), font.clone(), ui::SECONDARY, 1.6);
    let up = mon.uptime_shown();
    let uptime = ui::galley(ui, &format!("UP {} D {} H", up / 86400, (up / 3600) % 24), font, ui::SECONDARY, 1.0);
    ui.painter().galley(pos2(rect.left(), rect.center().y - date.size().y / 2.0), date.clone(), ui::SECONDARY);
    if date.size().x + 16.0 + uptime.size().x <= rect.width() {
        ui.painter().galley(pos2(rect.right() - uptime.size().x, rect.center().y - uptime.size().y / 2.0), uptime, ui::SECONDARY);
    }
}

/// What the calendar has next today, as a way into the calendar.
fn next_event(ui: &mut Ui, next: &(String, chrono::NaiveDateTime)) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, format!("Next: {} at {}", next.0, next.1.format("%H:%M"))));
    let painter = ui.painter();
    painter.rect_filled(rect, ui::CONTROL_RADIUS + 2.0, if response.hovered() { ui::LINE } else { ui::TRACK });
    let icon = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(14.0, 14.0));
    painter.rect_stroke(icon, 2.0, Stroke::new(1.3, ui::SECONDARY), StrokeKind::Inside);
    painter.hline(icon.x_range(), icon.top() + 4.0, Stroke::new(1.3, ui::SECONDARY));
    let time = ui::galley(ui, &next.1.format("%H:%M").to_string(), ui::mono(11.0), ui::SECONDARY, 0.0);
    let room = rect.width() - 34.0 - time.size().x - 18.0;
    let mut job = eframe::egui::text::LayoutJob::simple_singleline(format!("Next: {}", next.0), ui::display(12.5), ui::TEXT);
    job.wrap = eframe::egui::text::TextWrapping::truncate_at_width(room.max(20.0));
    let title = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 34.0, rect.center().y - title.size().y / 2.0), title, ui::TEXT);
    painter.galley(pos2(rect.right() - 10.0 - time.size().x, rect.center().y - time.size().y / 2.0), time, ui::SECONDARY);
    if response.on_hover_cursor(eframe::egui::CursorIcon::PointingHand).clicked() {
        crate::agenda::request(ui.ctx(), pos2(rect.left(), rect.bottom() + 8.0));
    }
}

/// A line chart of recent values with the area under it lightly filled.
fn area_chart(ui: &mut Ui, values: &[f32], max: f32, size: eframe::egui::Vec2, color: eframe::egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if values.len() < 2 || max <= 0.0 { return; }
    let step = rect.width() / (values.len() - 1) as f32;
    let points: Vec<eframe::egui::Pos2> = values.iter().enumerate()
        .map(|(i, v)| pos2(rect.left() + i as f32 * step, rect.bottom() - 1.0 - (v / max).clamp(0.0, 1.0) * (rect.height() - 2.0)))
        .collect();
    let mut mesh = eframe::egui::Mesh::default();
    let fill = color.gamma_multiply(0.14);
    for pair in points.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], fill);
        mesh.colored_vertex(pair[1], fill);
        mesh.colored_vertex(pos2(pair[1].x, rect.bottom()), fill);
        mesh.colored_vertex(pos2(pair[0].x, rect.bottom()), fill);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    ui.painter().add(eframe::egui::Shape::mesh(mesh));
    ui.painter().line(points, Stroke::new(1.5, color));
}

/// The machine whose readings these are: its name, whether it is this one or one
/// followed through ssh, and services that failed.
fn machine_card(ui: &mut Ui, theme: &Theme, mon: &SysMon) {
    card(ui, |ui| {
        let name = mon.remote.as_ref().map_or(mon.host.as_str(), |remote| if remote.live { mon.host.as_str() } else { remote.name.as_str() });
        card_head(ui, &truncate(&name.to_uppercase(), 16), |ui| {
            match &mon.remote {
                Some(remote) if remote.live => { pill(ui, "REMOTE · LIVE", theme.main); }
                Some(remote) => { pill(ui, &format!("REMOTE · {}", remote.word.to_uppercase()), ui::CAUTION); }
                None => { pill(ui, "LOCAL", theme.main); }
            }
            if !mon.failed_units.is_empty() {
                pill(ui, &format!("{} FAILED", mon.failed_units.len()), ui::CAUTION)
                    .on_hover_text(mon.failed_units.join("\n"));
            }
        });
        let about = format!("{} · kernel {}", mon.os, mon.kernel.split(['-', '+']).next().unwrap_or(&mon.kernel));
        let fit = ((ui.available_width() / 7.2) as usize).max(8);
        ui::text(ui, &truncate(&about, fit), ui::mono(11.5), ui::SECONDARY).on_hover_text(format!("{} · kernel {}", mon.os, mon.kernel));
        for unit in mon.failed_units.iter().take(2) {
            ui::text(ui, &truncate(unit, fit), ui::mono(11.5), ui::CAUTION);
        }
    });
}

/// Keep the companion's footer independent of telemetry flow and clipping.
pub fn left(ui: &mut Ui, theme: &Theme, mon: &SysMon, pet: &mut crate::pet::Pet, widgets: &crate::widgets::Widgets, next: Option<&(String, chrono::NaiveDateTime)>) {
    let rect = ui.max_rect();
    let footer = pet.room().min(rect.height());
    let body = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.bottom()-footer-if footer>0.0 {8.0}else{0.0}).max(rect.top()+1.0)));
    let mut instruments = ui.new_child(eframe::egui::UiBuilder::new().id_salt("left-instruments").max_rect(body).layout(Layout::top_down(Align::Min)));
    instruments.set_clip_rect(body.intersect(ui.clip_rect()));
    if body.height() < 560.0 {
        left_compact(&mut instruments, theme, mon);
    } else {
        left_instruments(&mut instruments, theme, mon, widgets, next);
    }
    if footer > 0.0 {
        let area = Rect::from_min_max(pos2(rect.left(), rect.bottom()-footer), rect.max);
        let mut companion = ui.new_child(eframe::egui::UiBuilder::new().id_salt("left-pet").max_rect(area).layout(Layout::top_down(Align::Min)));
        companion.set_clip_rect(area.intersect(ui.clip_rect()));
        pet.show(&mut companion);
    }
}

/// On a short screen: the time, the processor and memory together, and the busiest
/// processes for as far as they fit.
fn left_compact(ui: &mut Ui, theme: &Theme, mon: &SysMon) {
    ui.spacing_mut().item_spacing.y = 5.0;
    clock(ui, theme, if ui.max_rect().height() < 300.0 {38.0}else{46.0});
    date_line(ui, mon, ui::mono(10.5));
    ui.add_space(6.0);
    let cpu = mon.cpu_hist.back().copied().unwrap_or(0.0);
    card(ui, |ui| {
        card_head(ui, "PROCESSOR", |ui| { ui::text(ui, &format!("{cpu:.0}%"), ui::mono(12.0), if cpu > ui::CAUTION_ABOVE * 100.0 { ui::CAUTION } else { ui::TEXT }); });
        ui::bar_grid(ui, theme, &mon.cores, 100.0, 18.0, |i, usage| format!("Core {}: {usage:.0}%", i+1));
        ui.add_space(2.0);
        card_head(ui, "MEMORY", |ui| { ui::text(ui, &format!("{:.1}/{:.0} GiB", mon.mem_used as f64/GIB, mon.mem_total as f64/GIB), ui::mono(11.0), ui::TEXT); });
        ui::bar(ui, theme, 6.0, mon.mem_used as f32/mon.mem_total.max(1) as f32, 0.0);
        if let Some(temperature) = mon.cpu_temperature() {
            reading_row(ui, "CPU TEMP", &format!("{temperature:.0}°"), if temperature >= 100.0 * ui::CAUTION_ABOVE { ui::CAUTION } else { ui::TEXT });
        }
    });
    if ui.available_height() >= CARD_CHROME + 24.0 {
        let floor = ui.max_rect().bottom() - CARD_PAD - CARD_GAP;
        card(ui, |ui| {
            card_head(ui, "PROCESSES", |ui| ui::meta(ui, "CPU", ui::LABEL));
            ui.spacing_mut().item_spacing.y = 0.0;
            for process in &mon.top {
                if ui.cursor().top() + 24.0 > floor { break; }
                let (row,_) = ui.allocate_exact_size(vec2(ui.available_width(),24.0),Sense::hover());
                let busy = process.cpu > ui::CAUTION_ABOVE * 100.0 / 2.0;
                ui.painter().text(row.left_center(),Align2::LEFT_CENTER,truncate(&process.name,16),ui::mono(11.5),ui::TEXT);
                ui.painter().text(row.right_center(),Align2::RIGHT_CENTER,format!("{:.0}%",process.cpu),ui::mono(11.5),if busy { ui::CAUTION } else { ui::SECONDARY });
            }
        });
    }
}

fn left_instruments(ui: &mut Ui, theme: &Theme, mon: &SysMon, widgets: &crate::widgets::Widgets, next: Option<&(String, chrono::NaiveDateTime)>) {
    // At the start of a session the column comes on with the rest of the frame.
    let stage = powerup::stage(ui.ctx());
    stage.apply(ui, Part::Left);
    ui.spacing_mut().item_spacing.y = 7.0;
    // The clock is sized for a full-width column and shrinks with it.
    clock(ui, theme, 64.0 * (ui.available_width() / 272.0).min(1.0));
    date_line(ui, mon, ui::display(12.5));
    if let Some(next) = next { next_event(ui, next); }
    ui.add_space(6.0);

    // Following a host through ssh: its name, and whether its readings are the ones shown.
    machine_card(ui, theme, mon);

    let load = mon.cpu_hist.back().copied().unwrap_or(0.0);
    card(ui, |ui| {
        card_head(ui, "PROCESSOR", |ui| {
            if let Some(average) = mon.load {
                // All three averages where they fit; the last minute's alone where not.
                let all = format!("LOAD {:.1} · {:.1} · {:.1}", average[0], average[1], average[2]);
                let fits = ui::galley(ui, &all, ui::mono(11.0), ui::LABEL, 0.0).size().x <= ui.available_width();
                let text = if fits { all } else { format!("LOAD {:.1}", average[0]) };
                ui::meta(ui, &text, ui::LABEL);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let hot = load > ui::CAUTION_ABOVE * 100.0;
            big_figure(ui, &format!("{load:.0}"), "%", if hot { ui::CAUTION } else { ui::TEXT });
            let width = ui.available_width();
            area_chart(ui, &stage.swept_values(&mon.cpu_hist).iter().copied().collect::<Vec<_>>(), 100.0, vec2(width, 38.0), theme.main);
        });
        // One bar per core, however many there are; the pointer reads any one of them.
        ui::bar_grid(ui, theme, &stage.swept_slice(&mon.cores), 100.0, 26.0, |i, usage| format!("Core {}: {usage:.0}%", i + 1));
        if let Some(gpu) = &mon.gpu {
            ui::meter(ui, theme, "GPU", 44.0, stage.swept(gpu.busy as f32 / 100.0), &format!("{}%", gpu.busy));
        }
    });

    card(ui, |ui| {
        card_head(ui, "MEMORY", |ui| { if mon.swap_total > 0 { ui::meta(ui, &format!("SWAP {:.1} GiB", mon.swap_used as f64 / GIB), ui::LABEL); } });
        let used = mon.mem_used as f32 / mon.mem_total.max(1) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            big_figure(ui, &format!("{:.1}", mon.mem_used as f64 / GIB), "", ui::TEXT);
            ui::text(ui, &format!("/ {:.1} GiB used", mon.mem_total as f64 / GIB), ui::mono(11.5), ui::SECONDARY);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui::text(ui, &format!("{:.0}%", used * 100.0), ui::mono(12.0), ui::TEXT); });
        });
        ui::bar(ui, theme, 8.0, stage.swept(used), 0.0);
    });

    // The instruments that need hardware to speak, and room to be seen.
    let temps = mon.temps.len().min(4);
    let fans = mon.fans.len().min(2);
    let thermal_height = if temps > 0 { CARD_CHROME + (temps + fans) as f32 * 23.0 } else { 0.0 };
    let filesystems = mon.filesystems.len().min(3);
    let storage_height = CARD_CHROME + filesystems as f32 * 23.0;
    // The user's own widgets come before the optional instruments, so they keep their room too.
    let widget_height = widgets.on(des_common::Panel::Left).count() as f32 * 60.0;
    let fitting = sections_that_fit(ui.available_height(), LEFT_RESERVE + widget_height, &[thermal_height, storage_height]);
    if temps > 0 && fitting >= 1 {
        let hottest = mon.temps[0].degrees;
        card(ui, |ui| {
            let hot = hottest >= 100.0 * ui::CAUTION_ABOVE;
            card_head(ui, "THERMAL", |ui| {
                if hot { pill(ui, &format!("{hottest:.0} °C · HOT"), ui::CAUTION); }
                else { ui::meta(ui, &format!("{hottest:.0} °C"), ui::LABEL); }
            });
            for reading in mon.temps.iter().take(temps) {
                ui::meter(ui, theme, &truncate(&reading.label, 7), 56.0, reading.degrees / 100.0, &format!("{:.0}°", reading.degrees));
            }
            for fan in mon.fans.iter().take(fans) {
                reading_row(ui, &truncate(&fan.label, 12), &if fan.rpm == 0 { "STILL".to_string() } else { format!("{} RPM", fan.rpm) }, ui::TEXT);
            }
        });
    }
    if fitting >= 2 {
        let read = mon.disk_read_hist.back().copied().unwrap_or(0.0);
        let written = mon.disk_write_hist.back().copied().unwrap_or(0.0);
        card(ui, |ui| {
            card_head(ui, "STORAGE", |ui| ui::meta(ui, &format!("R {}/s · W {}/s", human_bytes(read as f64), human_bytes(written as f64)), ui::LABEL));
            for fs in mon.filesystems.iter().take(filesystems) {
                ui::meter(ui, theme, &truncate(&fs.mount, 7), 56.0, stage.swept(fs.used), &format!("{:.0}%", fs.used * 100.0));
            }
        });
    }

    if widget_height > 0.0 && ui.available_height() > widget_height + LEFT_RESERVE {
        crate::widgets::show(ui, theme, widgets, des_common::Panel::Left);
    }

    // The list stops short of the companion's strip, so the animal always has its room.
    if ui.available_height() < CARD_CHROME + 26.0 { return; }
    let floor = ui.max_rect().bottom() - CARD_PAD - CARD_GAP;
    card(ui, |ui| {
        card_head(ui, "PROCESSES", |ui| ui::meta(ui, "CPU · MEMORY", ui::LABEL));
        ui.spacing_mut().item_spacing.y = 0.0;
        for process in &mon.top {
            if ui.cursor().top() + 26.0 > floor { break; }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
            let painter = ui.painter();
            let busy = process.cpu > ui::CAUTION_ABOVE * 100.0 / 2.0;
            let ink = if busy { ui::CAUTION } else { theme.main };
            painter.text(rect.left_center(), Align2::LEFT_CENTER, truncate(&process.name, 13), ui::mono(12.0), ui::TEXT);
            let memory_left = rect.right() - 64.0;
            painter.text(rect.right_center(), Align2::RIGHT_CENTER, human_bytes(process.mem as f64), ui::mono(11.5), ui::SECONDARY);
            painter.text(pos2(memory_left - 6.0, rect.center().y), Align2::RIGHT_CENTER, format!("{:.0}%", process.cpu), ui::mono(12.0), if busy { ui::CAUTION } else { ui::TEXT });
            let track = Rect::from_min_size(pos2(memory_left - 82.0, rect.center().y - 2.0), vec2(36.0, 4.0));
            painter.rect_filled(track, 2.0, ui::TRACK);
            painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * (process.cpu / 100.0).clamp(0.0, 1.0), 4.0)), 2.0, ink);
        }
    });
}

/// A large figure with a small unit after it, for the reading a card is about.
fn big_figure(ui: &mut Ui, value: &str, unit: &str, color: eframe::egui::Color32) {
    let number = ui::galley(ui, value, ui::display_bold(28.0), color, 0.0);
    let unit = ui::galley(ui, unit, ui::display(14.0), ui::SECONDARY, 0.0);
    let (rect, _) = ui.allocate_exact_size(vec2(number.size().x + unit.size().x + 2.0, 32.0), Sense::hover());
    ui.painter().galley(pos2(rect.left(), rect.bottom() - number.size().y), number.clone(), color);
    ui.painter().galley(pos2(rect.left() + number.size().x + 2.0, rect.bottom() - unit.size().y - 3.0), unit, ui::SECONDARY);
}

/// What each display is showing, for the right column.
fn display_tiles(ui: &mut Ui, theme: &Theme, desktop: &DesktopState) {
    let describe = |screen: usize| {
        let shown: Vec<&str> = desktop
            .apps
            .iter()
            .filter(|app| app.screen == screen && app.place != Place::Hidden)
            .map(|app| app.title.as_str())
            .collect();
        match (shown.as_slice(), screen) {
            ([], 0) => "Terminal".to_string(),
            ([], _) => "Idle".to_string(),
            ([one], 0) if desktop.terminal_split => format!("Terminal | {one}"),
            (titles, _) => titles.join(" | "),
        }
    };
    let focused_screen = match desktop.focus {
        Some(index) => desktop.apps.get(index).map_or(0, |app| app.screen),
        None => 0,
    };
    let draw = |ui: &mut Ui, screen: usize| {
        let active = screen == focused_screen;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
        crate::material::paint(ui.ctx(), ui.painter(), rect, ui::CONTROL_RADIUS, crate::material::Role::Panel);
        let painter = ui.painter_at(rect);
        painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, if active { theme.main } else { ui::LINE }), StrokeKind::Inside);
        let label = if screen == 0 { "1 · MAIN".to_string() } else { format!("{}", screen + 1) };
        painter.text(rect.min + vec2(12.0, 10.0), Align2::LEFT_TOP, label, ui::display(10.0), if active { theme.main } else { ui::LABEL });
        let full = describe(screen);
        let fit = ((rect.width() - 24.0).max(0.0) / 7.5) as usize;
        painter.text(rect.min + vec2(12.0, 29.0), Align2::LEFT_TOP, truncate(&full, fit), ui::mono(12.0), ui::TEXT);
        response.on_hover_text(full);
    };
    for pair in (0..desktop.screens).collect::<Vec<_>>().chunks(2) {
        match pair {
            [a, b] => {
                let (a, b) = (*a, *b);
                let width = (ui.available_width() - 8.0) / 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| draw(ui, a));
                    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| draw(ui, b));
                });
            }
            [a] => draw(ui, *a),
            _ => {}
        }
    }
}

/// Two or more choices side by side in a rounded well, one of them chosen. Returns
/// the one clicked.
pub fn tabs(ui: &mut Ui, theme: &Theme, labels: &[&str], chosen: usize) -> Option<usize> {
    let font = ui::display_bold(12.0);
    let galleys: Vec<_> = labels.iter().map(|label| ui::galley(ui, label, font.clone(), ui::SECONDARY, 1.2)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 28.0).collect();
    let (well, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 6.0 + 4.0 * (labels.len() as f32 - 1.0), 36.0), Sense::hover());
    ui.painter().rect_filled(well, ui::SURFACE_RADIUS, ui::TRACK);
    let mut x = well.left() + 3.0;
    let mut clicked = None;
    for (index, (galley, width)) in galleys.into_iter().zip(widths).enumerate() {
        let rect = Rect::from_min_size(pos2(x, well.top() + 3.0), vec2(width, 30.0));
        let response = ui.interact(rect, ui.id().with(("panel-tab", labels[index])), Sense::click());
        response.widget_info(|| eframe::egui::WidgetInfo::selected(eframe::egui::WidgetType::SelectableLabel, true, index == chosen, labels[index]));
        let on = index == chosen;
        if on { ui.painter().rect_filled(rect, ui::SURFACE_RADIUS - 2.0, theme.main.gamma_multiply(0.22)); ui.painter().rect_stroke(rect, ui::SURFACE_RADIUS - 2.0, Stroke::new(1.0, theme.main.gamma_multiply(0.6)), StrokeKind::Inside); }
        else if response.hovered() { ui.painter().rect_filled(rect, ui::SURFACE_RADIUS - 2.0, ui::LINE); }
        if response.has_focus() { ui.painter().rect_stroke(rect, ui::SURFACE_RADIUS - 2.0, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if on || response.hovered() { ui::TEXT } else { ui::SECONDARY });
        if response.clicked() { clicked = Some(index); }
        x += width + 4.0;
    }
    clicked
}

/// The network: on line or not, the interface and address, traffic both ways, and
/// on a tall panel how it went lately. Offline, a way to choose a network.
fn network_card(ui: &mut Ui, theme: &Theme, mon: &SysMon, chart: bool) -> bool {
    let stage = powerup::stage(ui.ctx());
    let online = !mon.ip.is_empty();
    let mut choose = false;
    card(ui, |ui| {
        card_head(ui, "NETWORK", |ui| { pill(ui, if online { "ONLINE" } else { "OFFLINE" }, if online { theme.main } else { ui::CAUTION }); });
        reading_row(ui, if mon.iface.is_empty() { "no interface" } else { &mon.iface }, if online { &mon.ip } else { "—" }, ui::TEXT);
        if !online {
            let link = ui.add(eframe::egui::Label::new(eframe::egui::RichText::new("Choose a network").font(ui::display(12.5)).color(theme.main)).sense(Sense::click()));
            choose = link.on_hover_cursor(eframe::egui::CursorIcon::PointingHand).clicked();
            return;
        }
        if chart {
            let peak = mon.rx_hist.iter().chain(mon.tx_hist.iter()).fold(1024.0_f32, |a, b| a.max(*b));
            ui::waterfall(ui, &stage.swept_values(&mon.rx_hist), &stage.swept_values(&mon.tx_hist), peak, vec2(ui.available_width(), 40.0), theme.main, ui::SECONDARY);
        }
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        let painter = ui.painter();
        ui::paint_icon(painter, Rect::from_center_size(rect.left_center() + vec2(6.0, 0.0), vec2(13.0, 13.0)), Icon::Download, theme.main, false);
        painter.text(rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, format!("{}/s", human_bytes(mon.rx_hist.back().copied().unwrap_or(0.0) as f64)), ui::mono(11.5), theme.main);
        painter.text(rect.right_center(), Align2::RIGHT_CENTER, format!("{}/s", human_bytes(mon.tx_hist.back().copied().unwrap_or(0.0) as f64)), ui::mono(11.5), ui::SECONDARY);
        let up = painter.layout_no_wrap(format!("{}/s", human_bytes(mon.tx_hist.back().copied().unwrap_or(0.0) as f64)), ui::mono(11.5), ui::SECONDARY);
        ui::paint_icon(painter, Rect::from_center_size(rect.right_center() - vec2(up.size().x + 10.0, 0.0), vec2(13.0, 13.0)), Icon::Upload, ui::SECONDARY, false);
        response.on_hover_text(format!("Downloaded {} · uploaded {}", human_bytes(mon.rx_total as f64), human_bytes(mon.tx_total as f64)));
    });
    choose
}

/// Rows of display tiles, two to a row.
fn display_rows(screens: usize) -> usize { screens.div_ceil(2) }
/// A display tile's height and the space under it.
const DISPLAY_ROW: f32 = 54.0 + 7.0;

/// The right column on the System tab: the quick settings `quick` draws, the
/// network, the world, the displays, and playback anchored to the floor. Optional
/// cards appear only where they fit whole, so none can push playback off the panel.
/// True when "Choose a network" was clicked.
pub fn right(
    ui: &mut Ui,
    theme: &Theme,
    mon: &SysMon,
    desktop: Option<&DesktopState>,
    world: &mut globe::Explorer,
    rotating: bool,
    geo: &Snapshot,
    widgets: &crate::widgets::Widgets,
    music_height: f32,
    quick: impl FnOnce(&mut Ui, f32),
    playing: impl FnOnce(&mut Ui),
) -> bool {
    let stage = powerup::stage(ui.ctx());
    stage.apply(ui, Part::Right);
    let area = ui.available_rect_before_wrap();
    let music = music_height.max(0.0).min(area.height());
    if music > 0.0 {
        let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - music), area.max);
        let mut child = ui.new_child(eframe::egui::UiBuilder::new().id_salt("right-playback").max_rect(rect).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        playing(&mut child);
    }
    let top = Rect::from_min_max(area.min, pos2(area.right(), (area.bottom() - music - if music > 0.0 { 8.0 } else { 0.0 }).max(area.top())));
    let mut column = ui.new_child(eframe::egui::UiBuilder::new().id_salt("right-cards").max_rect(top).layout(Layout::top_down(Align::Min)));
    column.set_clip_rect(top.intersect(ui.clip_rect()));
    let ui = &mut column;
    ui.spacing_mut().item_spacing.y = 7.0;
    let left = |ui: &Ui| top.bottom() - ui.cursor().top();

    let room = left(ui);
    quick(ui, room);

    let mut choose = false;
    let network_short = CARD_CHROME + 16.0 + 7.0 + 16.0;
    if left(ui) >= network_short {
        choose = network_card(ui, theme, mon, top.height() >= 640.0);
    }

    let screens = desktop.map_or(0, |desktop| desktop.screens);
    let displays_height = if screens == 0 { 0.0 } else if screens <= 2 { CARD_CHROME + display_rows(screens) as f32 * DISPLAY_ROW } else { CARD_CHROME + 16.0 };
    let widget_count = widgets.on(des_common::Panel::Right).count();
    let widget_height = widget_count as f32 * 100.0;
    let label = if geo.home.is_some() { 22.0 } else { 0.0 };
    let globe_room = left(ui) - displays_height - CARD_CHROME - label;
    if widget_count > 0 && globe_room - widget_height >= 96.0 {
        let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), widget_height));
        let mut child = ui.new_child(eframe::egui::UiBuilder::new().id_salt("right-optional-widgets").max_rect(rect).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        crate::widgets::show(&mut child, theme, widgets, des_common::Panel::Right);
        ui.add_space(widget_height + CARD_GAP);
    }
    let globe_room = left(ui) - displays_height - CARD_CHROME - label;
    let size = globe_room.min(ui.available_width() - 2.0 * CARD_PAD).min(224.0);
    if size >= 96.0 {
        card(ui, |ui| {
            card_head(ui, "WORLD VIEW", |ui| { if !geo.links.is_empty() { ui::meta(ui, &format!("{} LINKS", geo.links.len()), ui::LABEL); } });
            ui.vertical_centered(|ui| world.panel(ui, theme, size, geo, rotating));
            if let Some(home) = &geo.home {
                let place = format!("{} · {}", home.label.to_uppercase(), if geo.home_is_set { "SET BY YOU" } else { "BY IP" });
                ui::text(ui, &truncate(&place, ((ui.available_width() / 7.0) as usize).max(8)), ui::mono(10.5), ui::SECONDARY).on_hover_text(&home.label);
            }
        });
    }
    if let Some(desktop) = desktop.filter(|_| screens > 0 && left(ui) >= displays_height) {
        card(ui, |ui| {
            card_head(ui, "DISPLAYS", |ui| ui::meta(ui, &format!("{} CONNECTED", desktop.screens), ui::LABEL));
            if screens <= 2 {
                display_tiles(ui, theme, desktop);
            } else {
                let summary = (0..desktop.screens).map(|index| format!("Display {}", index + 1)).collect::<Vec<_>>().join(" · ");
                ui::text(ui, &truncate(&summary, ((ui.available_width() / 7.0) as usize).max(8)), ui::mono(11.0), ui::SECONDARY).on_hover_text(summary);
            }
        });
    }
    choose
}

#[cfg(test)]
mod status_island_tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn levels_move_in_steps() {
        assert_eq!([0, 33, 34, 66, 67, 100].map(ui::wifi_bars), [1, 1, 2, 2, 3, 3]);
        assert_eq!([0, 1, 33, 34, 66, 67, 100].map(ui::volume_waves), [0, 1, 1, 2, 2, 3, 3]);
        assert_eq!([0, 1, 9, 10, 11, 55, 99, 100, 120].map(ui::battery_tenths), [0, 1, 1, 1, 2, 6, 10, 10, 10]);
    }

    #[test]
    fn wired_means_up_and_not_wireless() {
        assert!(wired("enp0s4") && wired("eth0"));
        assert!(!wired("") && !wired("wlp2s0") && !wired("lo"));
    }

    /// Draws the islands across a screen `width` wide and returns where they went,
    /// after a second frame so the centre island has its own width.
    fn islands(width: f32, playing: bool, clock: bool) -> (Rect, [Rect; 3]) {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        let mut mon = SysMon::new();
        mon.wifi = Some(80);
        mon.power = Some(crate::sysmon::Battery { percent: 14, plugged: false, minutes: Some(81) });
        let band = Rect::from_min_size(pos2(0.0, 1080.0 - des_common::STATUS_HEIGHT), vec2(width, des_common::STATUS_HEIGHT));
        let desktop = DesktopState { screens: 1, ..Default::default() };
        let mut found = [Rect::NOTHING; 3];
        for _ in 0..4 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 1080.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band).layout(Layout::left_to_right(Align::Center)));
                    let panels = Panels { left: true, right: true, bottom: true };
                    let clicks = status(&mut child, &theme, &mon, Some(&desktop), panels, None, 3, None, des_common::ipc::Hands::Off, None, false, false,
                        playing.then_some(("Midnight City", "M83")), clock);
                    found = clicks.islands;
                });
            });
        }
        (band, found)
    }

    #[test]
    fn islands_float_apart_inside_the_band_at_every_width() {
        for width in [1024.0, 1280.0, 1366.0, 1920.0, 3840.0] {
            for playing in [false, true] {
                for clock in [false, true] {
                    let (band, [left, centre, right]) = islands(width, playing, clock);
                    let case = format!("{width} wide, playing {playing}, clock {clock}");
                    let drawn: Vec<Rect> = [left, centre, right].into_iter().filter(|island| *island != Rect::NOTHING).collect();
                    assert!(drawn.len() >= 2 && left != Rect::NOTHING && right != Rect::NOTHING, "{case}");
                    if width >= 1280.0 { assert_eq!(drawn.len(), 3, "{case}: the centre fits"); }
                    for island in drawn {
                        assert_eq!(island.height(), ISLAND_HEIGHT, "{case}");
                        assert_eq!(island.top(), band.top(), "{case}");
                        assert_eq!(band.bottom() - island.bottom(), des_common::frame::GAP, "{case}: floats above the screen edge");
                        assert!(island.width() > ISLAND_HEIGHT, "{case}: {island:?} has contents");
                    }
                    assert_eq!(left.left(), band.left() + des_common::frame::GAP, "{case}");
                    assert_eq!(right.right(), band.right() - des_common::frame::GAP, "{case}");
                    if centre != Rect::NOTHING {
                        assert!(left.right() + des_common::frame::GAP <= centre.left() + 0.5, "{case}: left {left:?} centre {centre:?}");
                        assert!(centre.right() + des_common::frame::GAP <= right.left() + 0.5, "{case}: centre {centre:?} right {right:?}");
                    }
                    assert!(left.right() + des_common::frame::GAP <= right.left() + 0.5, "{case}");
                    if width >= 1920.0 {
                        assert!((centre.center().x - band.center().x).abs() < 1.0, "{case}: centred when there is room");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod right_column_tests {
    use super::*;
    use eframe::egui;

    /// Draws the column into `area` with a stand-in quick settings card of `quick`
    /// points, and returns where each piece of text was painted.
    fn painted(area: Rect, quick: f32, music: f32, screens: usize) -> Vec<(String, Rect)> {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        ctx.style_mut(|style| style.spacing.interact_size = vec2(30.0, 30.0));
        let theme = Theme::load("signal");
        let mon = SysMon::new();
        let widgets = crate::widgets::Widgets::new(ctx.clone());
        let mut world = globe::Explorer::default();
        let geo = Snapshot { home: Some(crate::geo::Place { lat: 5.6, lon: -0.2, label: "Accra".into() }), ..Default::default() };
        let output = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), area.max.to_vec2() + vec2(20.0, 20.0))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut column = ui.new_child(egui::UiBuilder::new().max_rect(area));
                column.set_clip_rect(area);
                right(&mut column, &theme, &mon, Some(&DesktopState { screens, ..Default::default() }), &mut world, false, &geo, &widgets, music,
                    |ui, room| { card(ui, |ui| { ui.allocate_exact_size(vec2(10.0, quick.min(room - 60.0).max(0.0)), Sense::hover()); ui::text(ui, "QUICK", ui::mono(10.0), ui::TEXT); }); },
                    |ui| { ui::text(ui, "PLAYBACK", ui::mono(10.0), ui::TEXT); });
            });
        });
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some((text.galley.job.text.clone(), Rect::from_min_size(text.pos, text.galley.size()))),
            _ => None,
        }).collect()
    }

    #[test]
    fn cards_never_reach_into_playback_and_drop_whole_when_short() {
        for height in [300.0, 480.0, 720.0, 1000.0] {
            for width in [192.0, 292.0] {
                for music in [0.0, crate::music::PANEL_HEIGHT] {
                    for screens in [1, 2, 6] {
                        let area = Rect::from_min_size(pos2(10.0, 10.0), vec2(width, height));
                        let texts = painted(area, 150.0, music, screens);
                        let floor = area.bottom() - music - if music > 0.0 { 8.0 } else { 0.0 };
                        for (text, rect) in &texts {
                            if text == "PLAYBACK" { continue; }
                            assert!(rect.bottom() <= floor + 0.5, "{text:?} at {rect:?} crosses {floor} at {width}x{height}, music {music}, {screens} screens");
                            assert!(rect.right() <= area.right() + 0.5, "{text:?} too wide at {width}");
                        }
                        let has = |needle: &str| texts.iter().any(|(text, _)| text == needle);
                        assert!(has("QUICK"));
                        if height >= 480.0 { assert!(has("NETWORK"), "network missing at {width}x{height}"); }
                        if height >= 1000.0 { assert!(has("DISPLAYS"), "displays missing at {width}x{height}"); assert!(has("WORLD VIEW")); }
                        if music > 0.0 { assert!(has("PLAYBACK")); }
                    }
                }
            }
        }
    }
}

/// A piece of a pill in the status strip: an icon, a glyph that shows a level, or
/// text in a colour.
enum Piece { Icon(Icon, eframe::egui::Color32), Glyph(Glyph, eframe::egui::Color32), Text(String, eframe::egui::Color32) }

/// The status glyphs that show their state.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Glyph { Wifi(u8), Wired, Volume { waves: u8, muted: bool }, Battery { percent: u8, charging: bool, full: bool } }

impl Piece {
    fn width(&self, galley: Option<&eframe::egui::Galley>) -> f32 {
        match self {
            Piece::Icon(..) => 14.0,
            Piece::Glyph(Glyph::Battery { .. }, _) => 28.0,
            Piece::Glyph(Glyph::Volume { .. }, _) => 20.0,
            Piece::Glyph(..) => 22.0,
            Piece::Text(..) => galley.map_or(0.0, |g| g.size().x),
        }
    }
}

/// A rounded pill of icons and text, one control. `edge` and `fill` say how it stands.
fn strip_pill(ui: &mut Ui, pieces: &[Piece], edge: eframe::egui::Color32, fill: eframe::egui::Color32, label: &str) -> eframe::egui::Response {
    let galleys: Vec<Option<std::sync::Arc<eframe::egui::Galley>>> = pieces.iter().map(|piece| match piece {
        Piece::Text(text, color) => Some(ui.painter().layout_no_wrap(text.clone(), ui::mono(11.0), *color)),
        _ => None,
    }).collect();
    let gap = 5.0;
    let width: f32 = pieces.iter().zip(&galleys).map(|(piece, galley)| piece.width(galley.as_deref())).sum::<f32>()
        + gap * (pieces.len().saturating_sub(1)) as f32 + 20.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let response = response.on_hover_text(label);
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, label));
    let painter = ui.painter();
    painter.rect_filled(rect, 13.0, if response.hovered() { ui::TRACK } else { fill });
    painter.rect_stroke(rect, 13.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    paint_pieces(painter, pieces, galleys, rect.left() + 10.0, rect.center().y, gap);
    response
}

/// The status islands along the bottom edge, floating like the panels. Left: the
/// ways in. Centre: what has the keyboard and what plays. Right: applications'
/// own icons, the panels, one run of readings that opens the quick settings, and
/// the time when nothing else on screen shows it. `overlay` names what is shown in
/// place of the terminal. `ui.max_rect()` is the whole band, its gap included.
#[allow(clippy::too_many_arguments)]
pub fn status(
    ui: &mut Ui,
    theme: &Theme,
    mon: &SysMon,
    desktop: Option<&DesktopState>,
    panels: Panels,
    overlay: Option<&str>,
    notices: usize,
    recording: Option<std::time::Duration>,
    hands: des_common::ipc::Hands,
    tray: Option<&crate::tray::Tray>,
    keyboard_open: bool,
    quick_open: bool,
    playing: Option<(&str, &str)>,
    clock: bool,
) -> StatusClicks {
    let stage = powerup::stage(ui.ctx());
    if stage.alpha(Part::Status) < 1.0 {
        ui.set_opacity(stage.alpha(Part::Status));
    }
    let band = ui.max_rect();
    let gap = des_common::frame::GAP;
    let row = Rect::from_min_size(pos2(band.left() + gap, band.top()), vec2((band.width() - 2.0 * gap).max(1.0), ISLAND_HEIGHT));
    let mut clicks = StatusClicks::default();

    // Left: the ways in, each lit while what it opens is open.
    let left = island(ui, row, Layout::left_to_right(Align::Center), ISLAND_PADDING, |ui| {
        ui.spacing_mut().item_spacing.x = ICON_GAP;
        ui::tracked(ui, "DEs-UI", ui::display_bold(11.0), theme.main, 3.0);
        ui.add_space(18.0 - ICON_GAP);
        let context = ui.ctx().clone();
        let way_in = |ui: &mut Ui, name: &str, icon: Icon, hint: &str, id: &str| {
            ui::round_icon_button(ui, theme, icon, BUTTON, overlay == Some(name)).on_hover_text(crate::shortcuts::hint(&context, hint, id))
        };
        clicks.settings = way_in(ui, "SETTINGS", Icon::Settings, "Settings", "desktop.settings").clicked();
        clicks.music = way_in(ui, "MUSIC", Icon::Music, "Music", "desktop.music").clicked();
        clicks.power = way_in(ui, "POWER", Icon::Power, "Sleep, restart, shut down", "desktop.power").clicked();
        let bell = way_in(ui, "NOTIFICATIONS", Icon::Notifications, "Notifications", "desktop.notifications");
        clicks.notices = bell.clicked();
        if notices > 0 {
            // How many wait, as a badge on the bell.
            let text = if notices > 99 { "99+".to_string() } else { notices.to_string() };
            let galley = ui.painter().layout_no_wrap(text, ui::mono(9.0), ui::GROUND);
            let size = vec2((galley.size().x + 6.0).max(15.0), 15.0);
            let badge = Rect::from_min_size(pos2(bell.rect.right() - size.x * 0.7, bell.rect.top() + 1.0), size);
            ui.painter().rect_filled(badge, 7.5, theme.main);
            ui.painter().galley(badge.center() - galley.size() / 2.0, galley, ui::GROUND);
        }
    });

    // Right, laid out from the right edge inwards.
    let right = island(ui, row, Layout::right_to_left(Align::Center), ISLAND_PADDING, |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if clock {
            ui::text(ui, &crate::region::hours_minutes(&chrono::Local::now()), ui::display_bold(14.0), ui::TEXT);
            separator(ui);
        }
        // One run of readings; it opens the quick settings, and is lit while they are open.
        let mut pieces = Vec::new();
        let mut said = Vec::new();
        if let Some(wifi) = mon.wifi {
            pieces.push(Piece::Glyph(Glyph::Wifi(ui::wifi_bars(wifi)), ui::TEXT));
            said.push(format!("Wi-Fi {wifi}%"));
        } else if wired(&mon.iface) {
            pieces.push(Piece::Glyph(Glyph::Wired, ui::TEXT));
            said.push("wired network".to_string());
        }
        if let Some(volume) = mon.volume() {
            pieces.push(Piece::Glyph(Glyph::Volume { waves: ui::volume_waves(volume), muted: volume == 0 }, ui::TEXT));
            pieces.push(Piece::Text(format!("{volume}%"), ui::TEXT));
            said.push(format!("volume {volume}%"));
        }
        if let Some(battery) = mon.power {
            // Low and running down is the one reading here that asks for something.
            let charging = battery.plugged && battery.percent < 100;
            let ink = if !battery.plugged && battery.percent < 20 { ui::CAUTION } else if charging { theme.main } else { ui::TEXT };
            pieces.push(Piece::Glyph(Glyph::Battery { percent: battery.percent, charging, full: battery.plugged && battery.percent >= 100 }, ink));
            pieces.push(Piece::Text(format!("{}%", battery.percent), ink));
            if let Some(minutes) = battery.minutes {
                // "1 H 21" with hours; minutes alone keep their unit.
                let left = crate::power::duration(minutes);
                let left = if left.contains(" H ") { left.replace(" MIN", "") } else { left };
                pieces.push(Piece::Text(format!("· {left}"), if ink == ui::CAUTION { ink } else { ui::LABEL }));
            }
            said.push(crate::power::battery_line(battery));
        }
        if pieces.is_empty() { pieces.push(Piece::Icon(Icon::Quick, ui::TEXT)); }
        let label = crate::shortcuts::hint(ui.ctx(), &format!("Quick settings{}{}", if said.is_empty() { "" } else { ": " }, said.join(", ")), "desktop.quick");
        clicks.quick = readings(ui, theme, &pieces, quick_open, &label).clicked();
        separator(ui);
        // The panels' switches, together.
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (icon, panel, open) in [(Icon::PanelRight, Panel::Right, panels.right), (Icon::PanelBottom, Panel::Bottom, panels.bottom), (Icon::PanelLeft, Panel::Left, panels.left)] {
                let name = match panel { Panel::Left => "left", Panel::Right => "right", Panel::Bottom => "file" };
                let action = if open { "Collapse" } else { "Open" };
                if ui::round_icon_button(ui, theme, icon, TOGGLE, open).on_hover_text(format!("{action} the {name} panel")).clicked() {
                    clicks.toggled = Some(panel);
                }
            }
        });
        clicks.keyboard = ui::round_icon_button(ui, theme, Icon::Keyboard, TOGGLE, keyboard_open)
            .on_hover_text(crate::shortcuts::hint(ui.ctx(), "On-screen keyboard", "shell.keyboard"))
            .clicked();
        // While hand control is on, the camera is: said plainly, and one click from off.
        let hand = match hands {
            des_common::ipc::Hands::Off => None,
            des_common::ipc::Hands::Watching => Some(("CAMERA ON", ui::SECONDARY, "The camera is on for hand control, waiting for a hand. Click to turn it off.")),
            des_common::ipc::Hands::Following => Some(("CAMERA ON · HAND", theme.main, "The camera is on, and your hand moves the pointer. Click to turn it off.")),
            des_common::ipc::Hands::Failed => Some(("HAND CONTROL STOPPED", ui::CAUTION, "Hand control could not go on: is the camera there, and free? Click to try again.")),
        };
        if let Some((text, ink, hint)) = hand {
            let label = crate::shortcuts::hint(ui.ctx(), hint, "desktop.hands");
            clicks.hands = strip_pill(ui, &[Piece::Text(format!("● {text}"), ink)], ink, ui::PANEL, &label).clicked();
        }
        if let Some(running) = recording {
            // The one thing here that is happening to the user, so it is the one in colour.
            let seconds = running.as_secs();
            let pieces = [Piece::Text(format!("● REC {:02}:{:02}", seconds / 60, seconds % 60), ui::CAUTION), Piece::Text("· STOP".into(), ui::CAUTION.gamma_multiply(0.75))];
            clicks.stop = strip_pill(ui, &pieces, ui::CAUTION, ui::CAUTION.gamma_multiply(0.08), "End the recording").clicked();
        }
        // Applications' own icons, left of the rest.
        if let Some(tray) = tray {
            ui.spacing_mut().item_spacing.x = ICON_GAP;
            crate::tray::show(ui, theme, tray);
        }
    });

    // Centre: placed on the middle of the screen when it fits, using last frame's
    // width, and never over its neighbours.
    // Measured widths of the full and the compact centre, from earlier frames: the
    // compact one leaves out what plays and the counts, and with no room for even
    // that the centre is left out.
    let width_id = ui.id().with("status-centre-width");
    let (full, compact): (f32, f32) = ui.ctx().data(|data| data.get_temp(width_id)).unwrap_or((240.0, 160.0));
    let lo = left.right() + gap;
    let hi = (right.left() - gap).max(lo);
    let fits = |width: f32| width <= hi - lo;
    let short = !fits(full);
    let width = if short { compact } else { full };
    let x = (row.center().x - width / 2.0).clamp(lo, (hi - width).max(lo));
    let room = Rect::from_min_max(pos2(x, row.top()), pos2(hi.max(x + 1.0), row.bottom()));
    let centre = if short && !fits(compact) { Rect::NOTHING } else { island(ui, room, Layout::left_to_right(Align::Center), 20.0, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let holder = match (overlay, desktop.and_then(|d| d.focus.and_then(|index| d.apps.get(index)))) {
            (Some(overlay), _) => overlay.to_string(),
            (None, Some(app)) => truncate(&app.title, 28).to_uppercase(),
            (None, None) => "TERMINAL".to_string(),
        };
        ui::icon(ui, Icon::Keyboard, 16.0, ui::SECONDARY).on_hover_text(format!("{} has the keyboard", holder.to_lowercase()));
        ui::text(ui, &holder, ui::display_bold(12.0), ui::TEXT);
        if short { return; }
        if let Some(count) = desktop.map(|desktop| desktop.apps.len()) {
            ui::text(ui, &format!("· {count} {}", if count == 1 { "APP" } else { "APPS" }), ui::mono(11.0), ui::SECONDARY);
        }
        if desktop.is_some_and(|desktop| desktop.screens > 1) {
            ui::text(ui, &format!("· {} DISPLAYS", desktop.map_or(0, |d| d.screens)), ui::mono(11.0), ui::SECONDARY);
        }
        // What plays, as a way into Music.
        if let Some((track, artists)) = playing {
            separator(ui);
            let pieces = [Piece::Icon(Icon::Music, theme.main), Piece::Text(truncate(track, 28), ui::TEXT), Piece::Text(format!("· {}", truncate(artists, 22)), ui::SECONDARY)];
            clicks.music |= readings(ui, theme, &pieces, false, &format!("{track} — {artists}. Open Music")).clicked();
        }
    }) };
    // Remember what was measured; a changed width redraws once more to place it.
    if centre.width() > 0.0 && (centre.width() - width).abs() > 0.5 {
        let measured = if short { (full, centre.width()) } else { (centre.width(), compact) };
        ui.ctx().data_mut(|data| data.insert_temp(width_id, measured));
        ui.ctx().request_repaint();
    }
    clicks.islands = [left, centre, right];

    if clock {
        use chrono::Timelike;
        ui.ctx().request_repaint_after(Duration::from_secs((60 - chrono::Local::now().second()) as u64));
    }
    clicks
}

/// Height of each status island, inside `des_common::STATUS_HEIGHT`.
pub const ISLAND_HEIGHT: f32 = des_common::STATUS_ISLAND_HEIGHT;
const ISLAND_PADDING: f32 = 14.0;
/// Round buttons in the islands, and the room between them.
const BUTTON: f32 = 36.0;
const TOGGLE: f32 = 32.0;
const ICON_GAP: f32 = 10.0;

/// A rounded island in `area` sized to its contents, drawn with the panels' own
/// surface, edge and shadow behind them. Returns where it was drawn.
fn island(ui: &mut Ui, area: Rect, layout: Layout, padding: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    let background = ui.painter().add(eframe::egui::Shape::Noop);
    let inner = area.shrink2(vec2(padding, 0.0));
    let mut child = ui.new_child(eframe::egui::UiBuilder::new().max_rect(inner).layout(layout));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    add(&mut child);
    let used = child.min_rect();
    let rect = if used.width() > 0.0 {
        Rect::from_min_max(pos2(used.left() - padding, area.top()), pos2(used.right() + padding, area.bottom()))
    } else {
        Rect::from_min_size(area.min, vec2(0.0, area.height()))
    };
    let radius = rect.height() / 2.0;
    let mut shapes = vec![eframe::egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: eframe::egui::Color32::from_black_alpha(110) }.as_shape(rect, radius).into()];
    shapes.extend(crate::material::shapes(ui.ctx(), rect, radius, crate::material::Role::Panel));
    shapes.push(eframe::egui::Shape::rect_stroke(rect, radius, Stroke::new(1.0, ui::LINE), StrokeKind::Inside));
    ui.painter().set(background, eframe::egui::Shape::Vec(shapes));
    rect
}

/// A wired connection: an interface that is up and is not wireless.
fn wired(iface: &str) -> bool {
    !iface.is_empty() && !iface.starts_with("wl") && iface != "lo"
}

/// A run of readings or of what plays: one control, a soft pill only while
/// hovered or `lit`.
fn readings(ui: &mut Ui, theme: &Theme, pieces: &[Piece], lit: bool, label: &str) -> eframe::egui::Response {
    let galleys: Vec<Option<std::sync::Arc<eframe::egui::Galley>>> = pieces.iter().map(|piece| match piece {
        Piece::Text(text, color) => Some(ui.painter().layout_no_wrap(text.clone(), ui::mono(12.0), *color)),
        _ => None,
    }).collect();
    let gap = 6.0;
    let width: f32 = pieces.iter().zip(&galleys).map(|(piece, galley)| piece.width(galley.as_deref())).sum::<f32>()
        + gap * (pieces.len().saturating_sub(1)) as f32 + 24.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, TOGGLE), Sense::click());
    let response = response.on_hover_text(label);
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, label));
    let painter = ui.painter();
    if lit || response.hovered() {
        painter.rect_filled(rect, TOGGLE / 2.0, if lit { theme.tint() } else { ui::TRACK });
    }
    if response.has_focus() {
        painter.rect_stroke(rect, TOGGLE / 2.0, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    }
    paint_pieces(painter, pieces, galleys, rect.left() + 12.0, rect.center().y, gap);
    response
}

/// Paints `pieces` from `x` along the line at `y`.
fn paint_pieces(painter: &eframe::egui::Painter, pieces: &[Piece], galleys: Vec<Option<std::sync::Arc<eframe::egui::Galley>>>, mut x: f32, y: f32, gap: f32) {
    for (piece, galley) in pieces.iter().zip(galleys) {
        let width = piece.width(galley.as_deref());
        let c = pos2(x + width / 2.0, y);
        match piece {
            Piece::Icon(icon, color) => ui::paint_icon(painter, Rect::from_center_size(c, vec2(14.0, 14.0)), *icon, *color, false),
            Piece::Glyph(Glyph::Wifi(bars), color) => ui::paint_wifi(painter, c, *color, *bars, 1.0),
            Piece::Glyph(Glyph::Wired, color) => ui::paint_wired(painter, c, *color),
            Piece::Glyph(Glyph::Volume { waves, muted }, color) => ui::paint_volume(painter, c, *color, *waves, *muted),
            Piece::Glyph(Glyph::Battery { percent, charging, full }, color) => ui::paint_battery(painter, c, *color, *percent, *charging, *full, 1.0),
            Piece::Text(_, color) => if let Some(galley) = galley {
                painter.galley(pos2(x, y - galley.size().y / 2.0), galley, *color);
            },
        }
        x += width + gap;
    }
}

/// A short upright rule between groups in the strip.
fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, ui::LINE));
}

/// What was clicked in the status strip this frame, and where its islands were drawn.
pub struct StatusClicks {
    pub toggled: Option<Panel>,
    pub settings: bool,
    pub music: bool,
    pub notices: bool,
    pub power: bool,
    pub stop: bool,
    /// The hand control indicator.
    pub hands: bool,
    pub keyboard: bool,
    pub quick: bool,
    /// Left, centre and right.
    pub islands: [Rect; 3],
}

impl Default for StatusClicks {
    fn default() -> Self {
        Self { toggled: None, settings: false, music: false, notices: false, power: false, stop: false, hands: false, keyboard: false, quick: false, islands: [Rect::NOTHING; 3] }
    }
}

/// What a further display shows while no application is on it.
pub fn backdrop(ui: &mut Ui, theme: &Theme, mon: &SysMon, number: usize) {
    let rect = ui.max_rect();
    crate::material::paint(ui.ctx(), ui.painter(), rect, 0.0, crate::material::Role::Background);
    ui::corners(ui.painter(), rect.shrink(24.0), 28.0, Stroke::new(2.0, ui::LINE_STRONG));

    let inner = rect.shrink2(vec2(72.0, 48.0));
    let painter = ui.painter().clone();
    let brand = ui::galley(ui, "DEs-UI", ui::display_bold(14.0), theme.main, 4.0);
    let brand_width = brand.size().x;
    painter.galley(inner.left_top(), brand, theme.main);
    let label = ui::galley(ui, &format!("DISPLAY {number}"), ui::display(14.0), ui::SECONDARY, 4.0);
    painter.galley(inner.left_top() + vec2(brand_width + 14.0, 0.0), label, ui::SECONDARY);
    painter.text(
        inner.right_top(),
        Align2::RIGHT_TOP,
        format!("{:.0} × {:.0}", rect.width(), rect.height()),
        ui::mono(12.0),
        ui::SECONDARY,
    );

    // The clock takes about a third of the height, and never more than the width allows.
    let now = chrono::Local::now();
    let size = (rect.height() * 0.2).min(rect.width() * 0.16);
    let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display(size), ui::TEXT, 4.0);
    let seconds = ui::galley(ui, &now.format("%S").to_string(), ui::display(size * 0.3), theme.main, 0.0);
    let total = time.size().x + size * 0.08 + seconds.size().x;
    let top = rect.center().y - time.size().y * 0.62;
    let left = rect.center().x - total / 2.0;
    let seconds_pos = pos2(left + time.size().x + size * 0.08, top + time.size().y * 0.86 - seconds.size().y);
    let time_bottom = top + time.size().y;
    painter.galley(pos2(left, top), time, ui::TEXT);
    painter.galley(seconds_pos, seconds, theme.main);
    let date = now.format("%A %d %B %Y").to_string().to_uppercase();
    let date = ui::galley(ui, &date, ui::display(size * 0.11), ui::SECONDARY, size * 0.035);
    let date_height = date.size().y;
    painter.galley(pos2(rect.center().x - date.size().x / 2.0, time_bottom), date, ui::SECONDARY);
    let hint_y = time_bottom + date_height + 40.0;
    painter.hline(
        (rect.center().x - 260.0)..=(rect.center().x + 260.0),
        hint_y - 20.0,
        Stroke::new(1.0, ui::LINE),
    );
    // One line where the display is wide enough for it, two where it is not.
    let hints = ["SUPER O  sends the current application here", "SUPER SPACE  opens the launcher"];
    if rect.width() >= 1100.0 {
        painter.text(pos2(rect.center().x, hint_y), Align2::CENTER_TOP, hints.join("      "), ui::mono(13.0), ui::SECONDARY);
    } else {
        for (row, hint) in hints.iter().enumerate() {
            let at = pos2(rect.center().x, hint_y + row as f32 * 22.0);
            painter.text(at, Align2::CENTER_TOP, hint, ui::mono(13.0), ui::SECONDARY);
        }
    }

    // Four readings along the bottom.
    let load = mon.cpu_hist.back().copied().unwrap_or(0.0);
    let memory = mon.mem_used as f32 / mon.mem_total.max(1) as f32;
    let down = mon.rx_hist.back().copied().unwrap_or(0.0);
    let peak = mon.rx_hist.iter().fold(1024.0_f32, |a, b| a.max(*b));
    let tiles = [
        ("PROCESSOR", format!("{load:.0}%"), load / 100.0, true),
        ("MEMORY", format!("{:.1} GiB", mon.mem_used as f64 / GIB), memory, true),
        ("NETWORK", format!("{}/s", human_bytes(down as f64)), down / peak, false),
        (
            "BATTERY",
            mon.battery.map_or("AC".to_string(), |b| format!("{b}%")),
            mon.battery.map_or(1.0, |b| b as f32 / 100.0),
            false,
        ),
    ];
    let gap = 16.0;
    let width = (inner.width() - gap * 3.0) / 4.0;
    for (i, (label, reading, frac, is_load)) in tiles.into_iter().enumerate() {
        let tile = Rect::from_min_size(
            pos2(inner.left() + i as f32 * (width + gap), inner.bottom() - 58.0),
            vec2(width, 58.0),
        );
        crate::material::paint(ui.ctx(), &painter, tile, ui::CONTROL_RADIUS, crate::material::Role::Panel);
        painter.rect_stroke(tile, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let name = ui::galley(ui, label, ui::display_bold(12.0), ui::SECONDARY, 0.8);
        painter.galley(tile.left_top() + vec2(16.0, 13.0), name, ui::SECONDARY);
        if width >= 200.0 {
            painter.text(tile.right_top() + vec2(-16.0, 13.0), Align2::RIGHT_TOP, reading, ui::mono(12.0), ui::TEXT);
        }
        let track = Rect::from_min_size(tile.left_top() + vec2(16.0, 38.0), vec2(width - 32.0, 6.0));
        painter.rect_filled(track, 0.0, ui::TRACK);
        let frac = frac.clamp(0.0, 1.0);
        let mut filled = track;
        filled.set_width(track.width() * frac);
        let color = if is_load && frac > ui::CAUTION_ABOVE { ui::CAUTION } else { theme.main };
        painter.rect_filled(filled, 0.0, color);
    }
}

/// Quotes `s` so a POSIX shell reads it back as a single word.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A directory listing that follows the terminal's working directory.
pub struct FsView {
    path: PathBuf,
    entries: Vec<(String, bool)>,
    /// How big each file is, by name.
    sizes: std::collections::HashMap<String, u64>,
    loaded: Option<Instant>,
    /// The file picked by a click, which the keys and quick-look act on.
    selected: Option<String>,
    /// Whether the panel has the keyboard: after a click on a file, until Escape or a
    /// click elsewhere.
    focused: bool,
    /// Tiles in a row, as last laid out, for Up and Down.
    columns: usize,
    /// The preview over the workspace.
    pub look: crate::quicklook::QuickLook,
    /// A file dragged out of the panel, for the compositor to carry; once per drag.
    drag: Option<PathBuf>,
    dragging: bool,
    /// Set when a preview opens, so the terminal is brought into view under it.
    uncover: bool,
}

/// What a key does to the file panel while it has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsKey {
    Step(isize),
    Look,
    Open,
    Escape,
}

/// The entry `step` away from `at`, kept inside `len`.
pub fn stepped(at: Option<usize>, step: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(at) = at else { return Some(0) };
    Some((at as isize + step).clamp(0, len as isize - 1) as usize)
}

impl FsView {
    pub fn new() -> FsView {
        FsView {
            path: PathBuf::new(),
            entries: Vec::new(),
            sizes: std::collections::HashMap::new(),
            loaded: None,
            selected: None,
            focused: false,
            columns: 1,
            look: crate::quicklook::QuickLook::default(),
            drag: None,
            dragging: false,
            uncover: false,
        }
    }

    /// Whether the panel or its preview has the keyboard, which the terminal then does not.
    pub fn has_keyboard(&self) -> bool {
        self.focused || self.look.is_open()
    }

    /// Lets go of the keyboard and closes the preview, as when the panel closes.
    pub fn blur(&mut self) {
        self.focused = false;
        self.look.close();
    }

    pub fn take_drag(&mut self) -> Option<PathBuf> {
        self.drag.take()
    }

    pub fn take_uncover(&mut self) -> bool {
        std::mem::take(&mut self.uncover)
    }

    fn selected_index(&self) -> Option<usize> {
        let name = self.selected.as_ref()?;
        self.entries.iter().position(|(entry, _)| entry == name)
    }

    fn preview(&mut self, index: usize, ctx: &eframe::egui::Context) {
        if let Some((name, _)) = self.entries.get(index) {
            self.selected = Some(name.clone());
            let path = self.path.join(name);
            self.look.open(path, (index, self.entries.len()), ctx);
        }
    }

    /// Applies a key; returns text to type into the terminal for Enter.
    pub fn key(&mut self, key: FsKey, ctx: &eframe::egui::Context) -> Option<String> {
        match key {
            FsKey::Escape if self.look.is_open() => self.look.close(),
            FsKey::Escape => self.focused = false,
            FsKey::Step(step) => {
                // In the preview, Left and Right go through the folder one by one.
                let step = if self.look.is_open() { step.signum() } else { step };
                if let Some(index) = stepped(self.selected_index(), step, self.entries.len()) {
                    self.selected = Some(self.entries[index].0.clone());
                    if self.look.is_open() {
                        self.preview(index, ctx);
                    }
                }
            }
            FsKey::Look if self.look.is_open() => self.look.close(),
            FsKey::Look => {
                if let Some(index) = self.selected_index() {
                    self.preview(index, ctx);
                    self.uncover = true;
                }
            }
            FsKey::Open => {
                let index = self.selected_index()?;
                let (name, is_dir) = self.entries[index].clone();
                self.look.close();
                return Some(self.typed_for(&name, is_dir));
            }
        }
        None
    }

    /// What a click (folders) or Enter (files) types: `cd` into a folder, or the
    /// file's quoted path.
    fn typed_for(&self, name: &str, is_dir: bool) -> String {
        let quoted = shell_quote(&self.path.join(name).to_string_lossy());
        if is_dir { format!("cd {quoted}\r") } else { format!("{quoted} ") }
    }

    fn reload(&mut self, cwd: &Path) {
        // In another folder, what was picked in the last one is let go.
        if self.path != cwd { self.selected = None; self.focused = false; }
        self.path = cwd.to_path_buf();
        self.loaded = Some(Instant::now());
        self.sizes.clear();
        self.entries = std::fs::read_dir(cwd)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let is_dir = e.path().is_dir();
                        if !is_dir { if let Ok(meta) = e.metadata() { self.sizes.insert(name.clone(), meta.len()); } }
                        (name, is_dir)
                    })
                    .filter(|(name, _)| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        // Directories first, then case-insensitive by name.
        self.entries
            .sort_by_key(|(name, is_dir)| (!*is_dir, name.to_lowercase()));
    }

    /// Draws the browser: the tabs `tabs` draws, the folder as a path to click back
    /// up, then the folders and files. Returns text to type into the terminal when an
    /// entry or a folder of the path is clicked.
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, cwd: Option<&Path>, tabs: impl FnOnce(&mut Ui)) -> Option<String> {
        powerup::stage(ui.ctx()).apply(ui, Part::Bottom);
        if let Some(cwd) = cwd {
            let stale = self
                .loaded
                .is_none_or(|t| t.elapsed() > Duration::from_secs(2));
            if cwd != self.path || stale {
                self.reload(cwd);
            }
        }
        let mut typed = None;
        let folders = self.entries.iter().filter(|(_, is_dir)| *is_dir).count();
        let files = self.entries.len() - folders;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            tabs(ui);
            // The path, each folder of it a way back up; the current one plain.
            ui.spacing_mut().item_spacing.x = 2.0;
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
            let mut walk = PathBuf::new();
            let under_home = home.as_ref().filter(|home| self.path.starts_with(home));
            let rest = match under_home {
                Some(home) => { crumbs.push(("~".into(), home.clone())); walk = home.clone(); self.path.strip_prefix(home).map(Path::to_path_buf).unwrap_or_default() }
                None => { crumbs.push(("/".into(), PathBuf::from("/"))); walk.push("/"); self.path.strip_prefix("/").map(Path::to_path_buf).unwrap_or_default() }
            };
            for part in rest.components() {
                walk.push(part);
                crumbs.push((part.as_os_str().to_string_lossy().into_owned(), walk.clone()));
            }
            // A deep path keeps its last folders, after an ellipsis.
            let shown = crumbs.len().min(5);
            if crumbs.len() > shown { ui::text(ui, "… /", ui::mono(12.0), ui::LABEL); }
            let last = crumbs.len().saturating_sub(1);
            for (index, (name, path)) in crumbs.iter().enumerate().skip(crumbs.len() - shown) {
                if index > crumbs.len() - shown && !(index == 1 && crumbs[0].0 == "/") { ui::text(ui, "/", ui::mono(12.0), ui::LINE_STRONG); }
                if index == last {
                    ui::text(ui, &truncate(name, 32), ui::mono(12.0), ui::TEXT);
                } else {
                    let crumb = ui.add(eframe::egui::Button::new(eframe::egui::RichText::new(truncate(name, 24)).font(ui::mono(12.0)).color(ui::SECONDARY)).frame(false));
                    if crumb.on_hover_text(format!("Go to {}", path.display())).clicked() {
                        typed = Some(format!("cd {}\r", shell_quote(&path.to_string_lossy())));
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if ui::button(ui, theme, "OPEN IN FILES ↗", None, false).on_hover_text("Open this folder in Files").clicked() {
                    crate::files::request(ui.ctx(), if self.path.is_absolute() { self.path.clone() } else { crate::files::home() });
                }
                pill(ui, "FOLLOWS THE TERMINAL", theme.main).on_hover_text("This panel shows the folder the terminal is in");
                let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
                ui::meta(ui, &format!("{} · {}", plural(folders, "folder", "folders"), plural(files, "file", "files")), ui::LABEL);
            });
        });
        ui.add_space(6.0);

        let panel = ui.max_rect();
        if self.has_keyboard() {
            let keys = ui.input(|input| {
                input.events.iter().filter_map(|event| match event {
                    eframe::egui::Event::Key { key, pressed: true, modifiers, .. } if modifiers.is_none() => match key {
                        eframe::egui::Key::ArrowLeft => Some(FsKey::Step(-1)),
                        eframe::egui::Key::ArrowRight => Some(FsKey::Step(1)),
                        eframe::egui::Key::ArrowUp => Some(FsKey::Step(-(self.columns as isize))),
                        eframe::egui::Key::ArrowDown => Some(FsKey::Step(self.columns as isize)),
                        eframe::egui::Key::Space => Some(FsKey::Look),
                        eframe::egui::Key::Enter => Some(FsKey::Open),
                        eframe::egui::Key::Escape => Some(FsKey::Escape),
                        _ => None,
                    },
                    _ => None,
                }).collect::<Vec<_>>()
            });
            for key in keys {
                typed = self.key(key, ui.ctx()).or(typed);
            }
            // A click elsewhere gives the keyboard back to the terminal.
            let elsewhere = ui.input(|input| input.pointer.primary_pressed() && input.pointer.interact_pos().is_some_and(|at| !panel.contains(at)));
            if elsewhere && !self.look.is_open() {
                self.focused = false;
            }
        }
        // While a file has the keyboard, a line at the foot says what the keys do.
        let hint = self.focused.then(|| self.selected.clone()).flatten().filter(|name| self.entries.iter().any(|(entry, _)| entry == name));
        let gap = 6.0;
        let columns = ((ui.available_width() + gap) / (170.0 + gap)).floor().max(1.0);
        self.columns = columns as usize;
        let width = ((ui.available_width() + gap) / columns - gap).floor();
        let mut tiles: Vec<(String, bool, Option<String>)> = Vec::new();
        if self.path.parent().is_some() {
            tiles.push(("Up a folder".to_string(), true, Some("cd ..\r".to_string())));
        }
        for (name, is_dir) in &self.entries {
            tiles.push((name.clone(), *is_dir, None));
        }
        let grid_height = (ui.available_height() - if hint.is_some() { 22.0 } else { 0.0 }).max(20.0);
        ScrollArea::vertical().auto_shrink([false, false]).max_height(grid_height).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            ui.horizontal_wrapped(|ui| {
                for (name, is_dir, command) in &tiles {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click_and_drag());
                    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, name));
                    let painter = ui.painter_at(rect);
                    let picked = command.is_none() && self.selected.as_deref() == Some(name.as_str());
                    let (fill, edge) = if picked && self.focused {
                        (theme.main.gamma_multiply(0.14), theme.main)
                    } else if picked {
                        (ui::TRACK, ui::LINE_STRONG)
                    } else if response.hovered() {
                        (ui::TRACK, ui::LINE)
                    } else {
                        (ui::PANEL, ui::PANEL)
                    };
                    painter.rect_filled(rect, ui::CONTROL_RADIUS + 3.0, fill);
                    painter.rect_stroke(rect, ui::CONTROL_RADIUS + 3.0, Stroke::new(1.0, edge), StrokeKind::Inside);
                    let badge = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(22.0, 20.0));
                    let mut right = rect.right() - 10.0;
                    if command.is_some() {
                        painter.rect_filled(badge, 5.0, ui::TRACK);
                        let c = badge.center();
                        painter.line(vec![c + vec2(-4.0, 1.5), c + vec2(0.0, -3.0), c + vec2(4.0, 1.5)], Stroke::new(1.5, ui::SECONDARY));
                        painter.line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 5.0)], Stroke::new(1.5, ui::SECONDARY));
                    } else if *is_dir {
                        painter.rect_filled(badge, 5.0, theme.main.gamma_multiply(0.16));
                        folder_icon(&painter, badge.center() + vec2(0.5, 0.0), theme.main);
                    } else {
                        let (tag, color) = file_kind(name);
                        painter.rect_filled(badge, 5.0, color.gamma_multiply(0.16));
                        painter.text(badge.center(), Align2::CENTER_CENTER, tag, ui::mono(7.5), color);
                        if let Some(size) = self.sizes.get(name) {
                            let size = painter.layout_no_wrap(human_bytes(*size as f64), ui::mono(10.0), ui::LABEL);
                            right -= size.size().x;
                            painter.galley(pos2(right, rect.center().y - size.size().y / 2.0), size, ui::LABEL);
                            right -= 8.0;
                        }
                    }
                    let room = (right - (rect.left() + 38.0)).max(10.0);
                    let mut job = eframe::egui::text::LayoutJob::simple_singleline(name.clone(), ui::mono(12.0), if command.is_some() { ui::SECONDARY } else { ui::TEXT });
                    job.wrap = eframe::egui::text::TextWrapping::truncate_at_width(room);
                    let label = painter.layout_job(job);
                    painter.galley(pos2(rect.left() + 38.0, rect.center().y - label.size().y / 2.0), label, ui::TEXT);
                    // Dragged out of its tile, a file or folder goes where it is dropped.
                    if command.is_none() && response.dragged() && !self.dragging {
                        let outside = response.interact_pointer_pos().is_some_and(|at| !rect.contains(at));
                        if outside {
                            self.dragging = true;
                            self.drag = Some(self.path.join(name));
                        }
                    }
                    if response.drag_stopped() {
                        self.dragging = false;
                    }
                    if response.clicked() {
                        if command.is_none() && !is_dir {
                            // A file is picked, for Space to look at and Enter to type.
                            self.selected = Some(name.clone());
                            self.focused = true;
                        } else {
                            typed = command.clone().or_else(|| Some(self.typed_for(name, *is_dir)));
                        }
                    }
                    if response.double_clicked() && command.is_none() && !is_dir {
                        typed = Some(self.typed_for(name, false));
                    }
                }
            });
        });
        if let Some(name) = hint {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui::text(ui, &truncate(&name, 40), ui::mono(11.0), theme.main);
                for words in ["Space preview", "↵ type its path", "arrows move", "Esc back to the terminal"] {
                    ui::text(ui, words, ui::mono(11.0), ui::LABEL);
                }
            });
        }
        typed
    }
}

/// A file's kind as a short tag and a colour of its own, from its extension.
fn file_kind(name: &str) -> (String, eframe::egui::Color32) {
    use eframe::egui::Color32;
    let extension = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()).map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let colour = match extension.as_str() {
        "md" | "txt" | "pdf" | "doc" | "docx" | "odt" | "ods" | "xlsx" | "csv" | "rtf" => Color32::from_rgb(0x6F, 0xA8, 0xDC),
        "sh" | "py" | "rs" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "rb" | "toml" | "json" | "yaml" | "yml" | "html" | "css" => Color32::from_rgb(0x5F, 0xD0, 0x8A),
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "deb" | "rpm" | "iso" | "zst" => Color32::from_rgb(0xE6, 0xC3, 0x5C),
        "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "bmp" | "heic" => Color32::from_rgb(0xD9, 0x8B, 0xC8),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "mp4" | "mkv" | "webm" | "mov" | "avi" => Color32::from_rgb(0x8C, 0x9E, 0xFF),
        _ => ui::SECONDARY,
    };
    let tag = if extension.is_empty() { "—".to_string() } else { extension.chars().take(3).collect::<String>().to_uppercase() };
    (tag, colour)
}

fn folder_icon(painter: &eframe::egui::Painter, center: eframe::egui::Pos2, color: eframe::egui::Color32) {
    let (x, y) = (center.x - 8.0, center.y - 6.0);
    let points = [(0.0, 1.0), (5.0, 1.0), (6.5, 3.0), (15.0, 3.0), (15.0, 12.0), (0.0, 12.0), (0.0, 1.0)];
    let points: Vec<_> = points.iter().map(|(dx, dy)| pos2(x + dx, y + dy)).collect();
    painter.line(points, Stroke::new(1.2, color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_steps_stay_inside_the_folder() {
        assert_eq!(stepped(None, 1, 0), None);
        assert_eq!(stepped(None, 3, 5), Some(0));
        assert_eq!(stepped(Some(1), 3, 5), Some(4));
        assert_eq!(stepped(Some(4), 3, 5), Some(4));
        assert_eq!(stepped(Some(1), -3, 5), Some(0));
    }

    #[test]
    fn keys_pick_preview_type_and_let_go() {
        let ctx = eframe::egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let mut fs = FsView::new();
        fs.reload(dir.path());
        fs.selected = Some("a.txt".into());
        fs.focused = true;
        assert!(fs.has_keyboard());
        assert_eq!(fs.key(FsKey::Step(1), &ctx), None);
        assert_eq!(fs.selected.as_deref(), Some("b.txt"));
        fs.key(FsKey::Look, &ctx);
        assert!(fs.look.is_open() && fs.take_uncover());
        // In the preview a row step goes one file at a time.
        fs.key(FsKey::Step(3), &ctx);
        assert_eq!(fs.look.path(), Some(dir.path().join("c.txt").as_path()));
        fs.key(FsKey::Escape, &ctx);
        assert!(!fs.look.is_open() && fs.has_keyboard());
        let typed = fs.key(FsKey::Open, &ctx).unwrap();
        assert!(typed.ends_with("c.txt' ") || typed.ends_with("c.txt "), "{typed}");
        fs.key(FsKey::Escape, &ctx);
        assert!(!fs.has_keyboard());
    }

    #[test]
    fn moving_to_another_folder_lets_go_of_the_picked_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let mut fs = FsView::new();
        fs.reload(dir.path());
        assert_eq!(fs.sizes.get("a.txt"), Some(&1));
        fs.selected = Some("a.txt".into());
        fs.focused = true;
        fs.reload(dir.path());
        assert!(fs.has_keyboard(), "the same folder again keeps the pick");
        fs.reload(&dir.path().join("inner"));
        assert!(fs.selected.is_none() && !fs.has_keyboard());
    }

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn optional_sections_yield_to_what_must_follow() {
        // Both fit with room to spare.
        assert_eq!(sections_that_fit(1000.0, 300.0, &[120.0, 110.0]), 2);
        // Only the first fits once the reserve is kept.
        assert_eq!(sections_that_fit(500.0, 300.0, &[120.0, 110.0]), 1);
        // Neither: the reserve comes first.
        assert_eq!(sections_that_fit(350.0, 300.0, &[120.0, 110.0]), 0);
        // A section of no height takes nothing and stops nothing.
        assert_eq!(sections_that_fit(420.0, 300.0, &[0.0, 110.0]), 2);
        assert_eq!(sections_that_fit(100.0, 300.0, &[]), 0);
    }

    #[test]
    fn formats_sizes_and_names() {
        assert_eq!(human_bytes(512.0), "512 B");
        assert_eq!(human_bytes(1536.0), "1.5 KB");
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        assert_eq!(file_kind("README.md").0, "MD");
        assert_eq!(file_kind("archive.tar.gz").0, "GZ");
        assert_eq!(file_kind("Makefile").0, "—");
        assert_eq!(file_kind(".bashrc").0, "—", "a dotfile has no extension");
        assert_eq!(file_kind("photo.JPEG").0, "JPE");
    }
}

/// Shared vertical time treatment for collapsed panels and the application rail.
/// Respect the configured time format and repaint only on the next minute.
pub fn collapsed_clock(ui: &mut Ui, theme: &Theme, rect: Rect) {
    if rect.width() < 8.0 || rect.height() < 16.0 { return; }
    let now = chrono::Local::now();
    let (hours,minutes,half)=crate::region::vertical_clock(&now);
    let text=format!("{hours}:{minutes}{}",half.map(|period|format!(" {period}")).unwrap_or_default());
    let font_size=26.0_f32.min((rect.width()-4.0).max(8.0));
    let mut galley=ui.painter().layout_no_wrap(text.clone(),ui::display(font_size),theme.main);
    if galley.size().x>rect.height() {
        galley=ui.painter().layout_no_wrap(text,ui::display(font_size*rect.height()/galley.size().x),theme.main);
    }
    let size=galley.size();
    let origin=pos2(rect.center().x-size.y/2.0,rect.center().y+size.x/2.0);
    ui.painter().add(eframe::egui::epaint::TextShape::new(origin,galley,theme.main).with_angle(-std::f32::consts::FRAC_PI_2));
    use chrono::Timelike;
    ui.ctx().request_repaint_after(Duration::from_secs((60-now.second()) as u64));
}

/// A minimized side column stays clickable; its tooltip also names the global shortcut.
pub fn collapsed_rail(ui: &mut Ui, theme: &Theme, panel: Panel) -> bool {
    let rect = ui.max_rect();
    let (symbol, label, id) = match panel {
        Panel::Left => (Icon::PanelLeft, "Expand left panel", "desktop.panel_left"),
        Panel::Right => (Icon::PanelRight, "Expand right panel", "desktop.panel_right"),
        Panel::Bottom => return false,
    };
    let hint = crate::shortcuts::hint(ui.ctx(), label, id);
    let response = ui.interact(rect, ui.id().with(("expand-panel", id)), Sense::click());
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, ui.is_enabled(), hint.as_str()));
    let highlighted = response.hovered() || response.has_focus();
    if highlighted { ui.painter().rect_filled(rect, 0.0, theme.tint()); }
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 22.0), vec2(24.0, 24.0));
    ui::paint_icon(ui.painter(), icon_rect, symbol, if highlighted { theme.main } else { ui::SECONDARY }, false);
    // Small inward chevron makes the collapsed state visible without another label.
    let d = if panel == Panel::Left { 1.0 } else { -1.0 };
    let c = pos2(rect.center().x, rect.top() + 48.0);
    ui.painter().line(vec![c + vec2(-2.0*d,-3.0), c + vec2(2.0*d,0.0), c + vec2(-2.0*d,3.0)], Stroke::new(1.2, ui::LABEL));
    if panel == Panel::Left && rect.height() >= 146.0 {
        collapsed_clock(ui, theme, Rect::from_min_max(pos2(rect.left(),rect.top()+72.0),pos2(rect.right(),rect.bottom()-8.0)));
    }
    let clicked = response.clicked();
    response.on_hover_text(hint);
    clicked
}

#[cfg(test)]
mod rail_interaction_tests {
    use super::*;
    use eframe::egui::{self, Event, Modifiers, PointerButton};

    #[test]
    fn both_rails_have_independent_click_ownership() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        for target in 0..2 {
            let point = pos2(if target == 0 { 30.0 } else { 770.0 }, 200.0);
            for phase in 0..3 {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                    ..Default::default()
                };
                input.events.push(Event::PointerMoved(point));
                if phase > 0 {
                    input.events.push(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: phase == 1, modifiers: Modifiers::NONE });
                }
                let mut clicked = [false; 2];
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        for (i, panel) in [Panel::Left, Panel::Right].into_iter().enumerate() {
                            let x = if i == 0 { 8.0 } else { 748.0 };
                            // Shared parent and default child salt match the real frame regions.
                            let mut rail = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(x, 8.0), vec2(44.0, 564.0))));
                            clicked[i] = collapsed_rail(&mut rail, &theme, panel);
                        }
                    });
                });
                for i in 0..2 { assert_eq!(clicked[i], phase == 2 && target == i); }
            }
        }
    }

    #[test]
    fn either_minimized_column_can_be_reopened_by_clicking_its_rail() {
        for panel in [Panel::Left, Panel::Right] {
            let ctx = egui::Context::default();
            ui::install_fonts(&ctx,None,None);
            let theme = Theme::load("signal");
            let point = pos2(18.0, 200.0);
            for phase in 0..3 {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0,0.0), vec2(800.0,600.0))),
                    ..Default::default()
                };
                input.events.push(Event::PointerMoved(point));
                if phase == 1 || phase == 2 {
                    input.events.push(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: phase == 1, modifiers: Modifiers::NONE });
                }
                let mut clicked = false;
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut rail = ui.new_child(egui::UiBuilder::new().id_salt("rail").max_rect(Rect::from_min_size(pos2(0.0,0.0),vec2(des_common::COLLAPSED_SIDE_WIDTH, 572.0))));
                        clicked = collapsed_rail(&mut rail, &theme, panel);
                    });
                });
                assert_eq!(clicked, phase == 2, "{panel:?}, phase {phase}");
            }
        }
    }
}

#[cfg(test)]
mod left_footer_tests {
    use super::*;
    #[test]
    fn pet_is_painted_inside_its_reserved_footer_on_short_and_tall_panels() {
        for height in [260.0, 480.0, 960.0] {
            let ctx = eframe::egui::Context::default();
            ui::install_fonts(&ctx,None,None);
            let theme = Theme::load("signal");
            let mon = SysMon::new();
            let widgets = crate::widgets::Widgets::new(ctx.clone());
            let mut pet = crate::pet::Pet::new("crab");
            let area = Rect::from_min_size(pos2(20.0,20.0),vec2(192.0,height));
            let footer = Rect::from_min_max(pos2(area.left(),area.bottom()-pet.room()),area.max);
            let output = ctx.run(eframe::egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0,0.0),vec2(400.0,height+40.0))), ..Default::default() }, |ctx| {
                eframe::egui::CentralPanel::default().show(ctx,|ui| {
                    let mut column = ui.new_child(eframe::egui::UiBuilder::new().max_rect(area));
                    column.set_clip_rect(area);
                    left(&mut column,&theme,&mon,&mut pet,&widgets,None);
                });
            });
            assert!(output.shapes.iter().any(|shape| match &shape.shape {
                eframe::egui::Shape::Mesh(mesh) => mesh.vertices.len()==4 && mesh.vertices.iter().all(|vertex|footer.contains(vertex.pos)),
                _ => false,
            }), "pet missing from reserved footer at {height}px");
        }
    }
}
