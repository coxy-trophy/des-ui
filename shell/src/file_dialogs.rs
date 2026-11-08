//! Two of the file manager's dialogs: renaming many items at once, and an item's
//! properties with its permissions.

use crate::file_ops::Operation;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Key, Modifiers};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RenameMode { Pattern, Replace }

/// The new name of each item that changes, or why the renaming cannot be done.
/// A pattern may hold {name}, {ext}, {n}, {n:03} and {date}; without {ext} the
/// extension is kept.
pub fn plan(items: &[(PathBuf, Option<SystemTime>)], mode: RenameMode, pattern: &str, find: &str, replace: &str) -> Result<Vec<(PathBuf, String)>, String> {
    if mode == RenameMode::Pattern && pattern.trim().is_empty() { return Err("Type a pattern, such as {name}-{n}".into()); }
    if mode == RenameMode::Replace && find.is_empty() { return Err("Type what to find".into()); }
    let mut out = Vec::new();
    let mut taken = std::collections::HashSet::new();
    for (index, (path, modified)) in items.iter().enumerate() {
        let old = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let new = match mode {
            RenameMode::Replace => old.replace(find, replace),
            RenameMode::Pattern => {
                let folder = path.is_dir();
                let (stem, ext) = match (path.file_stem(), path.extension()) {
                    (Some(stem), Some(ext)) if !folder && !old.starts_with('.') => (stem.to_string_lossy().into_owned(), ext.to_string_lossy().into_owned()),
                    _ => (old.clone(), String::new()),
                };
                let date = modified.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string()).unwrap_or_default();
                let mut name = expand_numbers(pattern, index + 1).replace("{name}", &stem).replace("{ext}", &ext).replace("{date}", &date);
                if !pattern.contains("{ext}") && !ext.is_empty() { name = format!("{name}.{ext}"); }
                name
            },
        };
        let new = new.trim().to_string();
        if new.is_empty() || new == "." || new == ".." || new.contains('/') || new.contains('\0') { return Err(format!("“{new}” cannot be a name")); }
        if !taken.insert(new.clone()) { return Err(format!("Two items would both be called “{new}”")); }
        if new == old { continue; }
        let target = path.with_file_name(&new);
        if items.iter().any(|(other, _)| *other == target) { return Err(format!("“{new}” is another item being renamed; rename in two steps")); }
        if std::fs::symlink_metadata(&target).is_ok() { return Err(format!("“{new}” already exists")); }
        out.push((path.clone(), new));
    }
    if out.is_empty() { return Err("Nothing would change".into()); }
    Ok(out)
}

/// `{n}` and `{n:0W}` as the item's number, padded to W digits.
fn expand_numbers(pattern: &str, n: usize) -> String {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(at) = rest.find("{n") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        if let Some(stripped) = after.strip_prefix('}') { out.push_str(&n.to_string()); rest = stripped; continue; }
        if let Some(spec) = after.strip_prefix(":0").and_then(|s| s.find('}').map(|end| (&s[..end], &s[end + 1..]))) {
            if let Ok(width) = spec.0.parse::<usize>() { out.push_str(&format!("{n:0width$}", width = width.min(9))); rest = spec.1; continue; }
        }
        out.push_str("{n"); rest = after;
    }
    out.push_str(rest);
    out
}

pub enum Outcome { Run(Vec<Operation>), Cancel }

