//! Appearance draft and local image chooser; nothing is applied until requested.
use eframe::egui;
use des_common::appearance::{AppearanceSettings, SurfaceStyle, WallpaperFit};
use crate::{ui, wallpaper::Loader};
use std::path::PathBuf;

#[derive(Default)]
pub struct Editor {
    basis: Option<AppearanceSettings>,
    draft: AppearanceSettings,
    path: String,
    loader: Loader,
    preview_key: Option<(PathBuf, u32)>,
    imported_generation: Option<PathBuf>,
    picker: Option<crate::files::Browser>,
    picker_focus: Option<egui::Id>,
}
impl Editor {
    pub fn reset(&mut self, saved: &AppearanceSettings) {
        self.basis = Some(saved.clone()); self.draft = saved.clone();
        self.path = saved.wallpaper.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        self.loader.clear(); self.preview_key = None; self.imported_generation = None;
        self.picker = None; self.picker_focus = None;
    }
    pub fn cached_view(&self, settings: &AppearanceSettings) -> Option<crate::wallpaper::WallpaperView> {
        if self.loader.loading || self.loader.error.is_some() || self.preview_key.as_ref() != settings.wallpaper.as_ref().map(|p| (p.clone(), settings.blur.to_bits())).as_ref() { return None; }
        self.loader.view.clone()
    }
    pub fn saved(&mut self, settings: &AppearanceSettings) {
        if let Some(asset) = &self.loader.asset { asset.keep(); }
        self.basis = Some(settings.clone()); self.draft = settings.clone();
    }
    fn finish_import(&mut self, path: PathBuf, rendered_blur: f32) {
        // Choosing a backdrop must make it visible throughout the workspace.
        if self.draft.style == SurfaceStyle::Solid {
            self.draft.style = SurfaceStyle::Frosted;
            self.draft.opacity = AppearanceSettings::default().opacity;
        }
        self.draft.wallpaper = Some(path.clone());
        self.imported_generation = Some(path.clone());
        self.preview_key = Some((path, rendered_blur.to_bits()));
    }
    fn choose(&mut self, ctx: &egui::Context, path: PathBuf) {
        if !path.is_absolute() { self.loader.error = Some("Enter an absolute image path, such as /home/you/Pictures/wallpaper.jpg".into()); return; }
        self.path = path.to_string_lossy().into_owned();
        self.imported_generation = None;
        self.preview_key = None;
        self.loader.request(ctx, &path, self.draft.blur, true);
    }
    pub fn picker_open(&self) -> bool { self.picker.is_some() }
    pub fn sync_visibility(&mut self, visible: bool) {
        if !visible { self.picker = None; self.picker_focus = None; }
    }
    fn picker(&mut self, ctx: &egui::Context) {
        if self.picker.is_none() { return; }
        let screen = ctx.screen_rect();
        let response = egui::Modal::new(egui::Id::new("wallpaper-file-picker")).show(ctx, |ui| {
            ui.set_width((screen.width() - 48.0).clamp(240.0, 780.0));
            ui.set_height((screen.height() - 64.0).clamp(240.0, 560.0));
            ui.label(egui::RichText::new("Choose wallpaper").font(ui::display_bold(20.0)));
            ui.add_space(12.0);
            self.picker.as_mut().unwrap().show(ui)
        });
        let close = response.should_close();
        match response.inner {
            Some(crate::files::Event::Choose(path)) => { self.picker = None; self.choose(ctx, path); },
            Some(crate::files::Event::Close) => self.picker = None,
            _ if close => self.picker = None,
            _ => {},
        }
        if self.picker.is_none() { if let Some(id) = self.picker_focus.take() { ctx.memory_mut(|memory| memory.request_focus(id)); } }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, saved: &AppearanceSettings) -> Option<AppearanceSettings> {
        if self.basis.as_ref() != Some(saved) { self.reset(saved); }
        self.loader.poll(ui.ctx());
        if let Some(asset) = &self.loader.asset {
            if !self.loader.loading && self.loader.error.is_none() && self.imported_generation.as_ref() != Some(&asset.path) {
                let path = asset.path.clone();
                self.finish_import(path, self.loader.requested_blur);
            }
        }
        ui.label(egui::RichText::new("Desktop appearance").font(ui::display_bold(20.0)).color(ui::TEXT));
        ui.add_space(8.0);
        let (preview, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 160.0), egui::Sense::hover());
        crate::material::preview(ui.painter(), preview, &self.draft, self.loader.view.as_ref());
        let panel = preview.shrink(16.0);
        ui.painter().text(panel.left_top() + egui::vec2(12.0, 12.0), egui::Align2::LEFT_TOP, "Terminal preview\n$ ready", ui::mono(13.0), ui::TEXT);
        ui.add_space(16.0);
        ui::tracked(ui, "WALLPAPER", ui::display_bold(14.0), ui::TEXT, 0.6);
        ui.add_space(8.0);
        crate::settings::note(ui, "Choose a picture and adjust its background.");
        ui.horizontal_wrapped(|ui| {
            let choose = ui.button("Choose image");
            if choose.clicked() {
                self.picker = Some(crate::files::Browser::new(crate::files::home().join("Pictures"), crate::files::Mode::ImagePicker));
                self.picker_focus = Some(choose.id);
            }
            if ui.add_enabled(self.draft.wallpaper.is_some() || self.loader.loading, egui::Button::new("Remove wallpaper")).clicked() {
                self.draft.wallpaper = None; self.path.clear(); self.loader.clear(); self.preview_key = None; self.imported_generation = None;
            }
        });
        egui::CollapsingHeader::new("Enter image path").show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.path).font(ui::mono(13.0)).hint_text("/home/you/Pictures/wallpaper.jpg").desired_width(ui.available_width()).id_salt("wallpaper-path"))
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Wallpaper image path"));
            if ui.button("Load image path").clicked() { self.choose(ui.ctx(), PathBuf::from(&self.path)); }
        });
        if self.loader.loading { ui.label("Preparing image…"); }
        if let Some(error) = &self.loader.error { ui.colored_label(ui::CAUTION, error); }
        ui.horizontal_wrapped(|ui| {
            ui.label("Placement");
            ui.selectable_value(&mut self.draft.fit, WallpaperFit::Fill, "Fill");
            ui.selectable_value(&mut self.draft.fit, WallpaperFit::Fit, "Fit");
        });
        let changed_blur = appearance_slider(ui, "Blur strength", &mut self.draft.blur, 0.0..=40.0, "");
        appearance_slider(ui, "Dim", &mut self.draft.dim, 0.0..=80.0, "%");
        ui.add_space(20.0);
        ui.separator();
        ui.add_space(12.0);
        ui::tracked(ui, "SURFACE STYLE", ui::display_bold(14.0), ui::TEXT, 0.6);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.draft.style, SurfaceStyle::Solid, "Opaque");
            ui.selectable_value(&mut self.draft.style, SurfaceStyle::Frosted, "Frosted");
        });
        if saved.style == SurfaceStyle::Liquid { ui.label("Using Frosted: live glass is unavailable in this build."); }
        appearance_slider(ui, "Surface opacity", &mut self.draft.opacity, 35.0..=100.0, "%");
        ui.checkbox(&mut self.draft.reduce_transparency, "Reduce transparency");
        ui.checkbox(&mut self.draft.reduce_motion, "Reduce motion");
        crate::settings::note(ui, "Wallpaper extends behind the terminal. Text stays sharp; controls retain a stronger tint for readability.");
        // Debounce blur edits; a busy worker retains at most one newer request for this editor.
        if !self.loader.loading && self.loader.error.is_none() {
            let key = self.draft.wallpaper.as_ref().map(|p| (p.clone(), self.draft.blur.to_bits()));
            if key != self.preview_key || changed_blur {
                self.preview_key = key;
                if let Some((path, _)) = &self.preview_key { self.loader.request(ui.ctx(), path, self.draft.blur, false); }
            }
        }
        let can_apply = !self.loader.loading && self.loader.error.is_none() && self.draft.validate().is_ok() && &self.draft != saved;
        let mut requested = None;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(can_apply, egui::Button::new("Apply appearance")).clicked() { requested = Some(self.draft.clone()); }
            if ui.button("Cancel changes").clicked() { self.reset(saved); }
            if ui.button("Reset appearance").clicked() {
                self.draft = AppearanceSettings::default(); self.path.clear(); self.loader.clear(); self.preview_key = None; self.imported_generation = None;
            }
        });
        crate::settings::note(ui, "Preview only. Apply saves these controls together; your theme and accent stay separate.");
        ui.add_space(24.0);
        self.picker(ui.ctx());
        requested
    }
}

