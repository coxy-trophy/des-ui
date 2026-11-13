//! Quick settings: the things changed many times a day, in one pull-down above the
//! right end of the status strip, without opening the settings. Wi-Fi, Bluetooth,
//! sound, brightness, the night light, Do not disturb and the on-screen keyboard,
//! with the battery and the ways to lock, the settings and the power menu.
//!
//! It is drawn by the shell and lifted above applications with the compositor's
//! overlay, as the scratchpad is; while both are open the quick settings take the
//! overlay, since they are the one in use.

use crate::radio::Radio;
use crate::settings::Change;
use crate::sound::Sound;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align, Key, Layout, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::time::{Duration, Instant};

const WIDTH: f32 = 420.0;
const MAX_HEIGHT: f32 = 820.0;
const INITIAL_HEIGHT: f32 = 620.0;
const PADDING: f32 = 18.0;
/// Room kept between the panel and the screen's right edge and the status strip.
const MARGIN: f32 = 12.0;
/// How long a level just set is shown before the system's reading takes over again.
const HOLD: Duration = Duration::from_millis(1500);

/// What the quick settings ask the shell to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Change(Change),
    /// Silence the default output, or let it speak again.
    Mute,
    Keyboard,
    DoNotDisturb(bool),
    Settings,
    /// The settings, at the page of this name.
    SettingsAt(&'static str),
    Power,
    Lock,
}

/// How things stand, as the shell knows them, for the panel to show.
#[derive(Debug, Clone, Default)]
pub struct Readings {
    pub volume: Option<u8>,
    pub brightness: Option<u8>,
    pub night_light: Option<(u8, u8, u32)>,
    pub do_not_disturb: bool,
    pub keyboard: bool,
    /// The battery itself, for the right panel's line, and how much of its first
    /// capacity it still holds.
    pub power: Option<crate::sysmon::Battery>,
    pub battery_health: Option<u8>,
}

#[derive(Default)]
pub struct Quick {
    pub open: bool,
    radio: Option<Radio>,
    sound: Option<Sound>,
    /// Levels being dragged: the value and when, held until the system reports it.
    volume: Option<(u8, Instant)>,
    brightness: Option<(u8, Instant)>,
    content_height: Option<f32>,
    drawn_area: Option<Rect>,
    view: View,
}

/// Where the panel goes: at the right of the screen, its foot just above the status strip.
fn area(screen: Rect, status_top: f32, content_height: Option<f32>) -> Rect {
    let width = WIDTH.min(screen.width() - 2.0 * MARGIN).max(0.0);
    let bottom = status_top - MARGIN;
    let desired = content_height.map_or(INITIAL_HEIGHT, |height| height + 2.0 * PADDING);
    let height = desired.min(MAX_HEIGHT).min(bottom - screen.top() - MARGIN).max(0.0);
    Rect::from_min_max(pos2(screen.right() - MARGIN - width, bottom - height), pos2(screen.right() - MARGIN, bottom))
}

/// The level to show: the one just set while it is fresh, else the system's reading.
fn shown(held: Option<(u8, Instant)>, reading: Option<u8>, now: Instant) -> Option<u8> {
    match held {
        Some((level, at)) if now.duration_since(at) < HOLD => Some(level),
        _ => reading,
    }
}

impl Quick {
    pub fn drawn_area(&self) -> Option<Rect> { self.open.then_some(self.drawn_area).flatten() }