pub struct BatchRename { items: Vec<(PathBuf, Option<SystemTime>)>, mode: RenameMode, pattern: String, find: String, replace: String, focused: bool }
impl BatchRename {
    pub fn new(items: Vec<(PathBuf, Option<SystemTime>)>) -> Self {
        BatchRename { items, mode: RenameMode::Pattern, pattern: "{name}-{n:02}".into(), find: String::new(), replace: String::new(), focused: false }
    }
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Outcome> {
        let mut outcome = None;
        let response = egui::Modal::new(egui::Id::new("files-batch-rename")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(260.0, 520.0));
            ui.label(egui::RichText::new(format!("Rename {} items", self.items.len())).font(ui::display_bold(19.0)).color(ui::TEXT));
            ui.add_space(10.0);
            crate::settings::segmented(ui, theme, &mut self.mode, &[(RenameMode::Pattern, "Pattern"), (RenameMode::Replace, "Find & replace")]);
            ui.add_space(10.0);
            let width = ui.available_width();
            let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, id: &str| {
                ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(text).id(egui::Id::new(id)).margin(egui::vec2(10.0, 4.0)).vertical_align(egui::Align::Center).font(ui::mono(13.0)).hint_text(ui::hint(hint)))
            };
            let first = match self.mode {
                RenameMode::Pattern => {
                    let r = field(ui, &mut self.pattern, "{name}-{n:02}", "rename-pattern");
                    crate::settings::note(ui, "{name} the name · {ext} the extension · {n} or {n:03} a number · {date} when it was changed. The extension is kept unless you use {ext}.");
                    r
                },
                RenameMode::Replace => {
                    let r = field(ui, &mut self.find, "Find", "rename-find");
                    ui.add_space(4.0);
                    field(ui, &mut self.replace, "Replace with", "rename-replace");
                    r
                },
            };
            if !self.focused { first.request_focus(); self.focused = true; }
            ui.add_space(10.0);
            let planned = plan(&self.items, self.mode, &self.pattern, &self.find, &self.replace);
            match &planned {
                Ok(changes) => {
                    for (path, new) in changes.iter().take(5) {
                        let old = path.file_name().unwrap_or_default().to_string_lossy();
                        let mut job = egui::text::LayoutJob::default();
                        job.append(&old, 0.0, egui::TextFormat::simple(ui::mono(12.0), ui::SECONDARY));
                        job.append("  →  ", 0.0, egui::TextFormat::simple(ui::mono(12.0), ui::LABEL));
                        job.append(new, 0.0, egui::TextFormat::simple(ui::mono(12.0), ui::TEXT));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
                        ui.label(job);
                    }
                    if changes.len() > 5 { crate::settings::note(ui, &format!("and {} more", changes.len() - 5)); }
                },
                Err(why) => { ui.label(egui::RichText::new(why).font(ui::display(13.0)).color(ui::CAUTION)); },
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let ready = planned.as_ref().map_or(0, Vec::len);
                let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                if (ui.add_enabled_ui(ready > 0, |ui| ui::button(ui, theme, &format!("RENAME {ready}"), Some("ENTER"), ready > 0)).inner.clicked() || enter) && ready > 0 {
                    if let Ok(changes) = &planned {
                        outcome = Some(Outcome::Run(changes.iter().map(|(source, name)| Operation::Rename { source: source.clone(), name: name.clone() }).collect()));
                    }
                }
                if ui::button(ui, theme, "CANCEL", Some("ESC"), false).clicked() { outcome = Some(Outcome::Cancel); }
            });
        });
        if response.should_close() && outcome.is_none() { outcome = Some(Outcome::Cancel); }
        outcome
    }
}

