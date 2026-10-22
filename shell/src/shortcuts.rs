//! Registry-backed shortcut reference and Settings editor.
use eframe::egui::{self, Key, Modifiers, RichText};
use des_common::shortcuts::{Bindings, Chord, DEFINITIONS};
use crate::{theme::Theme, ui};

/// egui and compositor routing use the same canonical chord spelling.
pub fn chord_for_event(key: Key, mods: Modifiers) -> Option<Chord> {
    let mut name = String::new();
    if mods.ctrl { name.push_str("Ctrl+"); }
    if mods.alt { name.push_str("Alt+"); }
    if mods.shift { name.push_str("Shift+"); }
    if mods.mac_cmd { name.push_str("Super+"); }
    name.push_str(match key {
        Key::ArrowLeft => "Left", Key::ArrowRight => "Right", Key::ArrowUp => "Up", Key::ArrowDown => "Down",
        Key::Questionmark => "Slash", Key::Colon => "Semicolon", Key::Pipe => "Backslash",
        Key::OpenCurlyBracket => "BracketLeft", Key::CloseCurlyBracket => "BracketRight",
        Key::Exclamationmark => "1", Key::Plus => "Equal",
        _ => key.name(),
    });
    Chord::parse(&name).ok()
}

pub fn hint(ctx: &egui::Context, label: &str, id: &str) -> String {
    let chord = active(ctx).binding(id);
    if chord.is_empty() { label.into() } else { format!("{label} ({chord})") }
}

/// Older workspace widgets poll raw events directly. Hold these until the
/// reference renders, in addition to egui's native modal interaction barrier.
pub fn take_modal_input(ctx: &egui::Context, open: bool) -> Vec<egui::Event> {
    if open { ctx.input_mut(|input| std::mem::take(&mut input.events)) } else { Vec::new() }
}

/// Share the live bindings with pane-local event handlers.
pub fn publish(ctx: &egui::Context, bindings: &Bindings) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("des-ui-shortcut-bindings"), bindings.clone()));
}
pub fn active(ctx: &egui::Context) -> Bindings {
    ctx.data(|data| data.get_temp(egui::Id::new("des-ui-shortcut-bindings"))).unwrap_or_default()
}
pub fn matches_event(ctx: &egui::Context, id: &str, key: Key, mods: Modifiers) -> bool {
    chord_for_event(key, mods).is_some_and(|chord| active(ctx).matches(id, &chord))
}

#[derive(Default)]
pub struct Reference { pub open: bool, search: String, focused: bool }
impl Reference {
    pub fn close(&mut self) { self.open = false; self.focused = false; }
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, bindings: &Bindings) -> bool {
        if !self.open { self.focused = false; return false; }
        let mut configure = false;
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("shortcut-reference"))
            .frame(egui::Frame::NONE).show(ctx, |ui| {
            crate::material::surface(ui, egui::Frame::NONE.inner_margin(16).corner_radius(ui::SURFACE_RADIUS as u8), crate::material::Role::Floating, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.set_width((ctx.screen_rect().width() - 80.0).clamp(1.0, 640.0));
            ui.label(RichText::new("Keyboard shortcuts").font(ui::display_bold(22.0)));
            ui.label(RichText::new("Your current mappings, across the desktop.").font(ui::display(13.0)).color(ui::SECONDARY));
            ui.add_space(ui::SPACE);
            ui.vertical(|ui| {
                let search = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Find an action or shortcut").desired_width(ui.available_width()));
                if !self.focused { search.request_focus(); self.focused = true; }
                if ui.button("Clear").clicked() { self.search.clear(); search.request_focus(); }
            });
            ui.add_space(ui::SPACE);
            let query = self.search.to_lowercase();
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-330.0).max(64.0)).show(ui, |ui| {
                let mut group = "";
                let mut found = false;
                for def in grouped_definitions() {
                    let chord = bindings.binding(def.id);
                    if !format!("{} {} {}", def.group, def.label, chord).to_lowercase().contains(&query) { continue; }
                    found = true;
                    if group != def.group { group = def.group; group_label(ui, group); }
                    reference_row(ui, def.label, if chord.is_empty() { "Disabled" } else { &chord });
                }
                group_label(ui, "Application and system shortcuts");
                for (group, label, chord) in FIXED {
                    if !format!("{group} {label} {chord}").to_lowercase().contains(&query) { continue; }
                    found = true;
                    reference_row(ui, &format!("{group} · {label}"), chord);
                }
                if !found { ui.label("No shortcuts match. Try an action name or clear the search."); }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui::button(ui, theme, "Configure shortcuts", None, false).clicked() { configure = true; close = true; }
                if ui::button(ui, theme, "Close", Some("ESC"), false).clicked() { close = true; }
            });
            });
        });
        if response.should_close() || close { self.open = false; self.focused = false; }
        configure
    }
}

