//! The music player's frame: the library sidebar, the home view, the column of
//! what plays next, its lyrics and devices, and the player bar along the bottom.

use super::*;
use eframe::egui::{Color32, Pos2};

/// What the column beside the library shows.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum SideTab { #[default] Queue, Lyrics, Devices }

/// The small line drawings the player uses beside the transport triangles.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Glyph { Shuffle, Repeat, RepeatOne, Heart, Lyrics, Queue, Speaker, Plus }

/// A round button drawn with a glyph; `lit` draws it in the accent.
pub(super) fn glyph_button(ui: &mut Ui, theme: &Theme, glyph: Glyph, size: f32, lit: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), lit, label));
    let response = response.on_hover_text(label);
    let hot = response.hovered() || response.has_focus();
    if hot { ui.painter().circle_filled(rect.center(), size / 2.0, ui::TRACK); }
    if response.has_focus() { ui.painter().circle_stroke(rect.center(), size / 2.0, Stroke::new(1.0, theme.main)); }
    let ink = if lit { theme.main } else if hot { ui::TEXT } else { ui::SECONDARY };
    paint_glyph(ui.painter(), rect.shrink(size * 0.24), glyph, ink, lit);
    response
}

pub(super) fn paint_glyph(painter: &egui::Painter, r: Rect, glyph: Glyph, ink: Color32, filled: bool) {
    let s = Stroke::new(1.6, ink);
    let p = |x: f32, y: f32| pos2(r.left() + r.width() * x, r.top() + r.height() * y);
    let line = |points: Vec<Pos2>| { painter.add(Shape::line(points, s)); };
    match glyph {
        Glyph::Shuffle => {
            line(vec![p(0.0, 0.25), p(0.25, 0.25), p(0.7, 0.75), p(1.0, 0.75)]);
            line(vec![p(0.0, 0.75), p(0.25, 0.75), p(0.7, 0.25), p(1.0, 0.25)]);
            line(vec![p(0.85, 0.1), p(1.0, 0.25), p(0.85, 0.4)]);
            line(vec![p(0.85, 0.6), p(1.0, 0.75), p(0.85, 0.9)]);
        }
        Glyph::Repeat | Glyph::RepeatOne => {
            line(vec![p(0.05, 0.55), p(0.05, 0.25), p(0.9, 0.25)]);
            line(vec![p(0.75, 0.1), p(0.9, 0.25), p(0.75, 0.4)]);
            line(vec![p(0.95, 0.45), p(0.95, 0.75), p(0.1, 0.75)]);
            line(vec![p(0.25, 0.6), p(0.1, 0.75), p(0.25, 0.9)]);
            if glyph == Glyph::RepeatOne {
                painter.text(p(0.5, 0.5), Align2::CENTER_CENTER, "1", ui::mono(r.height() * 0.42), ink);
            }
        }
        Glyph::Heart => {
            let points: Vec<Pos2> = (0..=40).map(|i| {
                let t = i as f32 / 40.0 * std::f32::consts::TAU;
                let x = 16.0 * t.sin().powi(3);
                let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                pos2(r.center().x + x / 34.0 * r.width(), r.center().y - (y + 2.0) / 34.0 * r.height())
            }).collect();
            if filled { painter.add(Shape::convex_polygon(points.clone(), ink, Stroke::NONE)); }
            painter.add(Shape::closed_line(points, s));
        }
        Glyph::Lyrics => {
            line(vec![p(0.0, 0.2), p(1.0, 0.2)]);
            line(vec![p(0.0, 0.5), p(0.7, 0.5)]);
            line(vec![p(0.0, 0.8), p(0.45, 0.8)]);
        }
        Glyph::Queue => {
            line(vec![p(0.0, 0.15), p(0.75, 0.15)]);
            line(vec![p(0.0, 0.45), p(0.75, 0.45)]);
            line(vec![p(0.0, 0.75), p(0.45, 0.75)]);
            line(vec![p(0.82, 0.55), p(0.82, 1.0)]);
            line(vec![p(0.6, 0.78), p(1.04, 0.78)]);
        }
        Glyph::Speaker => {
            painter.add(Shape::convex_polygon(vec![p(0.0, 0.35), p(0.25, 0.35), p(0.55, 0.1), p(0.55, 0.9), p(0.25, 0.65), p(0.0, 0.65)], Color32::TRANSPARENT, s));
            line(vec![p(0.72, 0.3), p(0.82, 0.5), p(0.72, 0.7)]);
        }
        Glyph::Plus => { line(vec![p(0.5, 0.05), p(0.5, 0.95)]); line(vec![p(0.05, 0.5), p(0.95, 0.5)]); }
    }
}