/// Keep a native, named slider while giving the caption its own wrapping row.
fn appearance_slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) -> bool {
    ui.add(egui::Label::new(egui::RichText::new(label).font(ui::display(13.0)).color(ui::TEXT)).wrap());
    let response = ui.add(egui::Slider::new(value, range).suffix(suffix).integer());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), label));
    ui.add_space(4.0);
    response.changed()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(editor: &mut Editor, ctx: &egui::Context, saved: &AppearanceSettings, events: Vec<egui::Event>) -> (egui::FullOutput, Option<AppearanceSettings>) {
        let mut change = None;
        let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 1100.0))), events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { change = editor.show(ui, saved); });
        });
        (output, change)
    }
    fn button(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos + text.galley.size() / 2.0), _ => None,
        }).unwrap_or_else(|| panic!("missing {label}"))
    }
    fn click(editor: &mut Editor, ctx: &egui::Context, saved: &AppearanceSettings, pos: egui::Pos2) -> Option<AppearanceSettings> {
        let mut change = None;
        for pressed in [true, false] {
            change = render(editor, ctx, saved, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]).1.or(change);
        }
        change
    }
    #[test]
    fn appearance_editor_apply_cancel_and_reset_are_draft_only() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let saved = AppearanceSettings::default();
        let mut editor = Editor::default();
        editor.reset(&saved);
        editor.draft.style = SurfaceStyle::Frosted;
        editor.draft.dim = 50.0;
        let (output, change) = render(&mut editor, &ctx, &saved, Vec::new());
        assert!(change.is_none());
        let change = click(&mut editor, &ctx, &saved, button(&output, "Apply appearance")).unwrap();
        assert_eq!(change.style, SurfaceStyle::Frosted);
        assert_eq!(change.dim, 50.0);
        assert_eq!(editor.basis.as_ref(), Some(&saved), "request alone cannot mark an unsuccessful save as committed");
        let output = render(&mut editor, &ctx, &saved, Vec::new()).0;
        assert!(click(&mut editor, &ctx, &saved, button(&output, "Cancel changes")).is_none());
        assert_eq!(editor.draft, saved);
        editor.draft.style = SurfaceStyle::Frosted;
        let output = render(&mut editor, &ctx, &saved, Vec::new()).0;
        assert!(click(&mut editor, &ctx, &saved, button(&output, "Reset appearance")).is_none());
        assert_eq!(editor.draft, AppearanceSettings::default());
    }
    #[test]
    fn appearance_editor_keyboard_can_apply_and_cancel() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let saved = AppearanceSettings::default();
        let mut editor = Editor::default(); editor.reset(&saved);
        editor.draft.style = SurfaceStyle::Frosted;
        fn key(editor: &mut Editor, ctx: &egui::Context, saved: &AppearanceSettings, key: egui::Key) -> Option<AppearanceSettings> {
            let mut change = None;
            for pressed in [true, false] {
                change = render(editor, ctx, saved, vec![egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: egui::Modifiers::NONE }]).1.or(change);
            }
            change
        }
        fn focus(editor: &mut Editor, ctx: &egui::Context, saved: &AppearanceSettings, label: &str) {
            let output = render(editor, ctx, saved, Vec::new()).0;
            let pos = button(&output, label);
            for _ in 0..40 {
                if ctx.memory(|memory| memory.focused()).and_then(|id| ctx.read_response(id)).is_some_and(|response| response.rect.contains(pos)) { return; }
                assert!(key(editor, ctx, saved, egui::Key::Tab).is_none());
            }
            panic!("could not focus {label} with Tab");
        }
        focus(&mut editor, &ctx, &saved, "Apply appearance");
        assert_eq!(key(&mut editor, &ctx, &saved, egui::Key::Enter).unwrap().style, SurfaceStyle::Frosted);
        focus(&mut editor, &ctx, &saved, "Cancel changes");
        assert!(key(&mut editor, &ctx, &saved, egui::Key::Enter).is_none());
        assert_eq!(editor.draft, saved);
    }
    #[test]
    fn appearance_hidden_settings_dismiss_picker_and_keep_the_draft() {
        let mut editor = Editor::default(); editor.reset(&AppearanceSettings::default());
        editor.draft.dim = 51.0;
        editor.picker = Some(crate::files::Browser::new(std::env::temp_dir(), crate::files::Mode::ImagePicker));
        assert!(editor.picker_open());
        editor.sync_visibility(false);
        assert!(!editor.picker_open(), "hidden settings must not retain an input-blocking modal");
        assert_eq!(editor.draft.dim, 51.0);
    }
    #[test]
    fn appearance_native_picker_escape_cancels_without_applying() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let saved = AppearanceSettings::default();
        let mut editor = Editor::default(); editor.reset(&saved);
        editor.picker = Some(crate::files::Browser::new(std::env::temp_dir(), crate::files::Mode::ImagePicker));
        assert!(render(&mut editor, &ctx, &saved, Vec::new()).1.is_none());
        assert!(editor.picker_open());
        assert!(render(&mut editor, &ctx, &saved, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]).1.is_none());
        assert!(!editor.picker_open());
        assert_eq!(editor.draft, saved);
    }
    #[test]
    fn choosing_wallpaper_establishes_visible_frosted_surfaces() {
        let mut editor = Editor::default();
        editor.finish_import(PathBuf::from("/tmp/wallpaper.png"), 18.0);
        assert_eq!(editor.draft.style, SurfaceStyle::Frosted);
        assert!(editor.draft.opacity <= 65.0);
        assert!(editor.draft.validate().is_ok());
    }
    #[test]
    fn appearance_import_completion_keeps_the_rendered_blur_generation() {
        let mut editor = Editor::default();
        editor.draft.blur = 35.0;
        editor.finish_import(PathBuf::from("/tmp/import.png"), 18.0);
        assert_eq!(editor.preview_key, Some((PathBuf::from("/tmp/import.png"), 18.0f32.to_bits())));
        assert_ne!(editor.preview_key.as_ref().unwrap().1, editor.draft.blur.to_bits(), "a newer blur edit must trigger regeneration before Apply");
    }
    #[test]
    fn appearance_editor_invalid_path_never_replaces_saved_image() {
        let saved = AppearanceSettings { wallpaper: Some("/tmp/existing.png".into()), ..Default::default() };
        let mut editor = Editor::default();
        editor.reset(&saved);
        editor.choose(&egui::Context::default(), "relative.png".into());
        assert!(editor.loader.error.is_some());
        assert_eq!(editor.draft.wallpaper, saved.wallpaper);
    }
}
