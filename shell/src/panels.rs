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

/// Space between a column's sections.
const SECTION_GAP: f32 = 22.0;

/// What the left column keeps for the process list and the companion, so that the
/// optional instrument sections above them never crowd them out.
const LEFT_RESERVE: f32 = 3.0 * 27.0 + SECTION_GAP + 30.0 + 100.0;

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
fn tile_pair(ui: &mut Ui, mut left: impl FnMut(&mut Ui), mut right: impl FnMut(&mut Ui)) {
    let width = (ui.available_width() - 8.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| left(ui));
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| right(ui));
    });
}

/// The clock, in the display face: hours and minutes large, seconds in the accent.
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

pub fn left(ui: &mut Ui, theme: &Theme, mon: &SysMon, pet: &mut crate::pet::Pet, widgets: &crate::widgets::Widgets) {
    // At the start of a session the column comes on with the rest of the frame.
    let stage = powerup::stage(ui.ctx());
    stage.apply(ui, Part::Left);
    ui.spacing_mut().item_spacing.y = 7.0;
    let now = chrono::Local::now();
    // The clock is sized for a full-width column and shrinks with it.
    clock(ui, theme, 68.0 * (ui.available_width() / 272.0).min(1.0));
    ui.horizontal(|ui| {
        let date = now.format("%a %d %b %Y").to_string().to_uppercase();
        let date = ui::tracked(ui, &date, ui::display(13.0), ui::SECONDARY, 2.0);
        let up = mon.uptime_shown();
        let uptime = format!("UP {}D {}H", up / 86400, (up / 3600) % 24);
        let uptime = ui::galley(ui, &uptime, ui::display(13.0), ui::SECONDARY, 2.0);
        // On a narrow column the date alone has to do.
        let column = ui.max_rect();
        if date.rect.right() + 16.0 + uptime.size().x <= column.right() {
            let at = pos2(column.right() - uptime.size().x, date.rect.top());
            ui.painter().galley(at, uptime, ui::SECONDARY);
        }
    });

    ui.add_space(SECTION_GAP);
    // Following a host through ssh: its name, and whether its readings are the ones shown.
    let title = if mon.remote.is_some() { "REMOTE" } else { "SYSTEM" };
    ui::section(ui, title, |ui| {
        match &mon.remote {
            Some(remote) if remote.live => ui::meta(ui, &truncate(&mon.host, 18), theme.main),
            Some(remote) => {
                ui::meta(ui, remote.word, ui::CAUTION);
                ui::meta(ui, &truncate(&remote.name, 14), ui::LABEL);
            }
            None => ui::meta(ui, &truncate(&mon.host, 18), ui::LABEL),
        }
        // Failed units are the one thing here that asks for a look.
        if !mon.failed_units.is_empty() {
            let count = mon.failed_units.len();
            ui::meta(ui, &format!("{count} FAILED"), ui::CAUTION);
        }
    });
    tile_pair(
        ui,
        |ui| ui::value_tile(ui, theme, "OS", &truncate(&mon.os, 14)),
        |ui| ui::value_tile(ui, theme, "KERNEL", &truncate(&mon.kernel, 14)),
    );

    ui.add_space(SECTION_GAP);
    let load = mon.cpu_hist.back().copied().unwrap_or(0.0);
    ui::section(ui, "PROCESSOR", |ui| {
        ui::meta(ui, &format!("{load:.0}%"), ui::TEXT);
        if let Some(average) = mon.load {
            ui::meta(ui, &format!("LOAD {:.2}", average[0]), ui::LABEL);
        }
    });
    ui::sparkline(ui, &stage.swept_values(&mon.cpu_hist), 100.0, vec2(ui.available_width(), 56.0), theme.main, true);
    ui.add_space(3.0);
    // One bar per core, however many there are; the pointer reads any one of them.
    ui::bar_grid(ui, theme, &stage.swept_slice(&mon.cores), 100.0, 28.0, |i, usage| format!("Core {}: {usage:.0}%", i + 1));
    if let Some(gpu) = &mon.gpu {
        ui.add_space(3.0);
        ui::meter(ui, theme, "GPU", 30.0, stage.swept(gpu.busy as f32 / 100.0), &format!("{}", gpu.busy));
    }

    ui.add_space(SECTION_GAP);
    ui::section(ui, "MEMORY", |ui| {
        let total = format!("/ {:.1} GiB", mon.mem_total as f64 / GIB);
        ui::meta(ui, &total, ui::LABEL);
        ui::meta(ui, &format!("{:.1}", mon.mem_used as f64 / GIB), ui::TEXT);
    });
    ui::sparkline(ui, &stage.swept_values(&mon.mem_hist), 100.0, vec2(ui.available_width(), 30.0), theme.main, true);
    ui.add_space(3.0);
    ui::bar(ui, theme, 10.0, stage.swept(mon.mem_used as f32 / mon.mem_total.max(1) as f32), 0.0);
    ui.horizontal(|ui| {
        ui::text(ui, "USED", ui::mono(11.0), ui::SECONDARY);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let swap = format!("SWAP {:.1} GiB", mon.swap_used as f64 / GIB);
            ui::text(ui, &swap, ui::mono(11.0), ui::SECONDARY);
        });
    });

    // The instruments that need hardware to speak, and room to be seen.
    let temps = mon.temps.len().min(4);
    let fans = mon.fans.len().min(2);
    let thermal_height = if temps > 0 { SECTION_GAP + 24.0 + (temps + fans) as f32 * 23.0 } else { 0.0 };
    let filesystems = mon.filesystems.len().min(3);
    let storage_height = SECTION_GAP + 24.0 + 36.0 + 7.0 + filesystems as f32 * 23.0;
    // The user's own widgets come before the optional instruments, so they keep their room too.
    let widget_height = widgets.on(des_common::Panel::Left).count() as f32 * 60.0;
    let fitting = sections_that_fit(ui.available_height(), LEFT_RESERVE + widget_height, &[thermal_height, storage_height]);
    if temps > 0 && fitting >= 1 {
        ui.add_space(SECTION_GAP);
        let hottest = mon.temps[0].degrees;
        ui::section(ui, "THERMAL", |ui| {
            let color = if hottest >= 100.0 * ui::CAUTION_ABOVE { ui::CAUTION } else { ui::TEXT };
            ui::meta(ui, &format!("{hottest:.0}°C"), color);
        });
        for reading in mon.temps.iter().take(temps) {
            ui::meter(ui, theme, &reading.label, 44.0, reading.degrees / 100.0, &format!("{:.0}°", reading.degrees));
        }
        for fan in mon.fans.iter().take(fans) {
            ui.horizontal(|ui| {
                ui::text(ui, &truncate(&fan.label, 12), ui::mono(12.0), ui::LABEL);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let reading = if fan.rpm == 0 { "STILL".to_string() } else { format!("{} RPM", fan.rpm) };
                    ui::text(ui, &reading, ui::mono(12.0), ui::TEXT);
                });
            });
        }
    }
    if fitting >= 2 {
        ui.add_space(SECTION_GAP);
        let read = mon.disk_read_hist.back().copied().unwrap_or(0.0);
        let written = mon.disk_write_hist.back().copied().unwrap_or(0.0);
        ui::section(ui, "STORAGE", |ui| {
            ui::meta(ui, &format!("W {}/s", human_bytes(written as f64)), ui::SECONDARY);
            ui::meta(ui, &format!("R {}/s", human_bytes(read as f64)), ui::TEXT);
        });
        // Reads above the line, writes below, on one scale.
        let peak = mon
            .disk_read_hist
            .iter()
            .chain(mon.disk_write_hist.iter())
            .fold(1024.0_f32 * 1024.0, |a, b| a.max(*b));
        ui::waterfall(ui, &stage.swept_values(&mon.disk_read_hist), &stage.swept_values(&mon.disk_write_hist), peak, vec2(ui.available_width(), 36.0), theme.main, ui::SECONDARY);
        for fs in mon.filesystems.iter().take(filesystems) {
            ui::meter(ui, theme, &truncate(&fs.mount, 9), 60.0, stage.swept(fs.used), &format!("{:.0}%", fs.used * 100.0));
        }
    }

    crate::widgets::show(ui, theme, widgets, des_common::Panel::Left);

    ui.add_space(SECTION_GAP);
    ui::section(ui, "PROCESSES", |ui| ui::meta(ui, "CPU · MEM", ui::LABEL));
    ui.spacing_mut().item_spacing.y = 0.0;
    // The list stops short of the companion's strip, so the animal always has its room.
    let floor = ui.max_rect().bottom() - pet.room();
    for process in &mon.top {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 27.0), Sense::hover());
        if rect.bottom() > floor {
            break;
        }
        let painter = ui.painter();
        let busy = process.cpu > ui::CAUTION_ABOVE * 100.0 / 2.0;
        painter.text(rect.left_center(), Align2::LEFT_CENTER, truncate(&process.name, 16), ui::mono(12.0), ui::TEXT);
        painter.text(
            rect.right_center() - vec2(70.0, 0.0),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", process.cpu),
            ui::mono(12.0),
            if busy { ui::CAUTION } else { ui::TEXT },
        );
        painter.text(rect.right_center(), Align2::RIGHT_CENTER, human_bytes(process.mem as f64), ui::mono(12.0), ui::SECONDARY);
        painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, ui::TRACK));
    }
    pet.show(ui);
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
        ui::tile(ui, theme, active, |ui| {
            let label = if screen == 0 {
                "1 · MAIN".to_string()
            } else {
                format!("{}", screen + 1)
            };
            ui::tracked(ui, &label, ui::display(10.0), if active { theme.main } else { ui::LABEL }, 2.0);
            ui::text(ui, &truncate(&describe(screen), 15), ui::mono(12.0), ui::TEXT);
        });
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