    fn area(&self, screen: Rect, status_top: f32) -> Rect {
        area(screen, status_top, self.content_height)
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draws the panel when open. A click outside it, other than on the status strip
    /// (whose button toggles it), or Escape closes it.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, screen: Rect, status_top: f32, readings: &Readings) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let rect = self.area(screen, status_top);
        self.drawn_area = Some(rect);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return actions;
        }
        let clicked_outside = ctx.input(|input| {
            input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|at| !rect.contains(at) && at.y < status_top)
        });
        if clicked_outside {
            self.open = false;
            return actions;
        }
        let mut measured = None;
        egui::Area::new(egui::Id::new("quick-settings"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                crate::material::paint(ui.ctx(), ui.painter(), rect, ui::SURFACE_RADIUS, crate::material::Role::Floating);
                ui.painter().rect_stroke(rect, ui::SURFACE_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
                let inner = rect.shrink(PADDING);
                let mut content = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
                let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(inner.height()).show(&mut content, |ui| {
                    self.contents(ui, theme, readings, &mut actions);
                });
                measured = Some(scroll.content_size.y);
            });
        if let Some(height) = measured {
            if self.content_height.is_none_or(|old| (old - height).abs() > 1.0) {
                self.content_height = Some(height);
                ctx.request_repaint();
            }
        }
        actions
    }

    fn contents(&mut self, ui: &mut Ui, theme: &Theme, readings: &Readings, actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 10.0;
        let radio = self.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
        // Wi-Fi and Bluetooth lists take the whole pop-up, with their own way back.
        if matches!(self.view, View::Wifi | View::Bluetooth) {
            let (exit, page) = if self.view == View::Wifi { (radio.quick_wifi(ui, theme), "wifi") } else { (radio.quick_bluetooth(ui, theme), "bluetooth") };
            match exit {
                crate::radio::QuickExit::Stay => {}
                crate::radio::QuickExit::Back => self.view = View::Levels,
                crate::radio::QuickExit::Settings => { self.view = View::Levels; actions.push(Action::SettingsAt(page)); }
            }
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            return;
        }
        // The head: what this is, and the battery in a word or two.
        ui.horizontal(|ui| {
            ui::tracked(ui, "QUICK SETTINGS", ui::display_bold(12.0), ui::SECONDARY, 1.4);
            if let Some(power) = readings.power {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let low = !power.plugged && power.percent <= 15;
                    let left = match (power.plugged, power.minutes) {
                        (true, Some(m)) => format!("{}% · full in {}", power.percent, crate::power::duration(m).to_lowercase()),
                        (true, None) => format!("{}% · on mains", power.percent),
                        (false, Some(m)) => format!("{}% · {} left", power.percent, crate::power::duration(m).to_lowercase()),
                        (false, None) => format!("{}%", power.percent),
                    };
                    crate::panels::pill(ui, &left, if low { ui::CAUTION } else { ui::SECONDARY }).on_hover_text(crate::power::battery_line(power));
                });
            }
        });

        let wifi = radio.wifi_glance();
        let bluetooth = radio.bluetooth_glance();
        let now = Instant::now();
        let volume = shown(self.volume, readings.volume, now);
        let brightness = shown(self.brightness, readings.brightness, now);
        let state = |glance: &crate::radio::Glance| match glance.powered {
            None => "unavailable".to_string(),
            Some(on) => glance.working.clone().or_else(|| glance.detail.clone().filter(|_| on)).unwrap_or_else(|| (if on { "on" } else { "off" }).to_string()),
        };
        let night = readings.night_light;
        let night_detail = night.map_or("off".to_string(), |(from, to, _)| format!("{from:02}:00 – {to:02}:00"));
        let sound_detail = match volume { None => "—".to_string(), Some(0) => "muted".to_string(), Some(level) => format!("{level}%") };
        // Six switches, as the right panel's; Wi-Fi, Bluetooth and sound open a list
        // from the arrow at their edge.
        let gap = 8.0;
        let width = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
        let size = vec2(width, 66.0);
        let tiles: [(ui::Icon, &str, String, Option<bool>, bool); 6] = [
            (ui::Icon::Wifi, "Wi-Fi", state(&wifi), wifi.powered, true),
            (ui::Icon::Bluetooth, "Bluetooth", state(&bluetooth), bluetooth.powered, true),
            (ui::Icon::Moon, "Quiet", (if readings.do_not_disturb { "on" } else { "off" }).into(), Some(readings.do_not_disturb), false),
            (ui::Icon::Brightness, "Night light", night_detail, Some(night.is_some()), false),
            (ui::Icon::Keyboard, "Keyboard", (if readings.keyboard { "on" } else { "off" }).into(), Some(readings.keyboard), false),
            (if volume == Some(0) { ui::Icon::Muted } else { ui::Icon::Volume }, "Sound", sound_detail, volume.map(|level| level > 0), true),
        ];
        for chunk in tiles.chunks(3).enumerate() {
            let (row_index, row) = chunk;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (column, (icon, name, detail, on, arrow)) in row.iter().enumerate() {
                    let hit = tile(ui, theme, size, *icon, name, detail, *on, *arrow, self.view == View::Outputs && row_index * 3 + column == 5);
                    match (row_index * 3 + column, hit) {
                        (0, Hit::Switch) => if let Some(on) = wifi.powered { radio.toggle_wifi(on); },
                        (1, Hit::Switch) => if let Some(on) = bluetooth.powered { radio.toggle_bluetooth(on); },
                        (2, Hit::Switch) => actions.push(Action::DoNotDisturb(!readings.do_not_disturb)),
                        (3, Hit::Switch) => {
                            let value = night.is_none().then(|| crate::config::night_light_value((19, 7, 4500)));
                            actions.push(Action::Change(Change::Set("night_light", value)));
                        }
                        (4, Hit::Switch) => actions.push(Action::Keyboard),
                        (5, Hit::Switch) => actions.push(Action::Mute),
                        (0, Hit::Arrow) => self.view = View::Wifi,
                        (1, Hit::Arrow) => self.view = View::Bluetooth,
                        (5, Hit::Arrow) => self.view = if self.view == View::Outputs { View::Levels } else { View::Outputs },
                        _ => {}
                    }
                }
            });
        }

        match self.view {
            View::Outputs => {
                ui.horizontal(|ui| {
                    ui::tracked(ui, "SOUND GOES TO", ui::display_bold(10.5), ui::SECONDARY, 1.2);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if link(ui, theme, "BACK TO LEVELS").clicked() { self.view = View::Levels; }
                    });
                });
                let sound = self.sound.get_or_insert_with(|| Sound::new(ui.ctx().clone()));
                sound.quick_outputs(ui, theme);
            }
            View::Levels | View::Wifi | View::Bluetooth => {
                ui.spacing_mut().item_spacing.y = 4.0;
                if let Some(level) = brightness {
                    if let Some(to) = level_row(ui, theme, ui::Icon::Brightness, None, level, "Brightness", 40.0) {
                        let to = to.max(5);
                        if to != level { self.brightness = Some((to, now)); actions.push(Action::Change(Change::Brightness(to))); }
                    }
                }
                if let Some(level) = volume {
                    let icon = if level == 0 { ui::Icon::Muted } else { ui::Icon::Volume };
                    let mut mute = false;
                    if let Some(to) = level_row(ui, theme, icon, Some(&mut mute), level, "Volume", 40.0) {
                        if to != level { self.volume = Some((to, now)); actions.push(Action::Change(Change::Volume(to))); }
                    }
                    if mute { actions.push(Action::Mute); }
                }
                ui.spacing_mut().item_spacing.y = 10.0;
            }
        }

        // The foot: lock and the settings, and the power menu set apart.
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, ui::LINE));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if round(ui, theme, ui::Icon::Lock, "Lock", false).clicked() { actions.push(Action::Lock); }
            if round(ui, theme, ui::Icon::Settings, "Settings", false).clicked() { actions.push(Action::Settings); }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if round(ui, theme, ui::Icon::Power, "Power", true).clicked() { actions.push(Action::Power); }
            });
        });
        // Wi-Fi and sound are read in the background; look again while open.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let _ = ui.allocate_exact_size(vec2(0.0, 0.0), Sense::hover());
    }
}

