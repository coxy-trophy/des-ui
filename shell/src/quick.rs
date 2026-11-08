//! Quick settings: the things changed many times a day, in one pull-down above the
//! right end of the status strip, without opening the settings. Wi-Fi, Bluetooth,
//! sound, brightness, the night light, Do not disturb and the on-screen keyboard,
//! with the battery and the ways to lock, the settings and the power menu.
//!
//! It is drawn by the shell and lifted above applications with the compositor's
//! overlay, as the scratchpad is; while both are open the quick settings take the
//! overlay, since they are the one in use.

use crate::radio::Radio;
use crate::settings::{self, Change};
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
    pub battery: Option<String>,
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
    outputs_open: bool,
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
        ui.spacing_mut().item_spacing.y = 6.0;
        ui::tracked(ui, "QUICK SETTINGS", ui::display_bold(13.0), ui::SECONDARY, 0.8);
        if let Some(battery) = &readings.battery {
            ui.add(egui::Label::new(egui::RichText::new(battery).font(ui::mono(11.0)).color(ui::SECONDARY)).wrap());
        }
        ui.add_space(8.0);

        let radio = self.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
        let wifi = radio.wifi_glance();
        let bluetooth = radio.bluetooth_glance();
        ui::tile(ui, theme, false, |ui| {
            ui::tracked(ui, "CONNECTIVITY", ui::display_bold(11.0), ui::SECONDARY, 1.0);
            for (name, glance, is_wifi) in [("Wi-Fi", &wifi, true), ("Bluetooth", &bluetooth, false)] {
                if let Some(on) = glance.powered {
                    let what = glance.working.as_deref().or(glance.detail.as_deref()).unwrap_or(if on { "On" } else { "Off" });
                    if settings::switch_row(ui, theme, name, what, on) {
                        if is_wifi { radio.toggle_wifi(on); } else { radio.toggle_bluetooth(on); }
                    }
                } else {
                    ui::text(ui, &format!("{name} · unavailable"), ui::display(12.0), ui::SECONDARY);
                }
            }
        });
        ui.add_space(4.0);

        let now = Instant::now();
        if let Some(volume) = shown(self.volume, readings.volume, now) {
            ui::tile(ui, theme, false, |ui| {
                ui.horizontal(|ui| {
                    let icon = if volume == 0 { ui::Icon::Muted } else { ui::Icon::Volume };
                    if ui::icon_button(ui, theme, icon, ui::CONTROL_HEIGHT, volume == 0).on_hover_text("Mute or unmute").clicked() {
                        actions.push(Action::Mute);
                    }
                    ui::text(ui, "Volume", ui::display_bold(14.0), ui::TEXT);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui::text(ui, &format!("{volume}%"), ui::mono(12.0), ui::TEXT);
                    });
                });
                let width = ui.available_width();
                if let Some(frac) = ui::prominent_slider(ui, theme, width, volume.min(100) as f32 / 100.0, "Volume") {
                    let level = (frac * 100.0).round() as u8;
                    if level != volume { self.volume = Some((level, now)); actions.push(Action::Change(Change::Volume(level))); }
                }
                let label = if self.outputs_open { "HIDE OUTPUTS" } else { "CHOOSE OUTPUT" };
                if ui::button(ui, theme, label, None, self.outputs_open).clicked() { self.outputs_open = !self.outputs_open; }
                if self.outputs_open {
                    let sound = self.sound.get_or_insert_with(|| Sound::new(ui.ctx().clone()));
                    sound.quick_outputs(ui, theme);
                }
            });
            ui.add_space(4.0);
        }
        if let Some(brightness) = shown(self.brightness, readings.brightness, now) {
            ui::tile(ui, theme, false, |ui| {
                ui.horizontal(|ui| {
                    ui::icon(ui, ui::Icon::Brightness, ui::CONTROL_HEIGHT, ui::SECONDARY);
                    ui::text(ui, "Brightness", ui::display_bold(14.0), ui::TEXT);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui::text(ui, &format!("{brightness}%"), ui::mono(12.0), ui::TEXT);
                    });
                });
                let width = ui.available_width();
                if let Some(frac) = ui::prominent_slider(ui, theme, width, brightness.min(100) as f32 / 100.0, "Brightness") {
                    let level = ((frac * 100.0).round() as u8).max(5);
                    if level != brightness { self.brightness = Some((level, now)); actions.push(Action::Change(Change::Brightness(level))); }
                }
            });
            ui.add_space(4.0);
        }
        ui::tile(ui, theme, false, |ui| {
            ui::tracked(ui, "CONTROLS", ui::display_bold(11.0), ui::SECONDARY, 1.0);
            let night = readings.night_light.is_some();
            if settings::switch_row(ui, theme, "Night light", if night { "Warmer colours in the evening" } else { "Off" }, night) {
                let value = (!night).then(|| crate::config::night_light_value((19, 7, 4500)));
                actions.push(Action::Change(Change::Set("night_light", value)));
            }
            if settings::switch_row(ui, theme, "Do not disturb", "Notifications wait in the list", readings.do_not_disturb) {
                actions.push(Action::DoNotDisturb(!readings.do_not_disturb));
            }
            if settings::switch_row(ui, theme, "On-screen keyboard", "Keys along the foot of the desktop", readings.keyboard) {
                actions.push(Action::Keyboard);
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (label, action) in [("LOCK", Action::Lock), ("SETTINGS", Action::Settings), ("POWER", Action::Power)] {
                if ui::button(ui, theme, label, None, false).clicked() { actions.push(action); }
            }
        });
        // Wi-Fi and sound are read in the background; look again while open.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let _ = ui.allocate_exact_size(vec2(0.0, 0.0), Sense::hover());
    }
}