/// A row in the sidebar: a name, a picture when it has one, and its place lit when open.
fn nav_row(ui: &mut Ui, theme: &Theme, name: &str, chosen: bool, cover: Option<Option<&TextureHandle>>) -> bool {
    let height = if cover.is_some() { 40.0 } else { 32.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), chosen, name));
    let painter = ui.painter_at(rect);
    let hot = response.hovered() || response.has_focus();
    if chosen { painter.rect_filled(rect, ui::CONTROL_RADIUS, theme.main.gamma_multiply(0.16)); } else if hot { painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
    let mut x = rect.left() + 10.0;
    if let Some(cover) = cover {
        let art = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 14.0), vec2(28.0, 28.0));
        match cover {
            Some(texture) => { painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
            None => { painter.rect_filled(art, 4.0, ui::TRACK); }
        }
        x = art.right() + 10.0;
    }
    let ink = if chosen || hot { ui::TEXT } else { ui::SECONDARY };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), ui::display(13.5), ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - 8.0).max(8.0));
    let galley = ui.painter().layout_job(job);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    response.clicked()
}

fn group_title(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| { ui.add_space(10.0); ui::tracked(ui, title, ui::mono(10.5), ui::LABEL, 1.4); });
    ui.add_space(2.0);
}

/// The places in the library that are lists of their own, from Spotify's browse.
pub(super) fn browse(kind: &str, name: &str) -> Source {
    Source::Browse { kind: kind.into(), key: String::new(), name: name.into(), range: "medium_term".into() }
}