/// What a click on a tile hit: its switch, or the arrow at its edge that opens a list.
#[derive(Clone, Copy, PartialEq)]
enum Hit { None, Switch, Arrow }

/// What the pop-up shows under its switches.
#[derive(Clone, Copy, PartialEq, Default)]
enum View {
    #[default]
    Levels,
    Outputs,
    Wifi,
    Bluetooth,
}

/// A round button with an icon, named for the screen reader and on hover. `warm`
/// sets it apart, for the power menu.
fn round(ui: &mut Ui, theme: &Theme, icon: ui::Icon, name: &str, warm: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let painter = ui.painter();
    let tone = if warm { ui::CAUTION } else { ui::SECONDARY };
    let hovered = response.hovered() || response.has_focus();
    if hovered { painter.circle_filled(rect.center(), 20.0, if warm { ui::CAUTION.gamma_multiply(0.12) } else { ui::TRACK }); }
    painter.circle_stroke(rect.center(), 19.5, Stroke::new(1.0, if response.has_focus() { theme.main } else if warm { ui::CAUTION.gamma_multiply(0.6) } else { ui::LINE_STRONG }));
    ui::paint_icon(painter, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, if hovered && !warm { ui::TEXT } else { tone }, false);
    response.on_hover_text(name)
}

impl Quick {
    /// The quick settings as the right panel's first card: Wi-Fi, Bluetooth and Do not
    /// disturb as three rows in one group, brightness and volume as pill sliders in a
    /// second, and the battery. It asks for the same things the pull-down does; a
    /// row's arrow opens the pull-down at that list.
    /// `room` is the height it may take; with less, it keeps the switches first,
    /// then the sliders, and leaves out the rest.
    pub fn panel_card(&mut self, ui: &mut Ui, theme: &Theme, readings: &Readings, room: f32) -> Vec<Action> {
        let mut actions = Vec::new();
        let radio = self.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
        let wifi = radio.wifi_glance();
        let bluetooth = radio.bluetooth_glance();
        let now = Instant::now();
        let volume = shown(self.volume, readings.volume, now);
        let brightness = shown(self.brightness, readings.brightness, now);
        let mut open_list = None;
        crate::panels::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let sliders = brightness.is_some() as usize + volume.is_some() as usize;
            let rows_height = 3.0 * ROW + 2.0;
            let levels_height = if sliders == 0 { 0.0 } else { 14.0 + sliders as f32 * (LABEL + SLIDER) + (sliders - 1) as f32 * 6.0 };
            let with_sliders = sliders > 0 && room >= 24.0 + rows_height + 8.0 + levels_height;
            let with_battery = room >= 24.0 + rows_height + 8.0 + if with_sliders { levels_height + 8.0 } else { 0.0 } + 28.0;
            let quiet = readings.do_not_disturb;
            let state = |glance: &crate::radio::Glance| match glance.powered {
                None => "Unavailable".to_string(),
                Some(on) => glance.working.clone().or_else(|| glance.detail.clone().filter(|_| on)).unwrap_or_else(|| (if on { "On" } else { "Off" }).to_string()),
            };
            group(ui, |ui| {
                let rows: [(RowIcon, &str, String, Option<bool>, bool); 3] = [
                    (RowIcon::Wifi, "Wi-Fi", state(&wifi), wifi.powered, true),
                    (RowIcon::Plain(ui::Icon::Bluetooth), "Bluetooth", state(&bluetooth), bluetooth.powered, true),
                    (RowIcon::Plain(ui::Icon::Moon), "Do not disturb", (if quiet { "On" } else { "Off" }).to_string(), Some(quiet), false),
                ];
                for (index, (icon, name, detail, on, arrow)) in rows.into_iter().enumerate() {
                    match switch_row(ui, theme, icon, name, &detail, on, arrow, index < 2) {
                        Hit::Switch => match (index, on) {
                            (0, Some(on)) => radio.toggle_wifi(on),
                            (1, Some(on)) => radio.toggle_bluetooth(on),
                            (2, _) => actions.push(Action::DoNotDisturb(!quiet)),
                            _ => {}
                        },
                        Hit::Arrow => open_list = Some(if index == 0 { View::Wifi } else { View::Bluetooth }),
                        Hit::None => {}
                    }
                }
            });
            if with_sliders {
                group(ui, |ui| {
                    // The pills sit inside the group, in line with their labels.
                    ui.add_space(7.0);
                    if let Some(level) = brightness {
                        section_label(ui, "DISPLAY");
                        let width = ui.available_width() - PERCENT - INSET;
                        let sun = |painter: &egui::Painter, c: egui::Pos2, color: egui::Color32| ui::paint_icon(painter, Rect::from_center_size(c, vec2(14.0, 14.0)), ui::Icon::Brightness, color, false);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(INSET);
                            let (picked, _) = ui::pill_slider(ui, theme, width, level.min(100) as f32 / 100.0, "Brightness", SLIDER, false, &sun);
                            percent(ui, level);
                            if let Some(frac) = picked {
                                let to = ((frac * 100.0).round() as u8).max(5);
                                if to != level { self.brightness = Some((to, now)); actions.push(Action::Change(Change::Brightness(to))); }
                            }
                        });
                    }
                    if let Some(level) = volume {
                        if brightness.is_some() { ui.add_space(6.0 - ui.spacing().item_spacing.y); }
                        section_label(ui, "SOUND");
                        let width = ui.available_width() - PERCENT - INSET;
                        let waves = ui::volume_waves(level);
                        let speaker = move |painter: &egui::Painter, c: egui::Pos2, color: egui::Color32| ui::paint_volume(painter, c + vec2(2.0, 0.0), color, waves, level == 0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(INSET);
                            let (picked, mute) = ui::pill_slider(ui, theme, width, level.min(100) as f32 / 100.0, "Volume", SLIDER, true, &speaker);
                            percent(ui, level);
                            if mute { actions.push(Action::Mute); }
                            if let Some(frac) = picked {
                                let to = (frac * 100.0).round() as u8;
                                if to != level { self.volume = Some((to, now)); actions.push(Action::Change(Change::Volume(to))); }
                            }
                        });
                    }
                    ui.add_space(3.0);
                });
            }
            if let Some(power) = readings.power.filter(|_| with_battery) {
                battery_row(ui, theme, power, readings.battery_health);
            }
        });
        if let Some(view) = open_list {
            self.open = true;
            self.view = view;
        }
        // Wi-Fi and Bluetooth are read in the background; look again now and then.
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        actions
    }
}