/// Chooses an application for files, the ones made for their kind first.
pub struct OpenWith {
    files: Vec<PathBuf>,
    loading: Option<Receiver<(Option<String>, Option<String>, Vec<crate::apps::Handler>)>>,
    mime: Option<String>, default: Option<String>, handlers: Vec<crate::apps::Handler>,
    show_all: bool, remember: bool, error: Option<String>,
}
impl OpenWith {
    pub fn new(files: Vec<PathBuf>, ctx: &egui::Context) -> Self {
        let (send, receive) = std::sync::mpsc::channel();
        let (first, wake) = (files.first().cloned(), ctx.clone());
        std::thread::spawn(move || {
            let mime = first.as_deref().and_then(crate::apps::mime_of);
            let default = mime.as_deref().and_then(crate::apps::default_for);
            let _ = send.send((mime, default, crate::apps::handlers()));
            wake.request_repaint();
        });
        OpenWith { files, loading: Some(receive), mime: None, default: None, handlers: Vec::new(), show_all: false, remember: false, error: None }
    }
    /// True when it should close.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> bool {
        if let Some((mime, default, handlers)) = self.loading.as_ref().and_then(|r| r.try_recv().ok()) {
            self.mime = mime; self.default = default; self.handlers = handlers; self.loading = None;
            if !self.handlers.iter().any(|h| self.mime.as_deref().is_some_and(|m| crate::apps::handles(h, m))) { self.show_all = true; }
        }
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("files-open-with")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(260.0, 460.0));
            let what = match self.files.as_slice() { [one] => one.file_name().unwrap_or_default().to_string_lossy().into_owned(), many => format!("{} items", many.len()) };
            ui.add(egui::Label::new(egui::RichText::new(format!("Open {what} with")).font(ui::display_bold(19.0)).color(ui::TEXT)).wrap());
            if let Some(mime) = &self.mime { ui.label(egui::RichText::new(mime).font(ui::mono(11.5)).color(ui::LABEL)); }
            ui.add_space(10.0);
            if self.loading.is_some() { crate::settings::note(ui, "Looking for applications…"); }
            let mime = self.mime.clone().unwrap_or_default();
            let mut fitting: Vec<&crate::apps::Handler> = self.handlers.iter().filter(|h| crate::apps::handles(h, &mime)).collect();
            fitting.sort_by_key(|h| Some(&h.entry.desktop_id) != self.default.as_ref());
            let others: Vec<&crate::apps::Handler> = if self.show_all { self.handlers.iter().filter(|h| !crate::apps::handles(h, &mime) && crate::apps::opens_files(h)).collect() } else { Vec::new() };
            let mut chosen = None;
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for handler in fitting.iter().chain(others.iter()) {
                    let default = Some(&handler.entry.desktop_id) == self.default.as_ref();
                    let width = ui.available_width();
                    let (rect, row) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::click());
                    if row.hovered() || row.has_focus() { ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
                    ui.painter().text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, &handler.entry.name, ui::display(14.0), ui::TEXT);
                    if default { ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, "DEFAULT", ui::mono(10.5), theme.main); }
                    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &handler.entry.name));
                    if row.clicked() { chosen = Some((*handler).clone()); }
                }
            });
            if let Some(handler) = chosen {
                match crate::apps::launch_with(&handler, &self.files) {
                    Ok(()) => {
                        if self.remember && !mime.is_empty() { if let Err(error) = crate::apps::set_default(&handler.entry.desktop_id, &mime) { self.error = Some(error); } }
                        close = self.error.is_none();
                    },
                    Err(error) => self.error = Some(format!("{} could not start: {error}", handler.entry.name)),
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let toggle = ui::toggle(ui, theme, self.show_all);
                if toggle.clicked() { self.show_all = !self.show_all; }
                ui.label(egui::RichText::new("Show every application").font(ui::display(13.0)).color(ui::TEXT));
            });
            if !mime.is_empty() && self.files.len() == 1 {
                ui.horizontal(|ui| {
                    let toggle = ui::toggle(ui, theme, self.remember);
                    if toggle.clicked() { self.remember = !self.remember; }
                    ui.label(egui::RichText::new("Always open this kind of file with it").font(ui::display(13.0)).color(ui::TEXT));
                });
            }
            if let Some(error) = &self.error { ui.colored_label(ui::CAUTION, error); }
            ui.add_space(10.0);
            if ui::button(ui, theme, "CANCEL", Some("ESC"), false).clicked() { close = true; }
        });
        close || response.should_close()
    }
}