/// The right column. `rotation` turns the globe; `desktop` is absent when the shell
/// runs as an ordinary window.
pub fn right(
    ui: &mut Ui,
    theme: &Theme,
    mon: &SysMon,
    desktop: Option<&DesktopState>,
    rotation: f32,
    geo: &Snapshot,
    widgets: &crate::widgets::Widgets,
    playing: impl FnOnce(&mut Ui),
) {
    let stage = powerup::stage(ui.ctx());
    stage.apply(ui, Part::Right);
    ui.spacing_mut().item_spacing.y = 7.0;
    let online = !mon.ip.is_empty();
    ui::section(ui, "NETWORK", |ui| {
        let (label, color) = if online {
            ("ONLINE", theme.main)
        } else {
            ("OFFLINE", ui::CAUTION)
        };
        ui::meta(ui, label, color);
        let (dot, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
        ui.painter().rect_filled(dot, 0.0, color);
    });
    ui.horizontal(|ui| {
        let iface = if mon.iface.is_empty() { "no interface" } else { &mon.iface };
        ui::text(ui, iface, ui::mono(12.0), ui::LABEL);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui::text(ui, if online { &mon.ip } else { "—" }, ui::mono(12.0), ui::TEXT);
        });
    });
    // Both graphs share a scale so their heights are comparable.
    let peak = mon
        .rx_hist
        .iter()
        .chain(mon.tx_hist.iter())
        .fold(1024.0_f32, |a, b| a.max(*b));
    let traffic = |ui: &mut Ui, symbol: Icon, total: u64, hist: &std::collections::VecDeque<f32>, color: eframe::egui::Color32| {
        ui::tile(ui, theme, false, |ui| {
            ui.horizontal(|ui| {
                ui::icon(ui, symbol, 14.0, ui::LABEL);
                // The running total, where the tile is wide enough for both.
                if ui.available_width() > 78.0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui::tracked(ui, &human_bytes(total as f64), ui::display(10.0), ui::LABEL, 1.0);
                    });
                }
            });
            let rate = hist.back().copied().unwrap_or(0.0);
            ui::text(ui, &format!("{}/s", human_bytes(rate as f64)), ui::mono(14.0), color);
        });
    };
    let width = (ui.available_width() - 8.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            traffic(ui, Icon::Download, mon.rx_total, &mon.rx_hist, theme.main)
        });
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            traffic(ui, Icon::Upload, mon.tx_total, &mon.tx_hist, ui::SECONDARY)
        });
    });
    // What comes in rises from the line, what goes out hangs from it.
    ui::waterfall(ui, &stage.swept_values(&mon.rx_hist), &stage.swept_values(&mon.tx_hist), peak, vec2(ui.available_width(), 48.0), theme.main, ui::SECONDARY);

    ui.add_space(SECTION_GAP);
    ui::section(ui, "WORLD VIEW", |ui| {
        if !geo.links.is_empty() {
            let plural = if geo.links.len() == 1 { "" } else { "S" };
            ui::meta(ui, &format!("{} LINK{plural}", geo.links.len()), ui::LABEL);
        }
    });
    crate::widgets::show(ui, theme, widgets, des_common::Panel::Right);
    globe::show(ui, theme, (ui.available_width() * 0.82).min(224.0), rotation, geo);
    if let Some(home) = &geo.home {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (dot, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
            ui.painter().rect_filled(dot, 0.0, ui::TEXT);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Says how this position is known, since one of the ways is a guess.
                ui::meta(ui, if geo.home_is_set { "SET BY YOU" } else { "BY IP" }, ui::LABEL);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui::text(ui, &home.label.to_uppercase(), ui::mono(11.0), ui::SECONDARY);
                });
            });
        });
    }

    playing(ui);

    if let Some(desktop) = desktop {
        ui.add_space(SECTION_GAP);
        ui::section(ui, "DISPLAYS", |ui| {
            ui::meta(ui, &format!("{} CONNECTED", desktop.screens), ui::LABEL)
        });
        display_tiles(ui, theme, desktop);
    }

    let readings = [
        ("VOLUME", mon.volume()),
        ("BRIGHTNESS", mon.brightness),
        ("BATTERY", mon.battery),
    ];
    if readings.iter().any(|(_, value)| value.is_some()) {
        ui.add_space(SECTION_GAP);
        // While another machine fills the panels, these stay this one's; say so quietly.
        if mon.remote.as_ref().is_some_and(|remote| remote.live) {
            ui::meta(ui, "LOCAL", ui::LABEL);
        }
        ui::section(ui, "CONTROLS", |_| {});
        for (label, value) in readings {
            if let Some(value) = value {
                // These are levels, not loads: a full battery is not a warning.
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                let hint = match (label, mon.battery_health) {
                    ("BATTERY", Some(health)) => format!("{label}: {value}% · holds {health}% of what it did when new"),
                    _ => format!("{label}: {value}%"),
                };
                response.on_hover_text(hint);
                let painter = ui.painter();
                let symbol = match label {
                    "VOLUME" if value == 0 => Icon::Muted,
                    "VOLUME" => Icon::Volume,
                    "BRIGHTNESS" => Icon::Brightness,
                    _ if mon.power.is_some_and(|power| power.plugged) => Icon::Charging,
                    _ => Icon::Battery,
                };
                ui::paint_icon(painter, Rect::from_center_size(rect.left_center() + vec2(8.0, 0.0), vec2(16.0,16.0)), symbol, ui::LABEL, false);
                painter.text(rect.right_center(), Align2::RIGHT_CENTER, value.to_string(), ui::mono(12.0), ui::TEXT);
                let track = Rect::from_min_max(
                    pos2(rect.left() + 30.0, rect.center().y - 3.0),
                    pos2(rect.right() - 40.0, rect.center().y + 3.0),
                );
                painter.rect_filled(track, 0.0, ui::TRACK);
                let mut filled = track;
                filled.set_width(track.width() * (value as f32 / 100.0).min(1.0));
                painter.rect_filled(filled, 0.0, theme.main);
            }
        }
    }

    ui.add_space(SECTION_GAP);
    ui::section(ui, "KEYS", |ui| ui::meta(ui, "HOLD SUPER", ui::LABEL));
    for (action, key) in [
        ("Launcher", "SPACE"),
        ("Settings", ","),
        ("Music", "M"),
        ("Notifications", "N"),
        ("Power", "ESC"),
        ("Lock", "L"),
        ("Next application", "TAB"),
        ("Workspace", "1 – 4"),
        ("Send to workspace", "SHIFT 1 – 4"),
        ("Split the workspace", "S"),
        ("Full screen", "F"),
        ("Other display", "O"),
        ("Close application", "Q"),
        ("Panels", "[  ]  \\"),
        ("Restart shell", "SHIFT R"),
    ] {
        // Only as many as fit: the list is a reminder, not a reference.
        if ui.available_height() < 26.0 {
            break;
        }
        ui::key_row(ui, action, key);
    }
}