/// Height of a switch row, a section label, a pill slider, and the room for a percent.
const ROW: f32 = 42.0;
const LABEL: f32 = 13.0;
const SLIDER: f32 = 22.0;
const PERCENT: f32 = 40.0;
/// How far a slider sits in from its group's left edge, as its label does.
const INSET: f32 = 10.0;
/// The fill of a group of rows inside a card: between the card and a track.
const WELL: egui::Color32 = egui::Color32::from_rgb(0x11, 0x16, 0x1C);

/// A rounded group inside a card, sized to what `add` puts in it.
fn group(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let background = ui.painter().add(egui::Shape::Noop);
    let response = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 0 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    }).response;
    let rect = response.rect;
    ui.painter().set(background, egui::Shape::Vec(vec![
        egui::Shape::rect_filled(rect, 11.0, WELL),
        egui::Shape::rect_stroke(rect, 11.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside),
    ]));
}

/// A small spaced label above a slider.
fn section_label(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), ui::mono(9.0), ui::SECONDARY);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LABEL), Sense::hover());
    ui.painter().galley(pos2(rect.left() + 11.0, rect.top()), galley, ui::SECONDARY);
}

/// A level's percent, right-aligned in its own room beside a slider.
fn percent(ui: &mut Ui, level: u8) {
    let (rect, _) = ui.allocate_exact_size(vec2(PERCENT, 30.0), Sense::hover());
    let galley = ui.painter().layout_no_wrap(format!("{level}%"), ui::mono(11.0), ui::TEXT);
    ui.painter().galley(pos2(rect.right() - 11.0 - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, ui::TEXT);
}

/// What a switch row's round icon shows.
#[derive(Clone, Copy)]
enum RowIcon { Wifi, Plain(ui::Icon) }

/// A row with a round switch, a name and its state; the switch fills with the accent
/// while on. `arrow` adds a chevron at the right that opens a list; `rule` draws the
/// line under the row. A click on the row works the switch, one on the chevron opens.
#[allow(clippy::too_many_arguments)]
fn switch_row(ui: &mut Ui, theme: &Theme, icon: RowIcon, name: &str, detail: &str, on: Option<bool>, arrow: bool, rule: bool) -> Hit {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), if on.is_some() { Sense::click() } else { Sense::hover() });
    let lit = on == Some(true);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, on.is_some(), lit, format!("{name}: {detail}")));
    let chevron = Rect::from_min_max(pos2(rect.right() - 30.0, rect.top()), rect.max);
    let arrow_response = arrow.then(|| {
        let response = ui.interact(chevron, response.id.with("arrow"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name}: show the list")));
        response
    });
    let on_arrow = arrow_response.as_ref().is_some_and(|r| r.hovered());
    let painter = ui.painter();
    if response.hovered() && !on_arrow { painter.rect_filled(rect.shrink(2.0), 9.0, ui::TRACK); }
    if response.has_focus() { painter.rect_stroke(rect.shrink(1.0), 10.0, Stroke::new(1.0, ui::TEXT), StrokeKind::Inside); }
    if rule { painter.hline(rect.x_range().shrink(10.0), rect.bottom(), Stroke::new(1.0, ui::LINE)); }
    let disc = pos2(rect.left() + 23.0, rect.center().y);
    painter.circle_filled(disc, 13.0, if lit { theme.main } else { ui::LINE });
    let ink = if lit { ui::GROUND } else if on.is_none() { ui::LABEL } else { ui::SECONDARY };
    match icon {
        RowIcon::Wifi => ui::paint_wifi(painter, disc + vec2(0.0, 0.5), ink, 3, 0.62),
        RowIcon::Plain(icon) => ui::paint_icon(painter, Rect::from_center_size(disc, vec2(14.0, 14.0)), icon, ink, lit),
    }
    let text_left = rect.left() + 46.0;
    let text_width = rect.right() - text_left - if arrow { 30.0 } else { 10.0 };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), ui::display_bold(12.5), ui::TEXT);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
    painter.galley(pos2(text_left, rect.top() + 6.0), painter.layout_job(job), ui::TEXT);
    let mut job = egui::text::LayoutJob::simple_singleline(detail.to_string(), ui::mono(10.0), ui::SECONDARY);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
    painter.galley(pos2(text_left, rect.top() + 23.0), painter.layout_job(job), ui::SECONDARY);
    if let Some(arrow_response) = &arrow_response {
        if arrow_response.hovered() { painter.rect_filled(chevron.shrink(4.0), 8.0, ui::TRACK); }
        let c = chevron.center();
        let tone = if arrow_response.hovered() { ui::TEXT } else { ui::SECONDARY };
        painter.line(vec![c + vec2(-2.0, -4.5), c + vec2(2.5, 0.0), c + vec2(-2.0, 4.5)], Stroke::new(1.6, tone));
    }
    if arrow_response.is_some_and(|r| r.on_hover_text(format!("{name}: show the list")).clicked()) { return Hit::Arrow; }
    if response.on_hover_text(format!("{name}: {detail}")).clicked() && on.is_some() { Hit::Switch } else { Hit::None }
}

