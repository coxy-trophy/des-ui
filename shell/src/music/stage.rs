//! Music away from its library: the words full screen beside the cover, the
//! player in the desktop's side panel, and the card on the lock screen.

use super::*;
use super::chrome::{Glyph, glyph_button, paint_glyph, play_button};
use eframe::egui::{Color32, Pos2};

impl Music {
    /// Leaves the full-screen words for wherever they were opened from.
    pub(super) fn leave_lyrics(&mut self) {
        self.source = self.trail.pop().unwrap_or(Source::Home);
    }

    /// The words, large, beside the cover, the position and the controls.
    pub(super) fn lyrics_stage(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, rect: Rect) {
        let Some(track) = now.track.clone() else { return; };
        // A soft light of the accent behind the cover.
        glow(ui.painter(), pos2(rect.left() + rect.width() * 0.2, rect.center().y), rect.height() * 0.75, theme.main.gamma_multiply(0.16));
        let mut top = ui.new_child(UiBuilder::new().id_salt("stage-top").max_rect(Rect::from_min_size(rect.min, vec2(rect.width(), 34.0))).layout(Layout::right_to_left(Align::Center)));
        top.spacing_mut().item_spacing.x = 12.0;
        if ui::button(&mut top, theme, "CLOSE LYRICS", None, false).clicked() { self.leave_lyrics(); return; }
        ui::text(&mut top, "Click a line to jump to it · Esc returns", ui::mono(11.0), ui::LABEL);

        let column = (rect.width() * 0.34).clamp(280.0, 400.0);
        let cover = column.min(rect.height() - 220.0).max(120.0);
        let block = cover + 200.0;
        let left = Rect::from_min_size(pos2(rect.left() + 24.0, rect.center().y - block / 2.0), vec2(column, block));
        let mut a = ui.new_child(UiBuilder::new().id_salt("stage-now").max_rect(left).layout(Layout::top_down(Align::Min)));
        a.spacing_mut().item_spacing.y = 6.0;
        let texture = self.covers.get(a.ctx(), track.cover.as_deref());
        let (art, _) = a.allocate_exact_size(vec2(cover, cover), Sense::hover());
        match texture {
            Some(texture) => { a.painter().image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
            None => {
                a.painter().rect_filled(art, 12.0, ui::TRACK);
                paint_glyph(a.painter(), art.shrink(cover * 0.36), Glyph::Lyrics, ui::LABEL, false);
            }
        }
        a.add_space(14.0);
        a.add(egui::Label::new(egui::RichText::new(&track.name).font(ui::display_bold(28.0)).color(ui::TEXT)).truncate()).on_hover_text(&track.name);
        let by = if track.album.is_empty() { track.artists.clone() } else { format!("{} · {}", track.artists, track.album) };
        a.add(egui::Label::new(egui::RichText::new(&by).font(ui::display(14.5)).color(ui::SECONDARY)).truncate());
        a.add_space(12.0);
        if let Some(to) = ui::slider(&mut a, theme, cover, now.position_ms() as f32 / track.duration_ms.max(1) as f32, "Playback position") {
            if a.input(|i| i.pointer.any_released()) || !a.input(|i| i.pointer.any_down()) { self.send(json!({ "cmd": "seek", "ms": (to * track.duration_ms as f32) as u32 })); }
        }
        let (times, _) = a.allocate_exact_size(vec2(cover, 14.0), Sense::hover());
        a.painter().text(times.left_center(), Align2::LEFT_CENTER, clock(now.position_ms()), ui::mono(11.0), ui::SECONDARY);
        a.painter().text(times.right_center(), Align2::RIGHT_CENTER, clock(track.duration_ms), ui::mono(11.0), ui::SECONDARY);
        a.add_space(8.0);
        a.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            self.transport(ui, theme, now, 38.0, 54.0);
        });