/// Names a new archive and picks its format.
pub struct Compress { sources: Vec<PathBuf>, name: String, format: &'static str, focused: bool }
impl Compress {
    pub fn new(sources: Vec<PathBuf>) -> Self {
        let name = match sources.as_slice() {
            // "notes.md" makes "notes.zip"; a folder keeps its whole name.
            [one] if one.is_dir() => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
            [one] => one.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
            _ => sources.first().and_then(|p| p.parent()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
        };
        Compress { sources, name, format: ".zip", focused: false }
    }
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Outcome> {
        let mut outcome = None;
        let response = egui::Modal::new(egui::Id::new("files-compress")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(260.0, 440.0));
            let what = if self.sources.len() == 1 { "1 item".to_string() } else { format!("{} items", self.sources.len()) };
            ui.label(egui::RichText::new(format!("Compress {what}")).font(ui::display_bold(19.0)).color(ui::TEXT));
            ui.add_space(10.0);
            let width = ui.available_width();
            let field = ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(&mut self.name).margin(egui::vec2(10.0, 4.0)).vertical_align(egui::Align::Center).hint_text(ui::hint("Archive name")));
            if !self.focused { field.request_focus(); self.focused = true; }
            ui.add_space(8.0);
            crate::settings::segmented(ui, theme, &mut self.format, &[(".zip", ".zip"), (".tar.gz", ".tar.gz"), (".tar.xz", ".tar.xz"), (".7z", ".7z")]);
            crate::settings::note(ui, ".zip opens everywhere; .tar.xz is smallest.");
            let name = self.name.trim();
            let valid = !name.is_empty() && !name.contains('/') && name != "." && name != "..";
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                if (ui.add_enabled_ui(valid, |ui| ui::button(ui, theme, "COMPRESS", Some("ENTER"), valid)).inner.clicked() || enter) && valid {
                    if let Some(folder) = self.sources.first().and_then(|p| p.parent()) {
                        let archive = folder.join(format!("{name}{}", self.format));
                        outcome = Some(Outcome::Run(vec![Operation::Compress { sources: self.sources.clone(), archive }]));
                    }
                }
                if ui::button(ui, theme, "CANCEL", Some("ESC"), false).clicked() { outcome = Some(Outcome::Cancel); }
            });
        });
        if response.should_close() && outcome.is_none() { outcome = Some(Outcome::Cancel); }
        outcome
    }
}

/// The name for a numeric id from a passwd- or group-like file.
fn name_for(file: &str, id: u32) -> Option<String> {
    std::fs::read_to_string(file).ok()?.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        (fields.nth(1)?.parse::<u32>().ok()? == id).then(|| name.to_string())
    })
}