// These belong to their focused application or the secure system path, not the
// remappable shell registry. Keep that distinction visible in the reference.
const FIXED: &[(&str, &str, &str)] = &[
    ("Terminal · fixed", "Copy selection / paste", "Ctrl+Shift+C / Ctrl+Shift+V"),
    ("Terminal · fixed", "Interrupt / end input", "Ctrl+C / Ctrl+D"),
    ("Files · fixed", "Copy / cut / paste", "Ctrl+C / Ctrl+X / Ctrl+V"),
    ("Files · fixed", "New folder / rename / Trash", "Ctrl+Shift+N / F2 / Delete"),
    ("Files · fixed", "Select entry / open selection", "Up or Down / Enter"),
    ("Switcher · fixed", "Choose window / accept / cancel", "Tab / Enter / Escape"),
    ("Overview · fixed", "Open overview", "Tap Super"),
    ("Overview · fixed", "Choose window / close", "Arrows / Enter / Escape"),
    ("Switcher · fixed", "Next / previous application", "Alt+Tab / Alt+Shift+Tab"),
    ("Switcher · fixed", "Windows of the same application", "Alt+Backtick / Alt+Shift+Backtick"),
    ("Dock · fixed", "Navigate / activate / close", "Arrows / Enter / Escape"),
    ("Window chooser · fixed", "Previous / next page", "Page Up / Page Down"),
    ("Launcher · fixed", "Navigate results / open / close", "Arrows / Enter / Escape"),
    ("File panel · fixed", "Select / preview / insert path", "Arrows / Space / Enter"),
    ("Find · fixed", "Next / previous result / close", "Enter / Shift+Enter / Escape"),
    ("Terminal · fixed", "Open a link", "Ctrl+click"),
    ("Capture · fixed", "Screenshot chooser", "Print"),
    ("Capture · fixed", "Window / region screenshot", "Alt+Print / Shift+Print"),
    ("Controls · fixed", "Move keyboard focus", "Tab / Shift+Tab"),
    ("Controls · fixed", "Adjust focused slider", "Arrows / Shift+Arrows / Home / End"),
    ("Security · fixed", "Switch virtual terminal", "Ctrl+Alt+F1…F12"),
    ("Security · fixed", "End desktop session", "Ctrl+Alt+Backspace"),
    ("Hardware · fixed", "Volume, brightness and media", "Dedicated hardware keys"),
    ("Security · fixed", "Lock desktop", "Super+L"),
    ("Security · fixed", "Leave secure dialogs", "Escape (where available)"),
];
fn grouped_definitions() -> impl Iterator<Item = &'static des_common::shortcuts::Definition> {
    ["Desktop", "Windows", "Workspaces", "Panels", "Shell", "Terminal"].into_iter()
        .flat_map(|group| DEFINITIONS.iter().filter(move |definition| definition.group == group))
}
fn group_label(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).font(ui::display_bold(13.0)).color(ui::SECONDARY));
    ui.separator();
}
fn reference_row(ui: &mut egui::Ui, label: &str, chord: &str) {
    if ui.available_width() > 520.0 { ui::key_row(ui, label, chord); return; }
    ui.vertical(|ui| {
        ui.add(egui::Label::new(RichText::new(label).font(ui::display(13.0))).wrap());
        ui.add(egui::Label::new(RichText::new(chord).font(ui::mono(12.0)).color(ui::SECONDARY)).wrap());
    });
    ui.add_space(4.0);
}