/// The strip along the bottom edge: what has the keyboard, the panel switches, and
/// the readings worth seeing at all times. Returns a panel whose switch was clicked,
/// and whether the settings were asked for. `overlay` names what is shown in place of
/// the terminal, if anything.
pub fn status(
    ui: &mut Ui,
    theme: &Theme,
    mon: &SysMon,
    desktop: Option<&DesktopState>,
    panels: Panels,
    overlay: Option<&str>,
    notices: usize,
    recording: Option<std::time::Duration>,
    tray: Option<&crate::tray::Tray>,
) -> (Option<Panel>, bool, bool, bool, bool, bool) {
    let mut toggled = None;
    let stage = powerup::stage(ui.ctx());
    if stage.alpha(Part::Status) < 1.0 {
        ui.set_opacity(stage.alpha(Part::Status));
    }
    ui.spacing_mut().item_spacing.x = 20.0;
    ui::tracked(ui, "DEs-UI", ui::display_bold(11.0), theme.main, 3.0);
    // The way into the settings, lit while they are open.
    let way_in = |ui: &mut Ui, name: &str, icon: Icon, hint: &str| {
        ui::icon_button(ui, theme, icon, 24.0, overlay == Some(name)).on_hover_text(hint).clicked()
    };
    let settings_clicked = way_in(ui, "SETTINGS", Icon::Settings, "Settings (Super+,)");
    let music_clicked = way_in(ui, "MUSIC", Icon::Music, "Music (Super+M)");
    let power_clicked = way_in(ui, "POWER", Icon::Power, "Sleep, restart, shut down (Super+Escape, or the power key)");
    let notices_clicked = way_in(ui, "NOTIFICATIONS", Icon::Notifications, "Notifications (Super+N)");
    if notices > 0 {
        // Keep the unread count next to the bell.
        ui.add_space(-14.0);
        ui::text(ui, &notices.to_string(), ui::mono(11.0), theme.main);
    }

    let holder = match (overlay, desktop.and_then(|d| d.focus.and_then(|index| d.apps.get(index)))) {
        (Some(overlay), _) => overlay.to_string(),
        (None, Some(app)) => truncate(&app.title, 28).to_uppercase(),
        (None, None) => "TERMINAL".to_string(),
    };
    ui::text(ui, &format!("{holder} HAS THE KEYBOARD"), ui::mono(11.0), ui::SECONDARY);

    let mut stop_clicked = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(running) = recording {
            // The one thing here that is happening to the user, so it is the one in colour.
            let seconds = running.as_secs();
            let label = ui::galley(ui, &format!("REC {:02}:{:02}  STOP", seconds / 60, seconds % 60), ui::mono(11.0), ui::CAUTION, 0.0);
            let (rect, response) = ui.allocate_exact_size(label.size() + vec2(14.0, 8.0), Sense::click());
            ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, ui::CAUTION), StrokeKind::Inside);
            ui.painter().galley(rect.center() - label.size() / 2.0, label, ui::CAUTION);
            stop_clicked = response.on_hover_text("End the recording").clicked();
        }
        if let Some(battery) = mon.power {
            // Low and running down is the one reading here that asks for something.
            let low = !battery.plugged && battery.percent <= 15;
            let left = battery.minutes.map_or(String::new(), |minutes| format!("  {}", crate::power::duration(minutes)));
            let label = match (battery.plugged, battery.minutes) {
                (true, Some(_)) => format!("CHARGING {}%{left}", battery.percent),
                (true, None) => format!("MAINS {}%", battery.percent),
                (false, _) => format!("BAT {}%{left}", battery.percent),
            };
            let value = format!("{}%{left}", battery.percent);
            ui::icon_value(ui, if battery.plugged { Icon::Charging } else { Icon::Battery }, &value,
                if low { ui::CAUTION } else { ui::TEXT }, &label);
        }
        if let Some(volume) = mon.volume() {
            ui::icon_value(ui, if volume == 0 { Icon::Muted } else { Icon::Volume }, &format!("{volume}%"), ui::SECONDARY, "Volume");
        }
        if let Some(wifi) = mon.wifi {
            ui::icon_value(ui, Icon::Wifi, &format!("{wifi}%"), ui::SECONDARY, "Wi-Fi signal strength");
        }
        if let Some(desktop) = desktop {
            if desktop.screens > 1 {
                ui::icon_value(ui, Icon::Display, &desktop.screens.to_string(), ui::SECONDARY, "Connected displays");
            }
            let count = desktop.apps.len();
            ui::icon_value(ui, Icon::Apps, &count.to_string(), ui::SECONDARY, "Open applications");
        }
        ui.spacing_mut().item_spacing.x = 4.0;
        for (icon, panel, open) in [
            (Icon::PanelRight, Panel::Right, panels.right),
            (Icon::PanelBottom, Panel::Bottom, panels.bottom),
            (Icon::PanelLeft, Panel::Left, panels.left),
        ] {
            let name = match panel {
                Panel::Left => "left",
                Panel::Right => "right",
                Panel::Bottom => "bottom",
            };
            let action = if open { "Collapse" } else { "Open" };
            let button = ui::icon_button(ui, theme, icon, 22.0, open)
                .on_hover_text(format!("{action} the {name} panel"));
            if button.clicked() {
                toggled = Some(panel);
            }
        }
        // Applications' own icons, left of the readings.
        if let Some(tray) = tray {
            ui.spacing_mut().item_spacing.x = 20.0;
            crate::tray::show(ui, theme, tray);
        }
    });
    (toggled, settings_clicked, music_clicked, notices_clicked, power_clicked, stop_clicked)
}