impl Quick {
    /// The quick settings as the right panel's first card: Wi-Fi, Bluetooth and Do not
    /// disturb as three switches, brightness and volume as sliders, and the battery.
    /// It asks for the same things the pull-down does.
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
        crate::panels::card(ui, |ui| {
            let gap = 6.0;
            let width = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
            let height = if width >= 74.0 && room >= 200.0 { 62.0 } else { 40.0 };
            let sliders = brightness.is_some() as usize + volume.is_some() as usize;
            let with_sliders = room >= 24.0 + height + 7.0 + 31.0 * sliders as f32 + 8.0;
            let with_battery = room >= 24.0 + height + 7.0 + 31.0 * sliders as f32 + 38.0 + 8.0;
            let quiet = readings.do_not_disturb;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let state = |glance: &crate::radio::Glance| match glance.powered {
                    None => "—".to_string(),
                    Some(on) => glance.working.clone().or_else(|| glance.detail.clone().filter(|_| on)).unwrap_or_else(|| (if on { "ON" } else { "OFF" }).to_string()),
                };
                let tiles = [
                    (ui::Icon::Wifi, "Wi-Fi", state(&wifi), wifi.powered, 0),
                    (ui::Icon::Bluetooth, "Bluetooth", state(&bluetooth), bluetooth.powered, 1),
                    (ui::Icon::Moon, "Quiet", (if quiet { "ON" } else { "OFF" }).to_string(), Some(quiet), 2),
                ];
                for (icon, name, detail, on, which) in tiles {
                    if switch_tile(ui, theme, vec2(width, height), icon, name, &detail, on) {
                        match (which, on) {
                            (0, Some(on)) => radio.toggle_wifi(on),
                            (1, Some(on)) => radio.toggle_bluetooth(on),
                            (2, _) => actions.push(Action::DoNotDisturb(!quiet)),
                            _ => {}
                        }
                    }
                }
            });
            if let Some(level) = brightness.filter(|_| with_sliders) {
                if let Some(to) = level_row(ui, theme, ui::Icon::Brightness, None, level, "Brightness") {
                    let to = to.max(5);
                    if to != level { self.brightness = Some((to, now)); actions.push(Action::Change(Change::Brightness(to))); }
                }
            }
            if let Some(level) = volume.filter(|_| with_sliders) {
                let icon = if level == 0 { ui::Icon::Muted } else { ui::Icon::Volume };
                let mut mute = false;
                if let Some(to) = level_row(ui, theme, icon, Some(&mut mute), level, "Volume") {
                    if to != level { self.volume = Some((to, now)); actions.push(Action::Change(Change::Volume(to))); }
                }
                if mute { actions.push(Action::Mute); }
            }
            if let Some(power) = readings.power.filter(|_| with_battery) {
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, ui::LINE));
                battery_row(ui, power, readings.battery_health);
            }
        });
        // Wi-Fi and Bluetooth are read in the background; look again now and then.
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        actions
    }
}