#[derive(Default)]
pub struct Editor { reset_all_armed: bool, recording: bool, focus_draft: bool, capture: Option<&'static str>, error: Option<String>, search: String, draft: String }
impl Editor {
    pub fn capturing(&self) -> bool { self.capture.is_some() || self.reset_all_armed }
    pub fn cancel(&mut self) { self.reset_all_armed = false; self.capture = None; self.error = None; self.recording = false; self.focus_draft = false; }
    pub fn show(&mut self, ui: &mut egui::Ui, theme: &Theme, saved: &Bindings) -> Option<Bindings> {
        let mut changed = None;
        if self.reset_all_armed && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) { self.cancel(); }
        // Process before drawing controls: Enter used to activate Change cannot
        // become the new shortcut in that same frame.
        if let Some(id) = self.capture {
            let events = ui.input(|input| input.events.clone());
            for event in events {
                if let egui::Event::Key { key, modifiers, pressed: true, repeat: false, .. } = event {
                    if key == Key::Escape {
                        ui.input_mut(|input| { input.consume_key(modifiers, key); });
                        self.cancel(); break;
                    }
                    if !self.recording { continue; }
                    ui.input_mut(|input| { input.consume_key(modifiers, key); });
                    if let Some(chord) = chord_for_event(key, modifiers) {
                        let mut next = saved.clone();
                        match next.set(id, &chord.to_string()) {
                            Ok(()) => { changed = Some(next); self.cancel(); }
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    } else { self.error = Some("Use a supported Ctrl combination. Enter desktop Super shortcuts in the text field.".into()); }
                    break;
                }
            }
        }
        ui.label(RichText::new("Keyboard shortcuts").font(ui::display_bold(20.0)));
        ui.label("Choose Change to enter a shortcut. Changes save immediately after Apply.");
        ui.add_space(ui::SPACE);
        ui.horizontal(|ui| {
            let search = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Find an action").desired_width((ui.available_width() - 64.0).max(1.0)));
            if ui.add_enabled(!self.search.is_empty(), egui::Button::new("Clear")).clicked() { self.search.clear(); search.request_focus(); }
        });
        if let Some(id) = self.capture {
            let label = DEFINITIONS.iter().find(|def| def.id == id).map_or(id, |def| def.label);
            ui.label(RichText::new(format!("New shortcut for {label}")).color(theme.main));
            let edit = ui.add_enabled(!self.recording, egui::TextEdit::singleline(&mut self.draft).hint_text("Super+Space").desired_width(ui.available_width()));
            if self.focus_draft { edit.request_focus(); self.focus_draft = false; }
            ui.label("Enter Super+… for desktop shortcuts. Escape cancels.");
            if self.recording {
                ui.label(RichText::new("Recording: press a shortcut to apply it immediately.").color(theme.main));
                if ui.button("Stop recording").clicked() { self.recording = false; self.focus_draft = true; }
            } else if ui.add_enabled(!id.starts_with("desktop."), egui::Button::new("Record shortcut")).on_hover_text("Record Ctrl combinations for shell and terminal actions; enter desktop Super combinations above").clicked() {
                self.recording = true;
                self.error = None;
                edit.surrender_focus();
            }
            if ui::button(ui, theme, "Apply shortcut", None, false).clicked() {
                let mut next = saved.clone();
                match next.set(id, self.draft.trim()) {
                    Ok(()) => { changed = Some(next); self.cancel(); }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            if ui::button(ui, theme, "Cancel change", Some("ESC"), false).clicked() { self.cancel(); }
        }
        if let Some(error) = &self.error { ui.colored_label(ui::CAUTION, error); }
        let query = self.search.to_lowercase();
        let mut group = "";
        let mut found = false;
        for def in grouped_definitions() {
            if !format!("{} {}", def.label, def.group).to_lowercase().contains(&query) { continue; }
            found = true;
            if group != def.group { group = def.group; group_label(ui, group); }
            ui.push_id(def.id, |ui| {
                ui.label(RichText::new(def.label).font(ui::display(14.0)));
                let binding = saved.binding(def.id);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(if binding.is_empty() { "Disabled" } else { &binding }).font(ui::mono(12.0)).color(ui::SECONDARY));
                    if ui.button(if self.capture == Some(def.id) { "Editing…" } else { "Change" }).clicked() { self.capture = Some(def.id); self.recording = false; self.focus_draft = true; self.error = None; self.draft = binding.clone(); }
                    if ui.add_enabled(!binding.is_empty(), egui::Button::new("Disable")).clicked() {
                        let mut next = saved.clone();
                        match next.set(def.id, "") { Ok(()) => { changed = Some(next); self.cancel(); }, Err(e) => self.error = Some(e.to_string()) }
                    }
                    if ui.button("Reset").on_hover_text(format!("Default: {}", def.default)).clicked() {
                        let mut next = saved.clone();
                        match next.reset(def.id) { Ok(()) => { changed = Some(next); self.cancel(); }, Err(e) => self.error = Some(e.to_string()) }
                    }
                });
                ui.add_space(6.0);
            });
        }
        if !found { ui.label("No actions match. Clear the search to see every shortcut."); }
        ui.separator();
        if self.reset_all_armed {
            ui.label("Replace all custom shortcuts with their defaults?");
            ui.horizontal_wrapped(|ui| {
                if ui::button(ui, theme, "Reset all mappings", None, false).clicked() { let mut next = saved.clone(); next.reset_all(); changed = Some(next); self.cancel(); }
                if ui::button(ui, theme, "Keep mappings", Some("ESC"), false).clicked() { self.cancel(); }
            });
        } else if ui::button(ui, theme, "Reset all shortcuts", None, false).clicked() { self.cancel(); self.reset_all_armed = true; }
        ui.label(RichText::new("Application editing keys, hardware controls and secure system shortcuts keep their standard behavior. Open the shortcut reference for these keys.").font(ui::display(12.0)).color(ui::SECONDARY));
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(editor: &mut Editor, ctx: &egui::Context, saved: &Bindings, key: Option<Key>) -> Option<Bindings> {
        let mut result = None;
        let events = key.into_iter().map(|key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::CTRL | Modifiers::SHIFT }).collect();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 800.0))), events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| result = editor.show(ui, &Theme::load("signal"), saved));
        });
        result
    }
    #[test]
    fn manual_edit_does_not_record_or_consume_editing_keys() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let saved = Bindings::default();
        let mut editor = Editor { capture: Some("shell.help"), draft: "Super+Space".into(), ..Default::default() };
        for (key, modifiers) in [(Key::Backspace, Modifiers::NONE), (Key::ArrowLeft, Modifiers::NONE), (Key::V, Modifiers::CTRL)] {
            let _ = ctx.run(egui::RawInput {
                events: vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }],
                ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(editor.show(ui, &Theme::load("signal"), &saved).is_none());
                    assert!(ui.input_mut(|input| input.consume_key(modifiers, key)), "manual editing key was consumed as a shortcut");
                });
            });
        }
        assert!(editor.capturing());
        assert!(editor.error.is_none());
    }
    #[test]
    fn cancelling_bulk_reset_preserves_custom_bindings_and_owns_escape() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let mut saved = Bindings::default(); saved.set("shell.help", "Ctrl+Shift+F12").unwrap();
        let mut editor = Editor { reset_all_armed: true, ..Default::default() };
        assert!(editor.capturing());
        let _ = ctx.run(egui::RawInput { events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(editor.show(ui, &Theme::load("signal"), &saved).is_none());
                assert!(!ui.input(|input| input.key_pressed(Key::Escape)));
            });
        });
        assert!(!editor.capturing());
        assert_eq!(saved.binding("shell.help"), "Ctrl+Shift+F12");
    }

    #[test]
    fn recording_conflict_preserves_saved_mapping_and_stays_open() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let saved = Bindings::default();
        let mut editor = Editor { capture: Some("shell.help"), recording: true, ..Default::default() };
        assert!(frame(&mut editor, &ctx, &saved, Some(Key::T)).is_none());
        assert!(editor.capturing());
        assert!(editor.error.is_some());
        assert_eq!(saved.binding("shell.help"), "Ctrl+Shift+H");
    }
    #[test]
    fn recorded_mapping_emits_once_and_does_not_mutate_saved() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let saved = Bindings::default();
        let mut editor = Editor { capture: Some("shell.help"), recording: true, ..Default::default() };
        let next = frame(&mut editor, &ctx, &saved, Some(Key::F12)).expect("new mapping");
        assert_eq!(next.binding("shell.help"), "Ctrl+Shift+F12");
        assert_eq!(saved.binding("shell.help"), "Ctrl+Shift+H");
        assert!(!editor.capturing());
        assert!(frame(&mut editor, &ctx, &saved, None).is_none());
    }
    #[test]
    fn shifted_punctuation_and_navigation_keys_normalize_like_compositor_chords() {
        let modifiers = Modifiers::CTRL | Modifiers::SHIFT;
        for (key, expected) in [(Key::Questionmark, "Ctrl+Shift+Slash"), (Key::OpenCurlyBracket, "Ctrl+Shift+BracketLeft"), (Key::Plus, "Ctrl+Shift+Equal"), (Key::Exclamationmark, "Ctrl+Shift+1"), (Key::Home, "Ctrl+Shift+Home"), (Key::PageDown, "Ctrl+Shift+PageDown")] {
            assert_eq!(chord_for_event(key, modifiers).unwrap(), Chord::parse(expected).unwrap());
        }
    }

    #[test]
    fn arrow_events_use_registry_names_and_live_bindings() {
        let ctx = egui::Context::default();
        let saved = Bindings::default(); publish(&ctx, &saved);
        assert!(matches_event(&ctx, "terminal.prompt_previous", Key::ArrowUp, Modifiers::CTRL | Modifiers::SHIFT));
    }
    #[test]
    fn reference_escape_never_reaches_the_launcher_underneath() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal"); let mut reference = Reference { open: true, ..Default::default() };
        let mut launcher = crate::launcher::Launcher::default(); launcher.open = true;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 640.0));
        for pressed in [false, true] {
            let events = if pressed { vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }] } else { Vec::new() };
            let _ = ctx.run(egui::RawInput { screen_rect: Some(rect), events, ..Default::default() }, |ctx| {
                let held = take_modal_input(ctx, reference.open);
                egui::CentralPanel::default().show(ctx, |ui| { launcher.show(ui, rect, &theme); });
                ctx.input_mut(|input| input.events.extend(held));
                reference.show(ctx, &theme, &Bindings::default());
            });
        }
        assert!(launcher.open, "reference Escape changed the underlying workspace");
        assert!(!reference.open, "reference did not receive Escape");
    }

    #[test]
    fn reference_is_bounded_on_narrow_screen() {
        for width in [320.0, 360.0, 900.0] {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        crate::apply_style(&ctx, &Theme::load("signal"));
        let mut reference = Reference { open: true, ..Default::default() };
        for _ in 0..3 {
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 640.0))), ..Default::default() }, |ctx| {
            assert!(!reference.show(ctx, &Theme::load("signal"), &Bindings::default()));
        });
        }
        assert!(reference.open);
        let rect = ctx.memory(|memory| memory.area_rect(egui::Id::new("shortcut-reference"))).expect("modal area");
        assert!(rect.is_finite() && rect.left() >= -1.0 && rect.top() >= -1.0 && rect.right() <= width + 1.0 && rect.bottom() <= 641.0, "reference exceeds {width}×640: {rect:?}");
        }
    }
}