        let words = Rect::from_min_max(pos2(left.right() + 56.0, rect.top() + 44.0), pos2(rect.right() - 24.0, rect.bottom()));
        let mut b = ui.new_child(UiBuilder::new().id_salt("stage-words").max_rect(words).layout(Layout::top_down(Align::Min)));
        b.set_clip_rect(words.intersect(ui.clip_rect()));
        self.lyrics(&mut b, theme, now);
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
    }

    /// Shuffle, previous, play or pause, next and repeat, in a row.
    fn transport(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, size: f32, play: f32) {
        if glyph_button(ui, theme, Glyph::Shuffle, size, now.shuffle, if now.shuffle { "Shuffle is on" } else { "Shuffle" }).clicked() { self.send(json!({ "cmd": "shuffle", "on": !now.shuffle })); }
        if control(ui, theme, Control::Previous, size, false).clicked() { self.send(json!({ "cmd": "prev" })); }
        if play_button(ui, theme, now.playing, play).clicked() { self.send(json!({ "cmd": "toggle" })); }
        if control(ui, theme, Control::Next, size, false).clicked() { self.send(json!({ "cmd": "next" })); }
        let (glyph, label, next) = match now.repeat.as_str() { "all" => (Glyph::Repeat, "Repeating all", "one"), "one" => (Glyph::RepeatOne, "Repeating this track", "off"), _ => (Glyph::Repeat, "Repeat", "all") };
        if glyph_button(ui, theme, glyph, size, now.repeat != "off", label).clicked() { self.send(json!({ "cmd": "repeat", "mode": next })); }
    }

    /// The player in the desktop's side panel. True when it asks for Music.
    pub fn panel(&mut self, ui: &mut Ui, theme: &Theme) -> bool {
        let now = self.now();
        let Some(track) = now.track.clone() else { return false; };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PANEL_HEIGHT), Sense::hover());
        let mut inner = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
        inner.set_clip_rect(rect.intersect(ui.clip_rect()));
        let ui = &mut inner;
        let width = rect.width();
        // Fixed rows, so the panel never grows past what the side panel keeps for it.
        let mut y = rect.top();
        let mut row = |height: f32, gap: f32| { let r = Rect::from_min_size(pos2(rect.left(), y), vec2(width, height)); y += height + gap; r };
        let (head, cover_row, controls_row, position_row, volume_row) = (row(14.0, 6.0), row(38.0, 6.0), row(30.0, 4.0), row(16.0, 6.0), row(16.0, 0.0));
        let line = |ui: &mut Ui, r: Rect, salt: &str| {
            let mut child = ui.new_child(UiBuilder::new().id_salt(("panel", salt)).max_rect(r).layout(Layout::left_to_right(Align::Center)));
            child.set_clip_rect(r.expand(4.0).intersect(ui.clip_rect()));
            child
        };
        let mut open = false;
        // Where it plays, and the way into Music.
        let opener = ui.interact(Rect::from_min_max(pos2(head.right() - 52.0, head.top() - 4.0), pos2(head.right(), head.bottom() + 4.0)), ui.id().with("open-music"), Sense::click());
        opener.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open Music"));
        open |= opener.clicked();
        let state = match (&now.device, now.playing) {
            (Some(device), true) => format!("PLAYING · {}", device.to_uppercase()),
            (None, true) => "PLAYING".to_string(),
            (_, false) => "PAUSED".to_string(),
        };
        let mut job = egui::text::LayoutJob::simple_singleline(state, ui::mono(10.0), if now.playing { theme.main } else { ui::LABEL });
        job.wrap = egui::text::TextWrapping::truncate_at_width((width - 62.0).max(20.0));
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(head.left(), head.center().y - galley.size().y / 2.0), galley, ui::LABEL);
        ui.painter().text(head.right_center(), Align2::RIGHT_CENTER, "OPEN ↗", ui::mono(10.5), if opener.hovered() { ui::TEXT } else { ui::SECONDARY });
        if opener.has_focus() { ui.painter().rect_stroke(opener.rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
        // The track, and the heart that keeps it.
        let mut a = line(ui, cover_row, "track");
        a.spacing_mut().item_spacing.x = 8.0;
        self.draw_cover(&mut a, 38.0, track.cover.as_deref());
        let room = (a.available_width() - 36.0).max(20.0);
        a.allocate_ui_with_layout(vec2(room, 38.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(2.0);
            let title = ui.add(egui::Label::new(egui::RichText::new(&track.name).font(ui::display_bold(13.0)).color(ui::TEXT)).truncate().sense(Sense::click()));
            open |= title.on_hover_text(format!("Open Music — {}", track.name)).clicked();
            ui.add(egui::Label::new(egui::RichText::new(&track.artists).font(ui::mono(10.5)).color(ui::SECONDARY)).truncate()).on_hover_text(&track.artists);
        });
        let liked = now.liked == Some(true);
        if glyph_button(&mut a, theme, Glyph::Heart, 28.0, liked, if liked { "Remove from Liked songs" } else { "Save to Liked songs" }).clicked() { self.send(json!({ "cmd": "like", "on": !liked })); }
        // The controls, spread across.
        let mut b = line(ui, controls_row, "controls");
        let (size, play) = (28.0, 30.0);
        let gap = ((width - size * 4.0 - play) / 4.0).clamp(2.0, 16.0);
        b.spacing_mut().item_spacing.x = gap;
        b.add_space(((width - size * 4.0 - play - gap * 4.0) / 2.0).max(0.0));
        self.transport(&mut b, theme, &now, size, play);
        // Where it is in the track.
        let mut c = line(ui, position_row, "position");
        c.spacing_mut().item_spacing.x = 6.0;
        ui::text(&mut c, &clock(now.position_ms()), ui::mono(10.0), ui::SECONDARY);
        let slide = (c.available_width() - 36.0).max(12.0);
        if let Some(to) = ui::slider_sized(&mut c, theme, slide, now.position_ms() as f32 / track.duration_ms.max(1) as f32, "Playback position", 16.0) {
            if c.input(|i| i.pointer.any_released()) || !c.input(|i| i.pointer.any_down()) { self.send(json!({ "cmd": "seek", "ms": (to * track.duration_ms as f32) as u32 })); }
        }
        ui::text(&mut c, &clock(track.duration_ms), ui::mono(10.0), ui::SECONDARY);
        // How loud Music plays, apart from the desktop's own volume.
        let mut d = line(ui, volume_row, "volume");
        d.spacing_mut().item_spacing.x = 6.0;
        let (speaker, _) = d.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        paint_glyph(d.painter(), speaker, Glyph::Speaker, ui::SECONDARY, false);
        let slide = (d.available_width() - 30.0).max(12.0);
        if let Some(to) = ui::slider_sized(&mut d, theme, slide, now.volume as f32 / 100.0, "Music volume", 16.0) { self.send(json!({ "cmd": "volume", "percent": (to * 100.0).round() as u8 })); }
        ui::text(&mut d, &now.volume.to_string(), ui::mono(10.0), ui::SECONDARY);
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
        open
    }

    /// What is playing, for the lock screen: the cover, the track, the three
    /// controls and how far along it is. Nothing when nothing is. It sits at the
    /// bottom of `within`, in the middle.
    pub fn lock_card(&mut self, ui: &mut Ui, theme: &Theme, within: Rect) {
        let now = self.now();
        let Some(track) = now.track.clone() else { return; };
        let size = vec2(440.0_f32.min(within.width()), 92.0);
        let mut card = Rect::from_center_size(pos2(within.center().x, within.bottom() - 40.0 - size.y / 2.0), size);
        // On a short screen the middle is taken by the password line: the corner then.
        if card.top() < within.center().y + 236.0 {
            card = Rect::from_min_size(within.right_bottom() - size, size);
        }
        let radius = 14.0;
        crate::material::paint(ui.ctx(), ui.painter(), card, radius, crate::material::Role::Panel);
        ui.painter().rect_stroke(card, radius, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        // How far along, as a thin line along the bottom edge.
        let frac = (now.position_ms() as f32 / track.duration_ms.max(1) as f32).clamp(0.0, 1.0);
        let line = Rect::from_min_size(pos2(card.left() + 18.0, card.bottom() - 6.0), vec2(card.width() - 36.0, 2.0));
        ui.painter().rect_filled(line, 1.0, ui::TRACK);
        ui.painter().rect_filled(Rect::from_min_size(line.min, vec2(line.width() * frac, 2.0)), 1.0, ui::TEXT);

        let mut inner = ui.new_child(UiBuilder::new().max_rect(card.shrink2(vec2(16.0, 12.0)).translate(vec2(0.0, -2.0))).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut inner;
        ui.spacing_mut().item_spacing.x = 14.0;
        self.draw_cover(ui, 60.0, track.cover.as_deref());
        let controls = 30.0 * 2.0 + 40.0 + 8.0 * 2.0;
        let room = (ui.available_width() - controls - 14.0).max(40.0);
        ui.allocate_ui_with_layout(vec2(room, 60.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let state = match (&now.device, now.playing) {
                (Some(device), true) => format!("PLAYING ON {}", device.to_uppercase()),
                (None, true) => "NOW PLAYING".to_string(),
                (_, false) => "PAUSED".to_string(),
            };
            ui.add(egui::Label::new(egui::RichText::new(state).font(ui::mono(10.0)).color(if now.playing { theme.main } else { ui::LABEL })).truncate());
            ui.add(egui::Label::new(egui::RichText::new(&track.name).font(ui::display_bold(15.5)).color(ui::TEXT)).truncate());
            ui.add(egui::Label::new(egui::RichText::new(&track.artists).font(ui::mono(11.5)).color(ui::SECONDARY)).truncate());
        });
        ui.spacing_mut().item_spacing.x = 8.0;
        if control(ui, theme, Control::Previous, 30.0, false).clicked() { self.send(json!({ "cmd": "prev" })); }
        if play_button(ui, theme, now.playing, 40.0).clicked() { self.send(json!({ "cmd": "toggle" })); }
        if control(ui, theme, Control::Next, 30.0, false).clicked() { self.send(json!({ "cmd": "next" })); }
        // The position keeps moving while it plays.
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
    }
}

/// A round light that fades from `color` at the middle to nothing at `radius`.
fn glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    let rings = [(0.35, 0.7), (0.65, 0.3), (1.0, 0.0)];
    let steps = 64;
    for (ring, &(reach, strength)) in rings.iter().enumerate() {
        for step in 0..steps {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * radius * reach, color.gamma_multiply(strength));
        }
        let base = 1 + ring as u32 * steps;
        for step in 0..steps {
            let (a, b) = (base + step, base + (step + 1) % steps);
            if ring == 0 { mesh.add_triangle(0, a, b); } else { let (pa, pb) = (a - steps, b - steps); mesh.add_triangle(pa, a, b); mesh.add_triangle(pa, b, pb); }
        }
    }
    painter.add(Shape::mesh(mesh));
}