/// A square switch with an icon, a name and its state; lit while on. True on a click.
/// `on` of `None` means it cannot be switched here.
fn switch_tile(ui: &mut Ui, theme: &Theme, size: egui::Vec2, icon: ui::Icon, name: &str, detail: &str, on: Option<bool>) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, if on.is_some() { Sense::click() } else { Sense::hover() });
    let lit = on == Some(true);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, on.is_some(), lit, format!("{name}: {detail}")));
    let painter = ui.painter();
    let fill = if lit { theme.main.gamma_multiply(0.14) } else if response.hovered() { ui::LINE } else { ui::TRACK };
    painter.rect_filled(rect, ui::SURFACE_RADIUS, fill);
    painter.rect_stroke(rect, ui::SURFACE_RADIUS, Stroke::new(1.0, if lit { theme.main } else { ui::LINE }), StrokeKind::Inside);
    if response.has_focus() { painter.rect_stroke(rect.expand(2.0), ui::SURFACE_RADIUS + 2.0, Stroke::new(1.0, ui::TEXT), StrokeKind::Inside); }
    let ink = if lit { theme.main } else { ui::SECONDARY };
    if size.y >= 56.0 {
        ui::paint_icon(painter, Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(16.0, 16.0)), icon, ink, lit);
        painter.text(rect.min + vec2(10.0, 30.0), egui::Align2::LEFT_TOP, name, ui::display(12.5), ui::TEXT);
        let mut job = egui::text::LayoutJob::simple_singleline(detail.to_uppercase(), ui::mono(9.5), ui::LABEL);
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 18.0);
        let galley = painter.layout_job(job);
        painter.galley(rect.min + vec2(10.0, 46.0), galley, ui::LABEL);
    } else {
        ui::paint_icon(painter, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, ink, lit);
    }
    response.on_hover_text(format!("{name}: {detail}")).clicked()
}

/// An icon, a slider and its percent. The icon mutes when `mute` is given.
fn level_row(ui: &mut Ui, theme: &Theme, icon: ui::Icon, mute: Option<&mut bool>, level: u8, name: &str) -> Option<u8> {
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        match mute {
            Some(mute) => { *mute = ui::icon_button(ui, theme, icon, 24.0, level == 0).on_hover_text("Mute or unmute").clicked(); }
            None => { ui::icon(ui, icon, 24.0, ui::SECONDARY); }
        }
        let width = (ui.available_width() - 48.0).max(20.0);
        if let Some(frac) = ui::slider_sized(ui, theme, width, level.min(100) as f32 / 100.0, name, 22.0) {
            changed = Some((frac * 100.0).round() as u8);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui::text(ui, &format!("{level}%"), ui::mono(12.0), ui::TEXT); });
    });
    changed
}

/// The battery: how full, how long it lasts or takes, and how much it still holds.
fn battery_row(ui: &mut Ui, power: crate::sysmon::Battery, health: Option<u8>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter();
    let icon = if power.plugged { ui::Icon::Charging } else { ui::Icon::Battery };
    ui::paint_icon(painter, Rect::from_center_size(rect.left_center() + vec2(9.0, 0.0), vec2(18.0, 18.0)), icon, ui::SECONDARY, false);
    let low = !power.plugged && power.percent <= 15;
    let figure = painter.layout_no_wrap(format!("{}%", power.percent), ui::display_bold(15.0), if low { ui::CAUTION } else { ui::TEXT });
    let figure_width = figure.size().x;
    painter.galley(pos2(rect.left() + 26.0, rect.center().y - figure.size().y / 2.0), figure, ui::TEXT);
    let left = match (power.plugged, power.minutes) {
        (true, Some(m)) => format!("full in {}", crate::power::duration(m).to_lowercase()),
        (true, None) => "on mains".to_string(),
        (false, Some(m)) => format!("{} left", crate::power::duration(m).to_lowercase()),
        (false, None) => String::new(),
    };
    painter.text(pos2(rect.left() + 34.0 + figure_width, rect.center().y), egui::Align2::LEFT_CENTER, left, ui::mono(11.0), ui::SECONDARY);
    if let Some(health) = health.filter(|_| rect.width() >= 230.0) {
        painter.text(rect.right_center(), egui::Align2::RIGHT_CENTER, format!("HEALTH {health}%"), ui::mono(10.0), ui::LABEL);
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
        let readings = Readings { volume: Some(40), brightness: Some(100), battery: Some("Battery at 84%. About 52 minutes left.".into()), ..Default::default() };
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
            (node.label() == Some("CHOOSE OUTPUT")).then(|| node.bounds()).flatten()).expect("output button");
        let at = pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let (_, first) = render(&mut quick, vec![egui::Event::PointerMoved(at), click(true)]);
        let (_, second) = render(&mut quick, vec![click(false)]);
        assert!(quick.outputs_open);
        assert!(first.is_empty() && second.is_empty(), "opening output choices must not alter system settings");
        render(&mut quick, Vec::new());
        let (expanded, _) = render(&mut quick, Vec::new());
        let bounds = expanded.platform_output.accesskit_update.unwrap().nodes.into_iter().find_map(|(_, node)|
            (node.label() == Some("HIDE OUTPUTS")).then(|| node.bounds()).flatten()).expect("hide outputs button");
        let at = pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        render(&mut quick, vec![egui::Event::PointerMoved(at), click(true)]);
        render(&mut quick, vec![click(false)]);
        assert!(!quick.outputs_open);
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
