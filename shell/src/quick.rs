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

const WIDTH: f32 = 360.0;
const HEIGHT: f32 = 560.0;
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
}

#[derive(Default)]
pub struct Quick {
    pub open: bool,
    radio: Option<Radio>,
    sound: Option<Sound>,
    /// Levels being dragged: the value and when, held until the system reports it.
    volume: Option<(u8, Instant)>,
    brightness: Option<(u8, Instant)>,
}

/// Where the panel goes: at the right of the screen, its foot just above the status strip.
pub fn area(screen: Rect, status_top: f32) -> Rect {
    let width = WIDTH.min(screen.width() - 2.0 * MARGIN).max(0.0);
    let bottom = status_top - MARGIN;
    let height = HEIGHT.min(bottom - screen.top() - MARGIN).max(0.0);
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
        let rect = area(screen, status_top);
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
        egui::Area::new(egui::Id::new("quick-settings"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                crate::material::paint(ui.ctx(), ui.painter(), rect, ui::SURFACE_RADIUS, crate::material::Role::Floating);
                ui.painter().rect_stroke(rect, ui::SURFACE_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
                let inner = rect.shrink(18.0);
                let mut content = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(inner.height()).show(&mut content, |ui| {
                    self.contents(ui, theme, readings, &mut actions);
                });
            });
        actions
    }

    fn contents(&mut self, ui: &mut Ui, theme: &Theme, readings: &Readings, actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui::section(ui, "QUICK SETTINGS", |ui| {
            if let Some(battery) = &readings.battery {
                ui::meta(ui, battery, ui::SECONDARY);
            }
        });

        let radio = self.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
        let wifi = radio.wifi_glance();
        if let Some(on) = wifi.powered {
            let what = wifi.working.clone().or_else(|| wifi.detail.clone()).unwrap_or_else(|| if on { "Not connected".into() } else { "Off".into() });
            if settings::switch_row(ui, theme, "Wi-Fi", &what, on) {
                radio.toggle_wifi(on);
            }
        }
        let bluetooth = radio.bluetooth_glance();
        if let Some(on) = bluetooth.powered {
            let what = bluetooth.working.clone().or_else(|| bluetooth.detail.clone()).unwrap_or_else(|| if on { "On".into() } else { "Off".into() });
            if settings::switch_row(ui, theme, "Bluetooth", &what, on) {
                radio.toggle_bluetooth(on);
            }
        }

        let now = Instant::now();
        ui.add_space(8.0);
        if let Some(volume) = shown(self.volume, readings.volume, now) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let icon = if volume == 0 { ui::Icon::Muted } else { ui::Icon::Volume };
                if ui::icon_button(ui, theme, icon, 22.0, volume == 0).on_hover_text("Mute or unmute").clicked() {
                    actions.push(Action::Mute);
                }
                let width = (ui.available_width() - 46.0).max(60.0);
                if let Some(frac) = ui::slider(ui, theme, width, volume.min(100) as f32 / 100.0, "Volume") {
                    let level = (frac * 100.0).round() as u8;
                    if level != volume {
                        self.volume = Some((level, now));
                        actions.push(Action::Change(Change::Volume(level)));
                    }
                }
                ui::text(ui, &format!("{volume}%"), ui::mono(12.0), ui::TEXT);
            });
        }
        let sound = self.sound.get_or_insert_with(|| Sound::new(ui.ctx().clone()));
        sound.quick_outputs(ui, theme);
        if let Some(brightness) = shown(self.brightness, readings.brightness, now) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui::icon(ui, ui::Icon::Brightness, 22.0, ui::SECONDARY);
                let width = (ui.available_width() - 46.0).max(60.0);
                if let Some(frac) = ui::slider(ui, theme, width, brightness.min(100) as f32 / 100.0, "Brightness") {
                    // Never all the way to black: a dark screen with no way back is a trap.
                    let level = ((frac * 100.0).round() as u8).max(5);
                    if level != brightness {
                        self.brightness = Some((level, now));
                        actions.push(Action::Change(Change::Brightness(level)));
                    }
                }
                ui::text(ui, &format!("{brightness}%"), ui::mono(12.0), ui::TEXT);
            });
        }

        ui.add_space(8.0);
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

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (label, action) in [("LOCK", Action::Lock), ("SETTINGS", Action::Settings), ("POWER", Action::Power)] {
                if ui::button(ui, theme, label, None, false).clicked() {
                    actions.push(action);
                }
            }
        });
        // Wi-Fi and sound are read in the background; look again while open.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let _ = ui.allocate_exact_size(vec2(0.0, 0.0), Sense::hover());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sits_above_the_status_strip_at_the_right_and_fits_small_screens() {
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0));
        let rect = area(screen, 1052.0);
        assert_eq!(rect.right(), 1920.0 - MARGIN);
        assert_eq!(rect.bottom(), 1052.0 - MARGIN);
        assert_eq!(rect.width(), WIDTH);
        assert_eq!(rect.height(), HEIGHT);
        let small = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 400.0));
        let rect = area(small, 372.0);
        assert!(rect.width() <= 300.0 - 2.0 * MARGIN && rect.top() >= MARGIN);
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