/// A square switch with an icon, a name and its state, lit while on, with an arrow down its right edge when `arrow`
/// is set, that opens a list of its own; `opened` lights the arrow while it is.
#[allow(clippy::too_many_arguments)]
fn tile(ui: &mut Ui, theme: &Theme, size: egui::Vec2, icon: ui::Icon, name: &str, detail: &str, on: Option<bool>, arrow: bool, opened: bool) -> Hit {
    let (rect, response) = ui.allocate_exact_size(size, if on.is_some() { Sense::click() } else { Sense::hover() });
    let lit = on == Some(true);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, on.is_some(), lit, format!("{name}: {detail}")));
    let edge = 26.0;
    let arrow_rect = Rect::from_min_max(pos2(rect.right() - edge, rect.top()), rect.max);
    let arrow_response = arrow.then(|| {
        let response = ui.interact(arrow_rect, response.id.with("arrow"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name}: show the list")));
        response
    });
    let on_arrow = arrow_response.as_ref().is_some_and(|r| r.hovered());
    let painter = ui.painter();
    let fill = if lit { theme.main.gamma_multiply(0.14) } else if response.hovered() && !on_arrow { ui::LINE } else { ui::TRACK };
    let line = if lit || opened { theme.main } else { ui::LINE };
    painter.rect_filled(rect, ui::SURFACE_RADIUS, fill);
    painter.rect_stroke(rect, ui::SURFACE_RADIUS, Stroke::new(1.0, line), StrokeKind::Inside);
    if response.has_focus() { painter.rect_stroke(rect.expand(2.0), ui::SURFACE_RADIUS + 2.0, Stroke::new(1.0, ui::TEXT), StrokeKind::Inside); }
    let ink = if lit { theme.main } else { ui::SECONDARY };
    let text_width = rect.width() - 18.0 - if arrow { edge } else { 0.0 };
    if size.y >= 56.0 {
        ui::paint_icon(painter, Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(16.0, 16.0)), icon, ink, lit);
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), ui::display(12.5), ui::TEXT);
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        painter.galley(rect.min + vec2(10.0, 30.0), painter.layout_job(job), ui::TEXT);
        let mut job = egui::text::LayoutJob::simple_singleline(detail.to_uppercase(), ui::mono(9.5), if lit { theme.main } else { ui::LABEL });
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        let galley = painter.layout_job(job);
        painter.galley(rect.min + vec2(10.0, 46.0), galley, ui::LABEL);
    } else {
        ui::paint_icon(painter, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, ink, lit);
    }
    if let Some(arrow_response) = &arrow_response {
        if arrow_response.hovered() || opened {
            painter.rect_filled(arrow_rect.shrink(1.0), egui::CornerRadius { nw: 0, sw: 0, ne: (ui::SURFACE_RADIUS - 1.0) as u8, se: (ui::SURFACE_RADIUS - 1.0) as u8 }, theme.main.gamma_multiply(0.12));
        }
        painter.vline(arrow_rect.left(), arrow_rect.y_range().shrink(10.0), Stroke::new(1.0, if lit { theme.main.gamma_multiply(0.5) } else { ui::LINE }));
        let c = arrow_rect.center();
        let tone = if lit || opened || arrow_response.hovered() { theme.main } else { ui::SECONDARY };
        painter.line(vec![c + vec2(-2.0, -5.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 5.0)], Stroke::new(1.6, tone));
    }
    let arrow_clicked = arrow_response.is_some_and(|r| r.on_hover_text(format!("{name}: show the list")).clicked());
    if arrow_clicked { return Hit::Arrow; }
    if response.on_hover_text(format!("{name}: {detail}")).clicked() { Hit::Switch } else { Hit::None }
}

