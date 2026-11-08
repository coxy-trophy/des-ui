//! The music player's pages: a playlist, album or Liked songs with its header,
//! and search results grouped by kind.

use super::*;
use super::chrome::{Glyph, glyph_button, paint_glyph};
use eframe::egui::Color32;

/// What search shows: everything, or one kind.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Found { #[default] All, Songs, Artists, Albums, Playlists, Podcasts, Episodes }
impl Found {
    fn holds(self, kind: Kind) -> bool {
        match self {
            Found::All => true,
            Found::Songs => kind == Kind::Track,
            Found::Artists => kind == Kind::Artist,
            Found::Albums => kind == Kind::Album,
            Found::Playlists => kind == Kind::Playlist,
            Found::Podcasts => matches!(kind, Kind::Show | Kind::Audiobook),
            Found::Episodes => matches!(kind, Kind::Episode | Kind::Chapter),
        }
    }
}

fn total(items: &[Item]) -> String {
    let ms: u64 = items.iter().map(|i| i.duration_ms as u64).sum();
    let minutes = ms / 60_000;
    if minutes >= 60 { format!("{} h {} min", minutes / 60, minutes % 60) } else { format!("{minutes} min") }
}

impl Music {
    /// The top of a playlist, album or Liked songs: its picture, name, who made it,
    /// how long it runs, and what can be done with it.
    pub(super) fn collection_header(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, source: &Source, items: &[Item], more: bool) {
        let (label, title, mut cover, mut about, mut meta) = match source {
            Source::Playlist { name, .. } => ("PLAYLIST", name.clone(), None, String::new(), String::new()),
            Source::Album { name, .. } => ("ALBUM", name.clone(), items.first().and_then(|i| i.cover.clone()), String::new(),
                items.first().map(|i| i.by.clone()).unwrap_or_default()),
            _ => ("YOUR LIBRARY", "Liked songs".to_string(), None, String::new(), String::new()),
        };
        let mut owner = false;
        if let Source::Playlist { id, .. } = source {
            self.want(format!("details:{id}"), json!({ "cmd": "playlist_details", "id": id }));
            let details = self.shared.lock().unwrap().details.get(id).cloned();
            let listed = self.shared.lock().unwrap().lists.get("playlists:").and_then(|(items, _)| items.iter().find(|p| p.id == *id).cloned());
            if let Some(v) = &details {
                about = v["description"].as_str().unwrap_or_default().to_string();
                meta = v["owner_name"].as_str().unwrap_or_default().to_string();
                cover = v["cover"].as_str().map(str::to_string);
                owner = v["owner"] == true;
            }
            if cover.is_none() { cover = listed.as_ref().and_then(|p| p.cover.clone()); }
            if meta.is_empty() { meta = listed.map(|p| p.by).unwrap_or_default(); }
        }
        let count = match (source, self.shared.lock().unwrap().details.get(match source { Source::Playlist { id, .. } => id.as_str(), _ => "" }).and_then(|v| v["total"].as_u64())) {
            (_, Some(n)) => format!("{n} songs"),
            _ if more => format!("{}+ songs", items.len()),
            _ => format!("{} songs", items.len()),
        };
        let length = if more { String::new() } else { format!(" · {}", total(items)) };
        let meta = [meta, count].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ") + &length;
        let side = if ui.available_width() >= 560.0 { 150.0 } else { 92.0 };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 22.0;
            let (art, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            match self.covers.get(ui.ctx(), cover.as_deref()) {
                Some(texture) => { ui.painter().image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                None => {
                    let liked = *source == Source::Liked;
                    ui.painter().rect_filled(art, 10.0, if liked { theme.main.gamma_multiply(0.35) } else { ui::TRACK });
                    paint_glyph(ui.painter(), art.shrink(side * 0.32), if liked { Glyph::Heart } else { Glyph::Queue }, ui::TEXT, liked);
                }
            }
            ui.vertical(|ui| {
                ui.add_space(if side > 100.0 { 18.0 } else { 2.0 });
                ui::tracked(ui, label, ui::mono(10.5), ui::SECONDARY, 1.6);
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(&title).font(ui::display_bold(if side > 100.0 { 40.0 } else { 26.0 })).color(ui::TEXT)).truncate());
                if !about.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&about).font(ui::display(13.5)).color(ui::SECONDARY)).truncate()); }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&meta).font(ui::mono(12.0)).color(ui::SECONDARY));
            });
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if big_play(ui, theme, &format!("Play {title}")).clicked() {
                if let Some(first) = items.iter().find(|i| matches!(i.kind, Kind::Track | Kind::Episode)) {
                    self.send(json!({ "cmd": "play", "context": source.context(), "track": first.uri }));
                }
            }
            if glyph_button(ui, theme, Glyph::Shuffle, 36.0, now.shuffle, if now.shuffle { "Shuffle is on" } else { "Shuffle" }).clicked() { self.send(json!({ "cmd": "shuffle", "on": !now.shuffle })); }
            let (uri, link) = match source {
                Source::Playlist { id, uri, .. } => (Some(uri.clone()), Some(format!("https://open.spotify.com/playlist/{id}"))),
                Source::Album { id, uri, .. } => (Some(uri.clone()), Some(format!("https://open.spotify.com/album/{id}"))),
                _ => (None, None),
            };
            if let Some(uri) = uri {
                self.want(format!("saved:{uri}"), json!({ "cmd": "library", "uri": uri }));
                let saved = self.shared.lock().unwrap().saved.get(&uri).copied();
                let label = if saved == Some(true) { "SAVED" } else { "SAVE" };
                if ui.add_enabled_ui(saved.is_some(), |ui| ui::button(ui, theme, label, None, saved == Some(true))).inner.on_hover_text(if saved == Some(true) { "Remove from your library" } else { "Save to your library" }).clicked() {
                    self.send(json!({ "cmd": "library", "uri": uri, "save": saved != Some(true) }));
                }
            }
            if let (Source::Playlist { id, .. }, true) = (source, owner) {
                if ui::button(ui, theme, "EDIT DETAILS", None, false).clicked() {
                    let v = self.shared.lock().unwrap().details.get(id).cloned().unwrap_or_default();
                    self.editor = Some(PlaylistEditor { id: id.clone(), name: v["name"].as_str().unwrap_or_default().into(), description: v["description"].as_str().unwrap_or_default().into(), public: v["public"].as_bool().unwrap_or(false), ..Default::default() });
                }
            }
            if let Some(link) = link { if ui::button(ui, theme, "COPY LINK", None, false).clicked() { ui.ctx().copy_text(link); } }
            if ui::button(ui, theme, "REFRESH", None, false).clicked() { self.asked.clear(); self.asked_further = None; self.shared.lock().unwrap().invalidate(); }
        });
        ui.add_space(12.0);
        // The columns of the list below.
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let painter = ui.painter();
        painter.text(pos2(head.left() + 28.0, head.center().y), Align2::RIGHT_CENTER, "#", ui::mono(10.5), ui::LABEL);
        painter.text(pos2(head.left() + 40.0, head.center().y), Align2::LEFT_CENTER, "TITLE", ui::mono(10.5), ui::LABEL);
        if head.width() > 620.0 { painter.text(pos2(head.left() + head.width() * 0.58, head.center().y), Align2::LEFT_CENTER, "ALBUM", ui::mono(10.5), ui::LABEL); }
        painter.text(pos2(head.right() - 10.0, head.center().y), Align2::RIGHT_CENTER, "TIME", ui::mono(10.5), ui::LABEL);
        painter.hline(head.x_range(), head.bottom(), Stroke::new(1.0, ui::LINE));
        ui.add_space(4.0);
    }

    /// Search: the kinds to narrow to, the best match beside the first songs, then
    /// artists, albums, playlists and podcasts.
    pub(super) fn search_page(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, source: &Source, list: &Option<(Vec<Item>, Option<String>)>, more: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (found, label) in [(Found::All, "ALL"), (Found::Songs, "SONGS"), (Found::Artists, "ARTISTS"), (Found::Albums, "ALBUMS"), (Found::Playlists, "PLAYLISTS"), (Found::Podcasts, "PODCASTS"), (Found::Episodes, "EPISODES")] {
                if ui::chip(ui, theme, label, None, self.found == found).clicked() { self.found = found; }
            }
        });
        ui.add_space(10.0);
        let items = match list {
            None => { ui::text(ui, "Searching…", ui::display(12.0), ui::LABEL); return; }
            Some((_, Some(why))) => { ui.add(egui::Label::new(egui::RichText::new(why).font(ui::display(12.0)).color(ui::CAUTION)).wrap()); return; }
            Some((items, None)) => items.clone(),
        };
        let shown: Vec<Item> = items.iter().filter(|i| self.found.holds(i.kind)).cloned().collect();
        if shown.is_empty() { ui::text(ui, "Nothing found. Try another song, artist or album.", ui::display(12.5), ui::SECONDARY); return; }
        let playing = now.track.as_ref().map(|t| t.uri.clone());
        let height = ui.available_height();
        ScrollArea::vertical().max_height(height).id_salt(("search", source.key())).auto_shrink([false, false]).show(ui, |ui| {
            let songs: Vec<&Item> = shown.iter().filter(|i| matches!(i.kind, Kind::Track | Kind::Episode)).collect();
            let others: Vec<Item> = shown.iter().filter(|i| !matches!(i.kind, Kind::Track | Kind::Episode | Kind::Chapter | Kind::Device)).cloned().collect();
            if self.found == Found::All {
                let width = ui.available_width();
                let split = width >= 760.0;
                let best = shown[0].clone();
                let best_card = |this: &mut Music, ui: &mut Ui, w: f32| {
                    ui::text(ui, "Top result", ui::display_bold(17.0), ui::TEXT);
                    let (rect, response) = ui.allocate_exact_size(vec2(w, 200.0), Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} — {}", best.name, described(&best))));
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 12.0, if response.hovered() || response.has_focus() { ui::TRACK } else { ui::PANEL });
                    if response.has_focus() { painter.rect_stroke(rect, 12.0, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
                    let art = Rect::from_min_size(rect.min + vec2(20.0, 20.0), vec2(96.0, 96.0));
                    match this.covers.get(ui.ctx(), best.cover.as_deref()) {
                        Some(texture) => { painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                        None => { painter.rect_filled(art, 8.0, ui::TRACK); }
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(best.name.clone(), ui::display_bold(26.0), ui::TEXT);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 40.0);
                    let galley = ui.painter().layout_job(job);
                    painter.galley(pos2(rect.left() + 20.0, art.bottom() + 14.0), galley, ui::TEXT);
                    let kind = match best.kind { Kind::Track => "SONG", Kind::Artist => "ARTIST", Kind::Album => "ALBUM", Kind::Playlist => "PLAYLIST", Kind::Show => "PODCAST", Kind::Episode => "EPISODE", Kind::Audiobook => "AUDIOBOOK", _ => "" };
                    painter.text(pos2(rect.left() + 20.0, rect.bottom() - 22.0), Align2::LEFT_CENTER, format!("{kind} · {}", best.by), ui::mono(11.5), ui::SECONDARY);
                    if response.clicked() { this.open_item(&best, source); }
                };
                let song_list = |this: &mut Music, ui: &mut Ui| {
                    ui::text(ui, "Songs", ui::display_bold(17.0), ui::TEXT);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, item) in songs.iter().take(4).enumerate() {
                        let cover = this.covers.get(ui.ctx(), item.cover.as_deref());
                        let lit = playing.as_deref() == Some(item.uri.as_str());
                        let picked = ui.push_id(("song", index), |ui| track_row(ui, theme, item, cover.as_ref(), lit, true, false, false, false, false, None)).inner;
                        this.picked(picked, item, source);
                    }
                };
                if split {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 28.0;
                        let left = (width * 0.36).clamp(280.0, 420.0);
                        ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Min), |ui| { ui.set_width(left); best_card(self, ui, left); });
                        let rest = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| { ui.set_width(rest); song_list(self, ui); });
                    });
                } else {
                    best_card(self, ui, width);
                    ui.add_space(12.0);
                    song_list(self, ui);
                }
                if !others.is_empty() {
                    ui.add_space(18.0);
                    ui::text(ui, "Artists, albums and playlists", ui::display_bold(17.0), ui::TEXT);
                    let columns = ((ui.available_width() + 10.0) / 170.0).floor().max(2.0) as usize;
                    self.cover_shelf(ui, theme, &others, columns);
                }
            } else if !songs.is_empty() {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, item) in songs.iter().enumerate() {
                    let cover = self.covers.get(ui.ctx(), item.cover.as_deref());
                    let lit = playing.as_deref() == Some(item.uri.as_str());
                    let picked = ui.push_id(("song", index), |ui| track_row(ui, theme, item, cover.as_ref(), lit, true, false, false, false, false, None)).inner;
                    self.picked(picked, item, source);
                }
            } else {
                let columns = ((ui.available_width() + 10.0) / 170.0).floor().max(2.0) as usize;
                self.cover_shelf(ui, theme, &others, columns);
            }
            if more && ui::button(ui, theme, "MORE RESULTS", None, false).clicked() { self.load_more(source, items.len()); }
        });
    }

    /// Plays a song, or opens what holds songs.
    pub(super) fn open_item(&mut self, item: &Item, source: &Source) {
        match item.kind {
            Kind::Track | Kind::Episode => self.send(json!({ "cmd": "play", "context": source.context(), "track": item.uri })),
            Kind::Playlist => self.go(Source::Playlist { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
            Kind::Album => self.go(Source::Album { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
            Kind::Artist => self.go(Source::Artist { id: item.id.clone(), name: item.name.clone() }, true),
            Kind::Show | Kind::Audiobook => self.go(Source::Browse { kind: if item.kind == Kind::Show { "show" } else { "audiobook" }.into(), key: item.id.clone(), name: item.name.clone(), range: "medium_term".into() }, true),
            _ => {}
        }
    }

    /// What a track row's click asked for, outside the main list.
    fn picked(&mut self, picked: Picked, item: &Item, source: &Source) {
        match picked {
            Picked::Row => self.open_item(item, source),
            Picked::Queue => { self.queued = Some((item.uri.clone(), Instant::now())); self.send(json!({ "cmd": "enqueue", "track": item.uri })); },
            Picked::Save | Picked::RemoveSaved => self.send(json!({ "cmd": "library", "uri": item.uri, "save": picked == Picked::Save })),
            Picked::AddToPlaylist => self.add_to = Some(item.uri.clone()),
            Picked::Album if !item.album_id.is_empty() => self.go(Source::Album { id: item.album_id.clone(), name: item.album.clone(), uri: format!("spotify:album:{}", item.album_id) }, true),
            Picked::Artist if !item.artist_id.is_empty() => self.go(Source::Artist { id: item.artist_id.clone(), name: item.by.split(", ").next().unwrap_or_default().to_string() }, true),
            _ => {}
        }
    }
}

/// The large round play button of a playlist or album, in the accent.
fn big_play(ui: &mut Ui, theme: &Theme, label: &str) -> Response {
    let size = 48.0;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let response = response.on_hover_text(label);
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, if response.hovered() { theme.main.gamma_multiply(0.85) } else { theme.main });
    if response.has_focus() { painter.circle_stroke(rect.center(), size / 2.0 + 3.0, Stroke::new(1.5, ui::TEXT)); }
    let c = rect.center();
    painter.add(Shape::convex_polygon(vec![c + vec2(-6.0, -10.0), c + vec2(11.0, 0.0), c + vec2(-6.0, 10.0)], ui::GROUND, Stroke::NONE));
    response
}