/// What a further display shows while no application is on it.
pub fn backdrop(ui: &mut Ui, theme: &Theme, mon: &SysMon, number: usize) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, ui::GROUND);
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
        painter.rect_filled(tile, 0.0, ui::PANEL);
        painter.rect_stroke(tile, 0.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let name = ui::galley(ui, label, ui::display_bold(12.0), ui::SECONDARY, 3.0);
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
        self.path = cwd.to_path_buf();
        self.loaded = Some(Instant::now());
        self.entries = std::fs::read_dir(cwd)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        (name, e.path().is_dir())
                    })
                    .filter(|(name, _)| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        // Directories first, then case-insensitive by name.
        self.entries
            .sort_by_key(|(name, is_dir)| (!*is_dir, name.to_lowercase()));
    }

    /// Draws the browser: a path, then a grid of tiles. Returns text to type into the
    /// terminal when an entry is clicked.
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, cwd: Option<&Path>) -> Option<String> {
        powerup::stage(ui.ctx()).apply(ui, Part::Bottom);
        if let Some(cwd) = cwd {
            let stale = self
                .loaded
                .is_none_or(|t| t.elapsed() > Duration::from_secs(2));
            if cwd != self.path || stale {
                self.reload(cwd);
            }
        }
        let count = self.entries.len();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui::tracked(ui, "FILES", ui::display_bold(12.0), ui::SECONDARY, 3.0);
            // The path, with everything but the current folder played down.
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let shown = match home.as_ref().and_then(|home| self.path.strip_prefix(home).ok()) {
                Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
                Some(rest) => format!("~/{}", rest.display()),
                None => self.path.display().to_string(),
            };
            let (parent, name) = match shown.rsplit_once('/') {
                Some((parent, name)) if !name.is_empty() => (format!("{parent}/"), name.to_string()),
                _ => (String::new(), shown.clone()),
            };
            ui.spacing_mut().item_spacing.x = 0.0;
            ui::text(ui, &truncate_start(&parent, 60), ui::mono(12.0), ui::LABEL);
            ui::text(ui, &name, ui::mono(12.0), ui::TEXT);
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let plural = if count == 1 { "" } else { "S" };
                ui::meta(ui, &format!("{count} ITEM{plural} · FOLLOWS THE TERMINAL"), ui::LABEL);
                let rest = ui.available_rect_before_wrap();
                if rest.width() > 4.0 {
                    ui.painter().hline(rest.x_range(), rest.center().y, Stroke::new(1.0, ui::LINE));
                }
            });
        });
        ui.add_space(8.0);

        let mut typed = None;
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
        let gap = 8.0;
        let columns = ((ui.available_width() + gap) / (200.0 + gap)).floor().max(1.0);
        self.columns = columns as usize;
        let width = ((ui.available_width() + gap) / columns - gap).floor();
        let mut tiles: Vec<(String, bool, Option<String>)> = Vec::new();
        if self.path.parent().is_some() {
            tiles.push(("..".to_string(), true, Some("cd ..\r".to_string())));
        }
        for (name, is_dir) in &self.entries {
            tiles.push((name.clone(), *is_dir, None));
        }
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            ui.horizontal_wrapped(|ui| {
                for (name, is_dir, command) in &tiles {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click_and_drag());
                    let painter = ui.painter_at(rect);
                    let picked = command.is_none() && self.selected.as_deref() == Some(name.as_str());
                    let line = if picked && self.focused {
                        theme.main
                    } else if response.hovered() || picked {
                        ui::LINE_STRONG
                    } else {
                        ui::LINE
                    };
                    painter.rect_filled(rect, 0.0, ui::GROUND);
                    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), StrokeKind::Inside);
                    let icon = pos2(rect.left() + 20.0, rect.center().y);
                    if *is_dir {
                        let color = if name == ".." { ui::SECONDARY } else { theme.main };
                        folder_icon(&painter, icon, color);
                    } else {
                        file_icon(&painter, icon, ui::LABEL);
                    }
                    painter.text(
                        pos2(rect.left() + 38.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        truncate(name, ((width - 48.0) / 7.4) as usize),
                        ui::mono(12.0),
                        if *is_dir && name != ".." { ui::TEXT } else { ui::SECONDARY },
                    );
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
        typed
    }
}