/// A word that does something, in the accent.
fn link(ui: &mut Ui, theme: &Theme, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), ui::mono(10.5), theme.main);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, text));
    let tone = if response.hovered() { ui::TEXT } else { theme.main };
    ui.painter().galley(rect.min, galley, tone);
    response
}

/// An icon, a slider and its percent. The icon mutes when `mute` is given.
fn level_row(ui: &mut Ui, theme: &Theme, icon: ui::Icon, mute: Option<&mut bool>, level: u8, name: &str, height: f32) -> Option<u8> {
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        match mute {
            Some(mute) => { *mute = ui::icon_button(ui, theme, icon, 24.0, level == 0).on_hover_text("Mute or unmute").clicked(); }
            None => { ui::icon(ui, icon, 24.0, ui::SECONDARY); }
        }
        let width = (ui.available_width() - 48.0).max(20.0);
        if let Some(frac) = ui::slider_sized(ui, theme, width, level.min(100) as f32 / 100.0, name, height) {
            changed = Some((frac * 100.0).round() as u8);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui::text(ui, &format!("{level}%"), ui::mono(12.0), ui::TEXT); });
    });
    changed
}

/// The battery: its icon, how full, how long it lasts or takes, and how much it
/// still holds as a small tag.
fn battery_row(ui: &mut Ui, theme: &Theme, power: crate::sysmon::Battery, health: Option<u8>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let painter = ui.painter();
    let low = !power.plugged && power.percent < 20;
    let charging = power.plugged && power.percent < 100;
    let ink = if low { ui::CAUTION } else if charging { theme.main } else { ui::TEXT };
    ui::paint_battery(painter, pos2(rect.left() + 18.0, rect.center().y), ink, power.percent, charging, power.plugged && power.percent >= 100, 1.0);
    let figure = painter.layout_no_wrap(format!("{}%", power.percent), ui::display_bold(16.0), if low { ui::CAUTION } else { ui::TEXT });
    let figure_width = figure.size().x;
    painter.galley(pos2(rect.left() + 38.0, rect.center().y - figure.size().y / 2.0), figure, ui::TEXT);
    let left = match (power.plugged, power.minutes) {
        (true, Some(m)) => format!("full in {}", crate::power::duration(m).to_lowercase()),
        (true, None) => if power.percent >= 100 { "full".to_string() } else { "on mains".to_string() },
        (false, Some(m)) => format!("{} left", crate::power::duration(m).to_lowercase()),
        (false, None) => String::new(),
    };
    let left = painter.layout_no_wrap(left, ui::mono(10.5), ui::SECONDARY);
    let left_x = rect.left() + 46.0 + figure_width;
    let left_end = left_x + left.size().x;
    painter.galley(pos2(left_x, rect.center().y - left.size().y / 2.0), left, ui::SECONDARY);
    // How much it still holds, as a tag where there is room beside the rest.
    if let Some(health) = health {
        let tag = painter.layout_no_wrap(format!("HEALTH {health}%"), ui::mono(9.0), ui::SECONDARY);
        let size = tag.size() + vec2(14.0, 6.0);
        let chip = Rect::from_min_size(pos2(rect.right() - 4.0 - size.x, rect.center().y - size.y / 2.0), size);
        if left_end + 10.0 <= chip.left() {
            painter.rect_stroke(chip, size.y / 2.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
            painter.galley(chip.center() - tag.size() / 2.0, tag, ui::SECONDARY);
        }
    }
    response.on_hover_text(crate::power::battery_line(power));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sits_above_the_status_strip_at_the_right_and_fits_small_screens() {
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0));
        let rect = area(screen, 1052.0, None);
        assert_eq!(rect.right(), 1920.0 - MARGIN);
        assert_eq!(rect.bottom(), 1052.0 - MARGIN);
        assert_eq!(rect.width(), WIDTH);
        assert_eq!(rect.height(), INITIAL_HEIGHT);
        let small = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 400.0));
        let rect = area(small, 372.0, None);
        assert!(rect.width() <= 300.0 - 2.0 * MARGIN && rect.top() >= MARGIN);
    }

    #[test]
    fn sizes_to_content_and_clamps_for_small_displays() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        assert_eq!(area(screen, 1052.0, Some(300.0)).height(), 300.0 + 2.0 * PADDING);
        assert_eq!(area(screen, 1052.0, Some(2000.0)).height(), MAX_HEIGHT);
        let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 400.0));
        assert!(area(small, 372.0, Some(2000.0)).top() >= MARGIN);
    }

    #[test]
    fn drawn_area_tracks_actual_frame_while_content_is_measured() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut quick = Quick { open: true, ..Default::default() };
        let readings = Readings { volume: Some(40), brightness: Some(100), ..Default::default() };
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let initial = quick.area(screen, 772.0);
        frame(&mut quick, &ctx, Vec::new(), &readings);
        assert_eq!(quick.drawn_area(), Some(initial));
        let next = quick.area(screen, 772.0);
        frame(&mut quick, &ctx, Vec::new(), &readings);
        assert_eq!(quick.drawn_area(), Some(next));
        assert!(quick.content_height.is_some());
        quick.open = false;
        assert_eq!(quick.drawn_area(), None);
    }

    #[test]
    fn frame_outlines_are_on_a_lower_layer_than_floating_controls() {
        assert!(crate::runtime::frame_lines_layer().order < Order::Foreground);
    }

    #[test]
    fn a_level_just_set_is_shown_until_the_system_catches_up() {
        let now = Instant::now();
        assert_eq!(shown(Some((30, now)), Some(80), now), Some(30));
        assert_eq!(shown(Some((30, now)), Some(80), now + HOLD), Some(80));
        assert_eq!(shown(None, Some(80), now), Some(80));
    }

    fn frame(quick: &mut Quick, ctx: &egui::Context, events: Vec<egui::Event>, readings: &Readings) -> Vec<Action> {
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1280.0, 800.0));
        let mut actions = Vec::new();
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let _ = ctx.run(input, |ctx| actions = quick.show(ctx, &Theme::load("signal"), screen, 772.0, readings));
        actions
    }

    #[test]
    fn its_switches_ask_for_their_changes_and_escape_closes_it() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let mut quick = Quick { open: true, ..Default::default() };
        let readings = Readings { do_not_disturb: false, keyboard: false, ..Default::default() };
        // A first frame lays it out; then a click on the Do-not-disturb switch.
        assert!(frame(&mut quick, &ctx, Vec::new(), &readings).is_empty());
        let mut found = None;
        for y in (300..760).step_by(4) {
            let at = pos2(1280.0 - MARGIN - 40.0, y as f32);
            let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            let actions = frame(&mut quick, &ctx, vec![egui::Event::PointerMoved(at), press(true), press(false)], &readings);
            if actions.contains(&Action::DoNotDisturb(true)) {
                found = Some(y);
                break;
            }
            quick.open = true;
        }
        assert!(found.is_some(), "the Do not disturb switch answers a click");
        assert!(quick.open);
        let escape = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&mut quick, &ctx, vec![escape], &readings);
        assert!(!quick.open);
    }

    /// The right panel's card drawn `width` wide with `room` to spare: the labels of
    /// its switches and sliders, and where they went.
    fn card(width: f32, room: f32) -> (Rect, Vec<(String, egui::accesskit::Role, egui::accesskit::Rect)>) {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None); ctx.enable_accesskit();
        let mut quick = Quick::default();
        let readings = Readings { volume: Some(40), brightness: Some(100), power: Some(crate::sysmon::Battery { percent: 48, plugged: false, minutes: Some(50) }), battery_health: Some(67), ..Default::default() };
        let area = Rect::from_min_size(pos2(20.0, 20.0), vec2(width, room));
        let output = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width + 40.0, room + 40.0))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
                quick.panel_card(&mut child, &Theme::load("signal"), &readings, room);
            });
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        let nodes = tree.nodes.iter().filter_map(|(_, node)| Some((node.label()?.to_string(), node.role(), node.bounds()?))).collect();
        (area, nodes)
    }

    #[test]
    fn the_panel_card_groups_its_switches_and_sliders_inside_the_card() {
        for width in [252.0, 292.0] {
            let (area, nodes) = card(width, 400.0);
            for name in ["Wi-Fi", "Bluetooth", "Do not disturb"] {
                assert!(nodes.iter().any(|(label, role, _)| *role == egui::accesskit::Role::CheckBox && label.starts_with(name)), "{name} switch at {width}");
            }
            for name in ["Brightness", "Volume"] {
                let (_, _, bounds) = nodes.iter().find(|(label, role, _)| *role == egui::accesskit::Role::Slider && label == name).expect(name);
                assert!(bounds.x0 >= area.left() as f64 && bounds.x1 <= area.right() as f64, "{name} outside the card at {width}: {bounds:?}");
                assert!(bounds.y1 - bounds.y0 >= 30.0, "{name} needs a drag target");
            }
            assert!(nodes.iter().any(|(label, _, _)| label == "Wi-Fi: show the list"));
        }
    }

    #[test]
    fn a_short_panel_keeps_the_switches_and_drops_the_sliders_first() {
        let (_, nodes) = card(272.0, 170.0);
        assert!(nodes.iter().any(|(label, _, _)| label.starts_with("Wi-Fi")));
        assert!(!nodes.iter().any(|(label, role, _)| *role == egui::accesskit::Role::Slider && label == "Volume"));
    }

    #[test]
    fn primary_sliders_fit_narrow_and_standard_panels() {
        for width in [300.0, 1280.0] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None); ctx.enable_accesskit();
            let mut quick = Quick { open: true, ..Default::default() };
            let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 1080.0));
            let readings = Readings { volume: Some(40), brightness: Some(80), ..Default::default() };
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                    quick.show(ctx, &Theme::load("signal"), screen, 1052.0, &readings);
                }));
            }
            let panel = quick.drawn_area().unwrap();
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in ["Volume", "Brightness"] {
                let bounds = tree.nodes.iter().find_map(|(_, node)|
                    (node.role() == egui::accesskit::Role::Slider && node.label() == Some(label)).then(|| node.bounds()).flatten()).expect(label);
                assert!(bounds.x0 >= panel.left() as f64 && bounds.x1 <= panel.right() as f64, "{label} outside panel at width {width}: {bounds:?}");
                assert!(bounds.y1 - bounds.y0 >= 40.0, "{label} needs a large drag target");
            }
        }
    }

    #[test]
    fn output_choices_expand_and_collapse_without_changing_volume() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None); ctx.enable_accesskit();
        let mut quick = Quick { open: true, ..Default::default() };
        let readings = Readings { volume: Some(40), brightness: Some(80), ..Default::default() };
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 900.0));
        let render = |quick: &mut Quick, events| {
            let mut actions = Vec::new();
            let output = ctx.run(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ctx| {
                actions = quick.show(ctx, &Theme::load("signal"), screen, 872.0, &readings);
            });
            (output, actions)
        };
        render(&mut quick, Vec::new());
        let (output, _) = render(&mut quick, Vec::new());
        let bounds = output.platform_output.accesskit_update.unwrap().nodes.into_iter().find_map(|(_, node)|
            (node.label() == Some("Sound: show the list")).then(|| node.bounds()).flatten()).expect("sound arrow");
        let at = pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let (_, first) = render(&mut quick, vec![egui::Event::PointerMoved(at), click(true)]);
        let (_, second) = render(&mut quick, vec![click(false)]);
        assert!(quick.view == View::Outputs);
        assert!(first.is_empty() && second.is_empty(), "opening output choices must not alter system settings");
        render(&mut quick, Vec::new());
        let (expanded, _) = render(&mut quick, Vec::new());
        let bounds = expanded.platform_output.accesskit_update.unwrap().nodes.into_iter().find_map(|(_, node)|
            (node.label() == Some("BACK TO LEVELS")).then(|| node.bounds()).flatten()).expect("back to levels");
        let at = pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        render(&mut quick, vec![egui::Event::PointerMoved(at), click(true)]);
        render(&mut quick, vec![click(false)]);
        assert!(quick.view == View::Levels);
    }

    #[test]
    fn the_wifi_arrow_opens_its_list_and_back_returns_without_changing_anything() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None); ctx.enable_accesskit();
        let mut quick = Quick { open: true, ..Default::default() };
        let readings = Readings { volume: Some(40), brightness: Some(80), ..Default::default() };
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 900.0));
        let render = |quick: &mut Quick, events| {
            let mut actions = Vec::new();
            let output = ctx.run(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ctx| {
                actions = quick.show(ctx, &Theme::load("signal"), screen, 872.0, &readings);
            });
            (output, actions)
        };
        let press = |quick: &mut Quick, label: &str| {
            render(quick, Vec::new());
            let (output, _) = render(quick, Vec::new());
            let bounds = output.platform_output.accesskit_update.unwrap().nodes.into_iter().find_map(|(_, node)|
                (node.label() == Some(label)).then(|| node.bounds()).flatten()).unwrap_or_else(|| panic!("{label}"));
            let at = pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
            let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            let (_, first) = render(quick, vec![egui::Event::PointerMoved(at), click(true)]);
            let (_, second) = render(quick, vec![click(false)]);
            first.into_iter().chain(second).collect::<Vec<_>>()
        };
        assert!(press(&mut quick, "Wi-Fi: show the list").is_empty(), "opening the list must not switch Wi-Fi");
        assert!(quick.view == View::Wifi);
        assert!(press(&mut quick, "Back to quick settings").is_empty());
        assert!(quick.view == View::Levels);
        assert!(press(&mut quick, "Bluetooth: show the list").is_empty());
        assert!(quick.view == View::Bluetooth);
    }

    #[test]
    fn a_click_outside_closes_it_but_not_one_on_the_status_strip() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let mut quick = Quick { open: true, ..Default::default() };
        let readings = Readings::default();
        let click = |at| {
            let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(at), press(true)]
        };
        frame(&mut quick, &ctx, click(pos2(1200.0, 790.0)), &readings);
        assert!(quick.open, "the strip's own button toggles it");
        frame(&mut quick, &ctx, click(pos2(100.0, 100.0)), &readings);
        assert!(!quick.open);
    }
}