pub struct Properties {
    path: PathBuf, kind: String, directory: bool, bytes: u64, modified: Option<SystemTime>,
    owner: String, group: String, mine: bool, mode: u32,
    count: Option<crate::quicklook::FolderCount>, counting: Option<Receiver<crate::quicklook::FolderCount>>,
    error: Option<String>,
}
impl Properties {
    pub fn new(path: PathBuf, kind: String, ctx: &egui::Context) -> Option<Self> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::symlink_metadata(&path).ok()?;
        let directory = meta.is_dir();
        let counting = directory.then(|| {
            let (send, receive) = std::sync::mpsc::channel();
            let (root, wake) = (path.clone(), ctx.clone());
            std::thread::spawn(move || { let _ = send.send(crate::quicklook::count_folder(&root, crate::quicklook::MAX_WALK)); wake.request_repaint(); });
            receive
        });
        Some(Properties {
            owner: name_for("/etc/passwd", meta.uid()).unwrap_or_else(|| meta.uid().to_string()),
            group: name_for("/etc/group", meta.gid()).unwrap_or_else(|| meta.gid().to_string()),
            mine: meta.uid() == rustix::process::geteuid().as_raw(),
            mode: meta.permissions().mode() & 0o777,
            kind, directory, bytes: meta.len(), modified: meta.modified().ok(), path, count: None, counting, error: None,
        })
    }
    /// True when it should close.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> bool {
        use std::os::unix::fs::PermissionsExt;
        if let Some(count) = self.counting.as_ref().and_then(|r| r.try_recv().ok()) { self.count = Some(count); self.counting = None; }
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("files-properties")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(260.0, 460.0));
            let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
            ui.add(egui::Label::new(egui::RichText::new(name).font(ui::display_bold(19.0)).color(ui::TEXT)).wrap());
            let size = if self.directory {
                match &self.count {
                    Some(c) => format!("{} files · {} folders · {}{}", c.files, c.folders, crate::quicklook::human_size(c.bytes), if c.partial { " at least" } else { "" }),
                    None => "Counting…".into(),
                }
            } else { crate::quicklook::human_size(self.bytes) };
            ui.label(egui::RichText::new(format!("{} · {size}", self.kind)).font(ui::mono(12.0)).color(ui::SECONDARY));
            ui.add_space(10.0);
            let row = |ui: &mut egui::Ui, label: &str, value: &str| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 18.0), egui::Sense::hover());
                    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, ui::mono(11.5), ui::LABEL);
                    ui.add(egui::Label::new(egui::RichText::new(value).font(ui::mono(12.0)).color(ui::TEXT)).wrap());
                });
            };
            row(ui, "Where", &self.path.parent().map(|p| p.display().to_string()).unwrap_or_default());
            if let Some(time) = self.modified { row(ui, "Modified", &chrono::DateTime::<chrono::Local>::from(time).format("%-d %b %Y, %H:%M").to_string()); }
            row(ui, "Owner", &format!("{} · group {}", self.owner, self.group));
            ui.add_space(10.0);
            crate::settings::heading(ui, "WHO CAN DO WHAT", if self.mine { "Changes apply at once." } else { "Only the owner can change these." });
            let mut mode = self.mode;
            egui::Grid::new("files-permissions").num_columns(4).spacing(egui::vec2(18.0, 8.0)).show(ui, |ui| {
                ui.label("");
                for title in ["READ", if self.directory { "CHANGE" } else { "WRITE" }, if self.directory { "OPEN" } else { "RUN" }] {
                    ui.label(egui::RichText::new(title).font(ui::mono(10.5)).color(ui::LABEL));
                }
                ui.end_row();
                for (who, shift) in [("You", 6), ("Group", 3), ("Others", 0)] {
                    ui.label(egui::RichText::new(who).font(ui::display(13.0)).color(ui::TEXT));
                    for (bit, what) in [(4, "read"), (2, "write"), (1, "run")] {
                        let mask = bit << shift;
                        let on = mode & mask != 0;
                        let toggle = ui.add_enabled_ui(self.mine, |ui| ui::toggle(ui, theme, on)).inner;
                        toggle.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, self.mine, on, format!("{who} can {what}")));
                        if toggle.clicked() { mode ^= mask; }
                    }
                    ui.end_row();
                }
            });
            ui.label(egui::RichText::new(format!("{mode:03o}")).font(ui::mono(11.5)).color(ui::LABEL));
            if mode != self.mode {
                match std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(mode)) {
                    Ok(()) => { self.mode = mode; self.error = None; },
                    Err(error) => self.error = Some(format!("Not changed: {error}")),
                }
            }
            if let Some(error) = &self.error { ui.colored_label(ui::CAUTION, error); }
            ui.add_space(12.0);
            if ui::button(ui, theme, "CLOSE", Some("ESC"), false).clicked() || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { close = true; }
        });
        close || response.should_close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns_number_keep_extensions_and_refuse_clashes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for n in ["IMG_1.jpg", "IMG_2.jpg", "taken.jpg"] { std::fs::write(p.join(n), b"").unwrap(); }
        let items = vec![(p.join("IMG_1.jpg"), None), (p.join("IMG_2.jpg"), None)];
        let names = |r: Result<Vec<(PathBuf, String)>, String>| r.map(|v| v.into_iter().map(|(_, n)| n).collect::<Vec<_>>());
        assert_eq!(names(plan(&items, RenameMode::Pattern, "holiday-{n:02}", "", "")), Ok(vec!["holiday-01.jpg".into(), "holiday-02.jpg".into()]));
        assert_eq!(names(plan(&items, RenameMode::Pattern, "{name}.{ext}.bak", "", "")), Ok(vec!["IMG_1.jpg.bak".into(), "IMG_2.jpg.bak".into()]));
        assert_eq!(names(plan(&items, RenameMode::Replace, "", "IMG_", "photo-")), Ok(vec!["photo-1.jpg".into(), "photo-2.jpg".into()]));
        assert!(plan(&items, RenameMode::Pattern, "same", "", "").unwrap_err().contains("both"));
        assert!(plan(&items[..1], RenameMode::Pattern, "taken", "", "").unwrap_err().contains("already exists"));
        assert!(plan(&items, RenameMode::Replace, "", "1", "2").unwrap_err().contains("both") || plan(&items, RenameMode::Replace, "", "1", "2").unwrap_err().contains("another"));
        assert_eq!(plan(&items, RenameMode::Pattern, "{name}", "", ""), Err("Nothing would change".into()));
        assert_eq!(expand_numbers("{n}-{n:03}-{nope}", 7), "7-007-{nope}");
    }
}