/// Keeps the end of `s`, which for a path is the part that matters.
fn truncate_start(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(count - max.saturating_sub(1)).collect();
        format!("…{tail}")
    }
}

fn folder_icon(painter: &eframe::egui::Painter, center: eframe::egui::Pos2, color: eframe::egui::Color32) {
    let (x, y) = (center.x - 8.0, center.y - 6.0);
    let points = [(0.0, 1.0), (5.0, 1.0), (6.5, 3.0), (15.0, 3.0), (15.0, 12.0), (0.0, 12.0), (0.0, 1.0)];
    let points: Vec<_> = points.iter().map(|(dx, dy)| pos2(x + dx, y + dy)).collect();
    painter.line(points, Stroke::new(1.2, color));
}

fn file_icon(painter: &eframe::egui::Painter, center: eframe::egui::Pos2, color: eframe::egui::Color32) {
    let (x, y) = (center.x - 6.0, center.y - 7.0);
    let points = [(0.0, 0.0), (8.0, 0.0), (12.0, 4.0), (12.0, 14.0), (0.0, 14.0), (0.0, 0.0)];
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
        assert_eq!(truncate_start("abcdefghij", 5), "…ghij");
        assert_eq!(truncate_start("short", 10), "short");
    }
}

/// A minimized side column stays clickable; its tooltip also names the global shortcut.
pub fn collapsed_rail(ui: &mut Ui, theme: &Theme, panel: Panel) -> bool {
    let rect = ui.max_rect();
    let (symbol, hint) = match panel {
        Panel::Left => (Icon::PanelLeft, "Expand left panel (Super+[)"),
        Panel::Right => (Icon::PanelRight, "Expand right panel (Super+])"),
        Panel::Bottom => return false,
    };
    let response = ui.interact(rect, ui.id().with("expand-panel"), Sense::click());
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, ui.is_enabled(), hint));
    let highlighted = response.hovered() || response.has_focus();
    if highlighted { ui.painter().rect_filled(rect, 0.0, theme.tint()); }
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 22.0), vec2(24.0, 24.0));
    ui::paint_icon(ui.painter(), icon_rect, symbol, if highlighted { theme.main } else { ui::SECONDARY }, false);
    // Small inward chevron makes the collapsed state visible without another label.
    let d = if panel == Panel::Left { 1.0 } else { -1.0 };
    let c = pos2(rect.center().x, rect.top() + 48.0);
    ui.painter().line(vec![c + vec2(-2.0*d,-3.0), c + vec2(2.0*d,0.0), c + vec2(-2.0*d,3.0)], Stroke::new(1.2, ui::LABEL));
    let clicked = response.clicked();
    response.on_hover_text(hint);
    clicked
}

#[cfg(test)]
mod rail_interaction_tests {
    use super::*;
    use eframe::egui::{self, Event, Modifiers, PointerButton};

    #[test]
    fn either_minimized_column_can_be_reopened_by_clicking_its_rail() {
        for panel in [Panel::Left, Panel::Right] {
            let ctx = egui::Context::default();
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