impl Music {
    /// Search, then the library's places, then the playlists with their covers.
    pub(super) fn sidebar(&mut self, ui: &mut Ui, theme: &Theme, height: f32, playlists: Option<(Vec<Item>, Option<String>)>) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let (field, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        ui.painter().rect_filled(field, ui::CONTROL_RADIUS, ui::PANEL);
        ui.painter().rect_stroke(field, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let edit = ui.put(
            Rect::from_min_max(field.min + vec2(10.0, 6.0), field.max - vec2(10.0, 6.0)),
            TextEdit::singleline(&mut self.query).vertical_align(Align::Center).frame(false).font(ui::display(13.0)).text_color(ui::TEXT)
                .hint_text(crate::ui::hint("Songs, artists, podcasts")),
        );
        edit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search Spotify"));
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.query.trim().is_empty() {
            self.go(Source::Search(self.query.trim().to_string()), false);
        }
        group_title(ui, "LIBRARY");
        let places: [(&str, Source); 7] = [
            ("Home", Source::Home), ("Liked songs", Source::Liked), ("Albums", Source::Albums),
            ("Artists", browse("following", "Followed artists")), ("Podcasts", browse("shows", "Podcasts")),
            ("Audiobooks", browse("audiobooks", "Audiobooks")), ("Recently played", browse("recent", "Recently played")),
        ];
        for (name, to) in places {
            let chosen = self.source.key() == to.key();
            if nav_row(ui, theme, name, chosen, None) && !chosen { self.go(to, false); }
        }
        ui.horizontal(|ui| {
            group_title(ui, "PLAYLISTS");
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                if glyph_button(ui, theme, Glyph::Plus, 22.0, false, "New playlist").clicked() { self.editor = Some(PlaylistEditor::default()); }
            });
        });
        match playlists {
            None => { ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL); }
            Some((_, Some(why))) => { ui.add(egui::Label::new(egui::RichText::new(why).font(ui::display(11.5)).color(ui::CAUTION)).wrap()); }
            Some((items, None)) => {
                let left = (height - ui.cursor().min.y + ui.max_rect().top()).max(40.0);
                ScrollArea::vertical().max_height(left).id_salt("playlists").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for playlist in &items {
                        let chosen = matches!(&self.source, Source::Playlist { id, .. } if *id == playlist.id);
                        let place = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 40.0));
                        let cover = if ui.is_rect_visible(place) { self.covers.get(ui.ctx(), playlist.cover.as_deref()) } else { None };
                        if ui.push_id(&playlist.id, |ui| nav_row(ui, theme, &playlist.name, chosen, Some(cover.as_ref()))).inner && !chosen {
                            self.go(Source::Playlist { id: playlist.id.clone(), name: playlist.name.clone(), uri: playlist.uri.clone() }, false);
                        }
                    }
                });
            }
        }
    }

    /// The first page: where to jump back in, what you play most, what you played
    /// last, and your playlists.
    pub(super) fn home(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, playlists: &[Item], height: f32) {
        let top = Source::Browse { kind: "top_tracks".into(), key: String::new(), name: "Top tracks".into(), range: self.top_range.into() };
        let recent = browse("recent", "Recently played");
        self.want(top.key(), top.request());
        self.want(recent.key(), recent.request());
        let (top_items, recent_items) = {
            let shared = self.shared.lock().unwrap();
            (shared.lists.get(&top.key()).cloned(), shared.lists.get(&recent.key()).cloned())
        };
        let playing = now.track.as_ref().map(|t| t.uri.clone());
        ScrollArea::vertical().max_height(height).id_salt("music-home").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            ui::text(ui, "Home", ui::display_bold(28.0), ui::TEXT);
            ui.add_space(6.0);
            // Jump back in: Liked songs, then the first playlists.
            let width = ui.available_width();
            let columns = if width >= 760.0 { 3 } else if width >= 420.0 { 2 } else { 1 };
            let tile_w = (width - 10.0 * (columns as f32 - 1.0)) / columns as f32;
            let mut tiles: Vec<(String, Option<String>, Source)> = vec![("Liked songs".into(), None, Source::Liked)];
            tiles.extend(playlists.iter().take(5).map(|p| (p.name.clone(), p.cover.clone(), Source::Playlist { id: p.id.clone(), name: p.name.clone(), uri: p.uri.clone() })));
            for row in tiles.chunks(columns) {
                ui.horizontal(|ui| {
                    for (name, cover, to) in row {
                        let (rect, response) = ui.allocate_exact_size(vec2(tile_w, 56.0), Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.as_str()));
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 8.0, if response.hovered() || response.has_focus() { ui::TRACK } else { ui::PANEL });
                        if response.has_focus() { painter.rect_stroke(rect, 8.0, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
                        let art = Rect::from_min_size(rect.min, vec2(56.0, 56.0));
                        match self.covers.get(ui.ctx(), cover.as_deref()) {
                            Some(texture) => { painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                            None => {
                                painter.rect_filled(art, 8.0, if *to == Source::Liked { theme.main.gamma_multiply(0.35) } else { ui::TRACK });
                                paint_glyph(&painter, art.shrink(18.0), if *to == Source::Liked { Glyph::Heart } else { Glyph::Queue }, ui::TEXT, *to == Source::Liked);
                            }
                        }
                        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), ui::display_bold(13.5), ui::TEXT);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((tile_w - 76.0).max(8.0));
                        let galley = ui.painter().layout_job(job);
                        painter.galley(pos2(art.right() + 12.0, rect.center().y - galley.size().y / 2.0), galley, ui::TEXT);
                        if response.clicked() { self.go(to.clone(), true); }
                    }
                });
            }
            ui.add_space(18.0);
            let split = width >= 760.0;
            let column = if split { (width - 36.0) * 0.55 } else { width };
            let top_section = |this: &mut Music, ui: &mut Ui| {
                ui.horizontal(|ui| {
                    ui::text(ui, "Your top tracks", ui::display_bold(18.0), ui::TEXT);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (label, range) in [("ALL TIME", "long_term"), ("6 MONTHS", "medium_term"), ("4 WEEKS", "short_term")] {
                            if ui::chip(ui, theme, label, None, this.top_range == range).clicked() { this.top_range = range; }
                        }
                    });
                });
                match &top_items {
                    None => { ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL); }
                    Some((_, Some(why))) => { ui.add(egui::Label::new(egui::RichText::new(why).font(ui::display(12.0)).color(ui::LABEL)).wrap()); }
                    Some((items, None)) => {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (n, item) in items.iter().take(5).enumerate() {
                            let lit = playing.as_deref() == Some(item.uri.as_str());
                            if ui.push_id(("top", n), |ui| this.ranked_row(ui, theme, Some(n + 1), item, lit)).inner { this.send(json!({ "cmd": "play", "track": item.uri })); }
                        }
                    }
                }
            };
            let recent_section = |this: &mut Music, ui: &mut Ui| {
                ui::text(ui, "Recently played", ui::display_bold(18.0), ui::TEXT);
                match &recent_items {
                    None => { ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL); }
                    Some((_, Some(why))) => { ui.add(egui::Label::new(egui::RichText::new(why).font(ui::display(12.0)).color(ui::LABEL)).wrap()); }
                    Some((items, None)) => {
                        let mut seen = std::collections::HashSet::new();
                        let unique: Vec<Item> = items.iter().filter(|i| seen.insert(i.uri.clone())).take(6).cloned().collect();
                        this.cover_shelf(ui, theme, &unique, 3);
                    }
                }
            };
            if split {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 36.0;
                    ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| { ui.set_width(column); top_section(self, ui); });
                    let rest = ui.available_width();
                    ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| { ui.set_width(rest); recent_section(self, ui); });
                });
            } else {
                top_section(self, ui);
                ui.add_space(14.0);
                recent_section(self, ui);
            }
            ui.add_space(22.0);
            ui::text(ui, "Your playlists", ui::display_bold(18.0), ui::TEXT);
            self.collection_cards(ui, theme, playlists);
        });
    }

    /// A numbered track with its picture, for the top tracks.
    fn ranked_row(&mut self, ui: &mut Ui, theme: &Theme, n: Option<usize>, item: &Item, lit: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Play {} by {}", item.name, item.by)));
        let painter = ui.painter_at(rect);
        if lit { painter.rect_filled(rect, 6.0, theme.main.gamma_multiply(0.14)); } else if response.hovered() || response.has_focus() { painter.rect_filled(rect, 6.0, ui::PANEL); }
        if let Some(n) = n { painter.text(pos2(rect.left() + 22.0, rect.center().y), Align2::RIGHT_CENTER, n.to_string(), ui::mono(12.0), if lit { theme.main } else { ui::LABEL }); }
        let art = Rect::from_min_size(pos2(rect.left() + if n.is_some() { 32.0 } else { 6.0 }, rect.center().y - 19.0), vec2(38.0, 38.0));
        match self.covers.get(ui.ctx(), item.cover.as_deref()) {
            Some(texture) => { painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
            None => { painter.rect_filled(art, 4.0, ui::TRACK); }
        }
        let text_left = art.right() + 12.0;
        let room = (rect.right() - 60.0 - text_left).max(8.0);
        let mut name = egui::text::LayoutJob::simple_singleline(item.name.clone(), ui::display(14.0), if lit { theme.main } else { ui::TEXT });
        name.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let name = ui.painter().layout_job(name);
        painter.galley(pos2(text_left, rect.center().y - 17.0), name, ui::TEXT);
        let mut by = egui::text::LayoutJob::simple_singleline(item.by.clone(), ui::mono(11.5), ui::SECONDARY);
        by.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let by = ui.painter().layout_job(by);
        painter.galley(pos2(text_left, rect.center().y + 3.0), by, ui::SECONDARY);
        if item.duration_ms > 0 { painter.text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, clock(item.duration_ms), ui::mono(12.0), ui::SECONDARY); }
        response.clicked()
    }

    /// Pictures with a name and who made them, a few to a row; a click plays a track
    /// or opens the rest.
    pub(super) fn cover_shelf(&mut self, ui: &mut Ui, theme: &Theme, items: &[Item], columns: usize) {
        let width = ui.available_width();
        let tile = ((width - 10.0 * (columns as f32 - 1.0)) / columns as f32).max(40.0);
        for row in items.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for item in row {
                    let (rect, response) = ui.allocate_exact_size(vec2(tile, tile + 44.0), Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} — {}", item.name, item.by)));
                    let painter = ui.painter_at(rect);
                    if response.hovered() || response.has_focus() { painter.rect_filled(rect, 8.0, ui::PANEL); }
                    if response.has_focus() { painter.rect_stroke(rect, 8.0, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
                    let art = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(tile - 12.0, tile - 12.0));
                    let cover = if ui.is_rect_visible(rect) { self.covers.get(ui.ctx(), item.cover.as_deref()) } else { None };
                    match cover {
                        Some(texture) => { painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                        None => { painter.rect_filled(art, 6.0, ui::TRACK); }
                    }
                    for (text, font, ink, y) in [(&item.name, ui::display_bold(13.0), ui::TEXT, 4.0), (&item.by, ui::mono(11.0), ui::SECONDARY, 23.0)] {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), font, ink);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((tile - 12.0).max(8.0));
                        let galley = ui.painter().layout_job(job);
                        painter.galley(pos2(art.left(), art.bottom() + y), galley, ink);
                    }
                    if response.clicked() {
                        match item.kind {
                            Kind::Track | Kind::Episode => self.send(json!({ "cmd": "play", "track": item.uri })),
                            Kind::Playlist => self.go(Source::Playlist { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
                            Kind::Album => self.go(Source::Album { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
                            Kind::Artist => self.go(Source::Artist { id: item.id.clone(), name: item.name.clone() }, true),
                            _ => {}
                        }
                    }
                }
            });
        }
    }

    /// The column beside the library: what plays next, the words, or where it plays.
    pub(super) fn side_column(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, height: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, label) in [(SideTab::Queue, "UP NEXT"), (SideTab::Lyrics, "LYRICS"), (SideTab::Devices, "DEVICES")] {
                if ui::chip(ui, theme, label, None, self.side_tab == tab).clicked() {
                    self.side_tab = tab;
                    if tab == SideTab::Devices { self.devices_asked = None; }
                    if tab == SideTab::Queue { self.queue_as_of = None; }
                }
            }
        });
        ui.add_space(10.0);
        let body = (height - 44.0).max(40.0);
        match self.side_tab {
            SideTab::Lyrics => {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), body), Sense::hover());
                let mut inner = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
                inner.set_clip_rect(rect.intersect(ui.clip_rect()));
                self.lyrics(&mut inner, theme, now);
            }
            tab => {
                let source = if tab == SideTab::Queue { Source::Queue } else { Source::Devices };
                let playing_uri = now.track.as_ref().map(|track| track.uri.clone());
                if tab == SideTab::Queue && self.queue_as_of != playing_uri { self.queue_as_of = playing_uri; self.send(json!({ "cmd": "queue" })); }
                if tab == SideTab::Devices && self.devices_asked.is_none_or(|at| at.elapsed() > Duration::from_secs(6)) {
                    self.devices_asked = Some(Instant::now());
                    self.send(json!({ "cmd": "devices" }));
                    ui.ctx().request_repaint_after(Duration::from_secs(6));
                }
                let list = self.shared.lock().unwrap().lists.get(&source.key()).cloned();
                match list {
                    None => { ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL); }
                    Some((_, Some(why))) => { ui.add(egui::Label::new(egui::RichText::new(why).font(ui::display(12.0)).color(ui::LABEL)).wrap()); }
                    Some((items, None)) if items.is_empty() => {
                        let empty = if tab == SideTab::Queue { "Nothing is waiting to play." } else { "No device is online. Open Spotify on one and it shows up here." };
                        ui.add(egui::Label::new(egui::RichText::new(empty).font(ui::display(12.5)).color(ui::SECONDARY)).wrap());
                    }
                    Some((items, None)) => {
                        if tab == SideTab::Queue {
                            ui.add(egui::Label::new(egui::RichText::new("Add from any track's menu. Spotify keeps the order and lets no other app remove from it.").font(ui::mono(10.5)).color(ui::LABEL)).wrap());
                            ui.add_space(6.0);
                        }
                        let room = (body - (ui.cursor().min.y - ui.max_rect().top())).max(40.0);
                        ScrollArea::vertical().max_height(room).id_salt(("side", source.key())).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (index, item) in items.iter().enumerate() {
                                if !ui.push_id(("side", index), |ui| self.ranked_row(ui, theme, None, item, item.active)).inner { continue; }
                                match item.kind {
                                    Kind::Device => { if !item.active { self.send(json!({ "cmd": "transfer", "device": item.id })); } }
                                    // Spotify offers no way to jump into the queue but one step at a time.
                                    _ if tab == SideTab::Queue => { for _ in 0..=index { self.send(json!({ "cmd": "next" })); } }
                                    _ => {}
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    /// The bar along the bottom of every page: what plays, the controls and where
    /// it is in the track, and where it plays and how loud.
    pub(super) fn player_bar(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, height: f32, wide_column: bool) {
        let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(strip, ui::SURFACE_RADIUS, ui::PANEL);
        ui.painter().rect_stroke(strip, ui::SURFACE_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let inner = strip.shrink2(vec2(14.0, 10.0));
        let Some(track) = now.track.clone() else {
            let mut ui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
            ui::text(&mut ui, "Your next listen starts here", ui::display_bold(15.0), ui::TEXT);
            ui::text(&mut ui, "Choose a playlist, album or song to start playback.", ui::display(12.0), ui::SECONDARY);
            return;
        };
        let wide = inner.width() >= 1040.0;
        let side = if wide { (inner.width() * 0.28).clamp(300.0, 360.0) } else { inner.width() };
        // What is playing, and the heart that keeps it.
        let info = if wide { Rect::from_min_size(inner.min, vec2(side, inner.height())) } else { Rect::from_min_size(inner.min, vec2(inner.width(), 52.0)) };
        let mut a = ui.new_child(UiBuilder::new().id_salt("bar-info").max_rect(info).layout(Layout::left_to_right(Align::Center)));
        a.spacing_mut().item_spacing.x = 12.0;
        self.draw_cover(&mut a, 52.0, track.cover.as_deref());
        let liked = now.liked == Some(true);
        let room = (a.available_width() - 44.0).max(20.0);
        a.allocate_ui_with_layout(vec2(room, 52.0), Layout::top_down(Align::Min), |ui| {
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(&track.name).font(ui::display_bold(14.5)).color(ui::TEXT)).truncate()).on_hover_text(&track.name);
            ui.add(egui::Label::new(egui::RichText::new(&track.artists).font(ui::mono(11.5)).color(ui::SECONDARY)).truncate()).on_hover_text(&track.artists);
        });
        if glyph_button(&mut a, theme, Glyph::Heart, 32.0, liked, if liked { "Remove from Liked songs" } else { "Save to Liked songs" }).clicked() { self.send(json!({ "cmd": "like", "on": !liked })); }
        // The controls over the position.
        let middle = if wide { Rect::from_min_max(pos2(info.right() + 20.0, inner.top()), pos2(inner.right() - side - 20.0, inner.bottom())) }
            else { Rect::from_min_max(pos2(inner.left(), info.bottom() + 4.0), pos2(inner.right(), info.bottom() + 76.0)) };
        let mut b = ui.new_child(UiBuilder::new().id_salt("bar-controls").max_rect(middle).layout(Layout::top_down(Align::Center)));
        b.spacing_mut().item_spacing = vec2(10.0, 4.0);
        b.horizontal(|ui| {
            let buttons = 34.0 * 4.0 + 42.0 + 10.0 * 4.0;
            ui.add_space(((ui.available_width() - buttons) / 2.0).max(0.0));
            if glyph_button(ui, theme, Glyph::Shuffle, 34.0, now.shuffle, if now.shuffle { "Shuffle is on" } else { "Shuffle" }).clicked() { self.send(json!({ "cmd": "shuffle", "on": !now.shuffle })); }
            if control(ui, theme, Control::Previous, 34.0, false).clicked() { self.send(json!({ "cmd": "prev" })); }
            if play_button(ui, theme, now.playing).clicked() { self.send(json!({ "cmd": "toggle" })); }
            if control(ui, theme, Control::Next, 34.0, false).clicked() { self.send(json!({ "cmd": "next" })); }
            let (glyph, label, next) = match now.repeat.as_str() { "all" => (Glyph::Repeat, "Repeating all", "one"), "one" => (Glyph::RepeatOne, "Repeating this track", "off"), _ => (Glyph::Repeat, "Repeat", "all") };
            if glyph_button(ui, theme, glyph, 34.0, now.repeat != "off", label).clicked() { self.send(json!({ "cmd": "repeat", "mode": next })); }
        });
        b.horizontal(|ui| {
            ui::text(ui, &clock(now.position_ms()), ui::mono(11.0), ui::SECONDARY);
            let width = (ui.available_width() - 48.0).max(12.0);
            if let Some(to) = ui::slider(ui, theme, width, now.position_ms() as f32 / track.duration_ms.max(1) as f32, "Playback position") {
                if ui.input(|i| i.pointer.any_released()) || !ui.input(|i| i.pointer.any_down()) { self.send(json!({ "cmd": "seek", "ms": (to * track.duration_ms as f32) as u32 })); }
            }
            ui::text(ui, &clock(track.duration_ms), ui::mono(11.0), ui::SECONDARY);
        });
        // Where it plays and how loud; the words and the queue when there is no column for them.
        let tools = if wide { Rect::from_min_size(pos2(inner.right() - side, inner.top()), vec2(side, inner.height())) }
            else { Rect::from_min_max(pos2(inner.left(), middle.bottom() + 2.0), inner.max) };
        let mut c = ui.new_child(UiBuilder::new().id_salt("bar-tools").max_rect(tools).layout(Layout::right_to_left(Align::Center)));
        c.spacing_mut().item_spacing.x = 8.0;
        let volume_width = (c.available_width() * 0.36).clamp(60.0, 110.0);
        if let Some(to) = ui::slider(&mut c, theme, volume_width, now.volume as f32 / 100.0, "Music volume") { self.send(json!({ "cmd": "volume", "percent": (to * 100.0).round() as u8 })); }
        let (speaker, _) = c.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        paint_glyph(c.painter(), speaker, Glyph::Speaker, ui::SECONDARY, false);
        let place = now.device.clone().unwrap_or_else(|| "This desktop".into()).to_uppercase();
        let devices = ui::button(&mut c, theme, &crate::panels::truncate(&place, 16), None, false).on_hover_text("Choose where it plays");
        if devices.clicked() { if wide_column { self.side_tab = SideTab::Devices; self.devices_asked = None; } else { self.go(Source::Devices, true); self.devices_asked = None; } }
        let queue_on = if wide_column { self.side_tab == SideTab::Queue } else { self.source == Source::Queue };
        if glyph_button(&mut c, theme, Glyph::Queue, 32.0, queue_on, "Up next").clicked() {
            if wide_column { self.side_tab = SideTab::Queue; self.queue_as_of = None; } else { self.go(Source::Queue, true); self.queue_as_of = None; }
        }
        let lyrics_on = if wide_column { self.side_tab == SideTab::Lyrics } else { self.source == Source::Lyrics };
        if glyph_button(&mut c, theme, Glyph::Lyrics, 32.0, lyrics_on, "Lyrics").clicked() {
            if wide_column { self.side_tab = SideTab::Lyrics; } else { self.go(Source::Lyrics, true); self.lyrics_view.reset(); }
        }
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
    }
}

/// The large round play or pause button in the middle of the bar.
fn play_button(ui: &mut Ui, theme: &Theme, playing: bool) -> Response {
    let size = 42.0;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let label = if playing { "Pause" } else { "Play" };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let response = response.on_hover_text(label);
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, if response.hovered() { Color32::WHITE } else { ui::TEXT });
    if response.has_focus() { painter.circle_stroke(rect.center(), size / 2.0 + 2.0, Stroke::new(1.5, theme.main)); }
    let c = rect.center();
    if playing {
        painter.rect_filled(Rect::from_center_size(c - vec2(4.5, 0.0), vec2(4.0, 15.0)), 1.0, ui::GROUND);
        painter.rect_filled(Rect::from_center_size(c + vec2(4.5, 0.0), vec2(4.0, 15.0)), 1.0, ui::GROUND);
    } else {
        painter.add(Shape::convex_polygon(vec![c + vec2(-5.0, -8.0), c + vec2(9.0, 0.0), c + vec2(-5.0, 8.0)], ui::GROUND, Stroke::NONE));
    }
    response
}
