//! Settings → System history: the integrity result in full, the snapshots, what
//! changed since one of them, and going back. Reading and running are in
//! `desos.rs`; this only draws what was last read and asks for more.

use crate::desos::{Action, Desos, Diff, Fetch, Snapshot, Status, Tone, View};
use crate::settings::{heading, note};
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use std::time::{Duration, Instant};

/// What going back means, said before it is done. Every line is shown.
pub const ROLLBACK_TERMS: [&str; 5] = [
    "It takes effect at the next restart. Until then this system keeps running as it is.",
    "Personal files are not touched: home folders are never part of a snapshot.",
    "System changes made between now and the restart are lost.",
    "The system being replaced is kept as a snapshot, so going back can itself be undone.",
    "You will be asked for an administrator password.",
];

/// How many changed files of each kind are listed before "and N more".
const FILES_SHOWN: usize = 12;
/// The list is read again when the section has been open this long.
const REFRESH_AFTER: Duration = Duration::from_secs(30);

pub fn tone_color(tone: Tone) -> Color32 {
    match tone { Tone::Plain => ui::TEXT, Tone::Quiet => ui::SECONDARY, Tone::Caution => ui::CAUTION }
}

fn paragraph(ui: &mut Ui, text: &str, font: egui::FontId, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(color)).wrap());
}

/// A fingerprint in groups of four, as people compare them.
fn grouped(fingerprint: &str) -> String {
    fingerprint.chars().collect::<Vec<_>>().chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

impl Desos {
    /// The section's content. Asks for the snapshots when it is first drawn and
    /// every so often while it stays open; never waits for them.
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme) {
        let view = self.view();
        let due = self.refreshed.is_none_or(|at| at.elapsed() >= REFRESH_AFTER);
        if due && view.working.is_none() {
            self.refreshed = Some(Instant::now());
            self.request(Action::Status);
            self.request(Action::List);
        }
        integrity(ui, &view);
        ui.add_space(22.0);
        self.history(ui, theme, &view);
    }

    fn history(&mut self, ui: &mut Ui, theme: &Theme, view: &View) {
        heading(ui, "SNAPSHOTS", "The system as it was before and after each change, newest first. Personal files are never part of a snapshot.");
        // Where undo cannot work, the tool's own reason stands in for the list.
        if let (Fetch::Unsupported(why), _) | (_, Fetch::Unsupported(why)) = (&view.status, &view.snapshots) {
            paragraph(ui, why, ui::display(13.0), ui::TEXT);
            return;
        }
        let status = match &view.status { Fetch::Ready(status) => Some(status), _ => None };
        if let Fetch::Failed(why) = &view.status { paragraph(ui, why, ui::display(13.0), ui::CAUTION); }
        if let Some(status) = status {
            if status.restart_pending {
                let target = status.rollback.as_ref().map_or(String::new(), |rollback| format!(" to snapshot {}", rollback.target));
                paragraph(ui, &format!("A restart is pending: the system goes back{target} when it next starts. System changes made until then are lost."), ui::display(13.0), ui::CAUTION);
            }
            if let Some(id) = status.booted_snapshot {
                paragraph(ui, &format!("This start is running read-only from snapshot {id}. Go back to it and restart to keep it."), ui::display(13.0), ui::CAUTION);
            }
        }
        let busy = view.working.is_some();
        let can_snapshot = !busy && status.is_some_and(|status| !status.restart_pending && status.booted_snapshot.is_none());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui.add_enabled_ui(can_snapshot, |ui| ui::button(ui, theme, "TAKE A SNAPSHOT NOW", None, false)).inner.clicked() {
                self.request(Action::Snapshot);
            }
            if ui.add_enabled_ui(!busy, |ui| ui::button(ui, theme, "REFRESH", None, false)).inner.clicked() {
                self.refreshed = Some(Instant::now());
                self.request(Action::Status);
                self.request(Action::List);
            }
        });
        match view.working {
            Some(Action::Snapshot) => note(ui, "Taking a snapshot. You are asked for an administrator password first."),
            Some(Action::Rollback(id)) => note(ui, &format!("Setting snapshot {id} as the system for the next restart. You are asked for an administrator password first.")),
            _ => {}
        }
        if let Some((done, message)) = &view.outcome {
            paragraph(ui, message, ui::display(13.0), if *done { ui::TEXT } else { ui::CAUTION });
        }
        ui.add_space(10.0);
        match &view.snapshots {
            Fetch::Idle | Fetch::Loading => note(ui, "Reading snapshots…"),
            Fetch::Failed(why) => paragraph(ui, why, ui::display(13.0), ui::CAUTION),
            Fetch::Unsupported(_) => {}
            Fetch::Ready(snapshots) if snapshots.is_empty() => note(ui, "No snapshots yet. One is taken before and after every system change, and when a start reaches the sign-in screen."),
            Fetch::Ready(snapshots) => {
                ui.spacing_mut().item_spacing.y = 2.0;
                for snapshot in snapshots {
                    let chosen = self.selected == Some(snapshot.id);
                    if snapshot_row(ui, theme, snapshot, chosen, status.and_then(|status| status.good_id) == Some(snapshot.id)).clicked() {
                        self.confirming = None;
                        self.selected = if chosen { None } else { Some(snapshot.id) };
                        if !chosen { self.request(Action::Diff(snapshot.id)); }
                    }
                    if chosen { self.detail(ui, theme, snapshot, view, status); }
                }
            }
        }
    }

    /// Under the chosen snapshot: what differs from now, and the way back.
    fn detail(&mut self, ui: &mut Ui, theme: &Theme, snapshot: &Snapshot, view: &View, status: Option<&Status>) {
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 0, top: 6, bottom: 14 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            match view.diff.as_ref().filter(|(id, _)| *id == snapshot.id).map(|(_, diff)| diff) {
                Some(Fetch::Ready(diff)) => diff_view(ui, diff),
                Some(Fetch::Failed(why)) | Some(Fetch::Unsupported(why)) => paragraph(ui, why, ui::display(13.0), ui::CAUTION),
                _ => note(ui, "Comparing with the system as it is now…"),
            }
            ui.add_space(6.0);
            let busy = view.working.is_some();
            let pending = status.is_some_and(|status| status.restart_pending);
            if self.confirming == Some(snapshot.id) {
                let named = if snapshot.reason.is_empty() { String::new() } else { format!(" — {}", snapshot.reason) };
                paragraph(ui, &format!("Go back to snapshot {}{named}?", snapshot.id), ui::display_bold(14.0), ui::TEXT);
                for term in ROLLBACK_TERMS { note(ui, term); }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ui.add_enabled_ui(!busy, |ui| ui::button(ui, theme, "GO BACK AT NEXT RESTART", None, true)).inner.clicked() {
                        self.confirming = None;
                        self.request(Action::Rollback(snapshot.id));
                    }
                    if ui::button(ui, theme, "CANCEL", None, false).clicked() { self.confirming = None; }
                });
            } else {
                if ui.add_enabled_ui(!busy && !pending, |ui| ui::button(ui, theme, "GO BACK TO THIS STATE", None, false)).inner.clicked() {
                    self.confirming = Some(snapshot.id);
                }
                if pending { note(ui, "Going back is already set for the next restart."); }
            }
        });
    }
}

/// The verify result in full: the line of the system panel, each check that did
/// not pass with its reason, what a pass does not mean, and the key it rests on.
fn integrity(ui: &mut Ui, view: &View) {
    heading(ui, "INTEGRITY", "Whether this system is still the one in its signed build record. Checked briefly at every start and in full once a day.");
    let Some(integrity) = &view.integrity else { note(ui, "Reading…"); return };
    let line = integrity.line();
    ui::text(ui, line.value, ui::mono(13.0), tone_color(line.tone));
    let lines: Vec<&str> = line.detail.lines().collect();
    for (index, text) in lines.iter().enumerate() {
        if index == 0 {
            paragraph(ui, text, ui::display(13.0), ui::TEXT);
        } else if index + 1 == lines.len() {
            ui.add_space(4.0);
            note(ui, text);
        } else {
            // One check and its reason.
            paragraph(ui, text, ui::mono(12.0), tone_color(line.tone));
        }
    }
    if let crate::desos::Integrity::Result(verify) = integrity {
        if let Some(fingerprint) = &verify.key_fingerprint {
            ui.add_space(4.0);
            note(ui, "Signing key the record was checked against. Compare it with a fingerprint obtained from somewhere other than this machine:");
            paragraph(ui, &grouped(fingerprint), ui::mono(12.0), ui::SECONDARY);
        }
    }
}

/// What differs between a snapshot and the system now: packages, then files.
fn diff_view(ui: &mut Ui, diff: &Diff) {
    let against = if diff.to == "current" { "the system as it is now".to_string() } else { format!("snapshot {}", diff.to) };
    let places = if diff.paths.is_empty() { "/etc".to_string() } else { diff.paths.join(", ") };
    ui::tracked(ui, &format!("SNAPSHOT {} COMPARED WITH {}", diff.from, against.to_uppercase()), ui::mono(10.5), ui::LABEL, 1.0);
    if diff.is_empty() {
        note(ui, &format!("Nothing differs: the same packages, and the same files in {places}."));
        return;
    }
    for line in diff.packages.lines() { paragraph(ui, &line, ui::mono(12.0), ui::TEXT); }
    for (sign, files) in [("+", &diff.added), ("−", &diff.removed), ("~", &diff.modified)] {
        for file in files.iter().take(FILES_SHOWN) { paragraph(ui, &format!("{sign} {file}"), ui::mono(12.0), ui::SECONDARY); }
        if files.len() > FILES_SHOWN { note(ui, &format!("{sign} and {} more", files.len() - FILES_SHOWN)); }
    }
    let shortened = if diff.truncated { " The list was shortened." } else { "" };
    note(ui, &format!("Files are compared in {places}: + added since the snapshot, − removed, ~ changed.{shortened}"));
}

/// One snapshot: its number, kind and time; why it was taken; what changed.
fn snapshot_row(ui: &mut Ui, theme: &Theme, snapshot: &Snapshot, chosen: bool, known_good: bool) -> egui::Response {
    const PAD: f32 = 10.0;
    let width = ui.available_width();
    let text_width = (width - 2.0 * PAD).max(1.0);
    let crumb = if known_good { format!("{} · LAST KNOWN GOOD", snapshot.crumb()) } else { snapshot.crumb() };
    let crumb = ui.painter().layout(crumb, ui::mono(10.5), ui::LABEL, text_width);
    let reason = if snapshot.reason.is_empty() { "No reason recorded" } else { snapshot.reason.as_str() };
    let name = ui.painter().layout(reason.to_owned(), ui::display(14.0), ui::TEXT, text_width);
    let changed = snapshot.what_changed();
    let about = (!changed.is_empty()).then(|| ui.painter().layout(changed, ui::display(12.0), ui::SECONDARY, text_width));
    let about_height = about.as_ref().map_or(0.0, |about| about.size().y + 2.0);
    let height = crumb.size().y + 3.0 + name.size().y + about_height + 2.0 * 8.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let lit = response.hovered() || response.has_focus();
    if chosen {
        ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, theme.main.gamma_multiply(0.16));
    } else if lit {
        ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    }
    let mut top = rect.left_top() + vec2(PAD, 8.0);
    let crumb_height = crumb.size().y;
    ui.painter().galley(top, crumb, ui::LABEL);
    top.y += crumb_height + 3.0;
    let name_height = name.size().y;
    ui.painter().galley(top, name, ui::TEXT);
    top.y += name_height + 2.0;
    if let Some(about) = about { ui.painter().galley(top, about, ui::SECONDARY); }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, chosen, format!("Snapshot {}: {reason}", snapshot.id)));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desos::{Integrity, Kind};

    fn snapshot(id: u64, kind: Kind, reason: &str) -> Snapshot {
        Snapshot { id, kind, reason: reason.into(), created: Some("2026-11-12T10:03:22Z".into()), kernel: None, packages: None, planned: None, protected: false }
    }

    /// Draws the section and returns every piece of text painted.
    fn texts(desos: &mut Desos, events: Vec<egui::Event>) -> Vec<String> {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        let mut output = None;
        // Twice: the first pass lays out, the second delivers a click.
        for events in [Vec::new(), events] {
            output = Some(ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 2400.0))), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| desos.show(ui, &theme));
            }));
        }
        output.unwrap().shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()), _ => None,
        }).collect()
    }

    fn has(texts: &[String], needle: &str) -> bool { texts.iter().any(|text| text.contains(needle)) }

    #[test]
    fn an_unsupported_system_shows_the_reason_and_no_list_or_actions() {
        let why = "Undo needs a btrfs system root; this one is ext4.";
        let mut desos = Desos::for_test(Some(Integrity::NotRun), Fetch::Unsupported(why.into()), Fetch::Unsupported(why.into()));
        let texts = texts(&mut desos, Vec::new());
        assert!(has(&texts, why), "{texts:?}");
        assert!(!has(&texts, "TAKE A SNAPSHOT NOW") && !has(&texts, "No snapshots yet"), "{texts:?}");
    }

    #[test]
    fn the_list_says_what_changed_and_why_newest_first() {
        let mut after = snapshot(14, Kind::Post, "after: apt upgraded nano");
        after.packages = Some(crate::desos::Packages { upgraded: vec![crate::desos::PackageChange { name: "nano".into(), version: None, from: Some("7.2-1".into()), to: Some("8.0-1".into()) }], ..Default::default() });
        let mut desos = Desos::for_test(Some(Integrity::NotRun), Fetch::Ready(Status { good_id: Some(12), ..Default::default() }), Fetch::Ready(vec![after, snapshot(12, Kind::BootGood, "start reached the sign-in screen")]));
        let texts = texts(&mut desos, Vec::new());
        assert!(has(&texts, "after: apt upgraded nano") && has(&texts, "upgraded nano 7.2-1 → 8.0-1"), "{texts:?}");
        assert!(has(&texts, "#14 · AFTER A CHANGE") && has(&texts, "LAST KNOWN GOOD"), "{texts:?}");
        assert!(has(&texts, "TAKE A SNAPSHOT NOW"));
        assert!(!has(&texts, "GO BACK"), "nothing is offered before a snapshot is chosen");
    }

    #[test]
    fn going_back_states_its_terms_before_anything_is_asked_for() {
        let mut desos = Desos::for_test(Some(Integrity::NotRun), Fetch::Ready(Status::default()), Fetch::Ready(vec![snapshot(13, Kind::Pre, "before: apt upgrade nano")]));
        desos.selected = Some(13);
        let chosen = texts(&mut desos, Vec::new());
        assert!(has(&chosen, "GO BACK TO THIS STATE") && !has(&chosen, "GO BACK AT NEXT RESTART"), "{chosen:?}");
        desos.confirming = Some(13);
        let asked = texts(&mut desos, Vec::new());
        assert!(has(&asked, "Go back to snapshot 13 — before: apt upgrade nano?"), "{asked:?}");
        for term in ROLLBACK_TERMS { assert!(has(&asked, term), "missing: {term}"); }
        assert!(ROLLBACK_TERMS.iter().any(|term| term.contains("next restart")) && ROLLBACK_TERMS.iter().any(|term| term.contains("Personal files are not touched")));
        assert!(has(&asked, "GO BACK AT NEXT RESTART") && has(&asked, "CANCEL"));
        assert!(desos.view().working.is_none(), "drawing the question starts nothing");
    }

    #[test]
    fn a_pending_restart_is_said_and_blocks_further_changes() {
        let status = Status { restart_pending: true, rollback: Some(crate::desos::Rollback { target: 13, undo: Some(15), time: None, automatic: false }), ..Default::default() };
        let mut desos = Desos::for_test(Some(Integrity::NotRun), Fetch::Ready(status), Fetch::Ready(vec![snapshot(13, Kind::Pre, "before")]));
        desos.selected = Some(13);
        let texts = texts(&mut desos, Vec::new());
        assert!(has(&texts, "A restart is pending: the system goes back to snapshot 13"), "{texts:?}");
        assert!(has(&texts, "already set for the next restart"));
    }

    #[test]
    fn the_integrity_group_lists_failing_checks_and_the_caveat() {
        let bytes = br#"{"schema":"desos-verify-v1","mode":"full","overall":"fail","finished":"2026-11-12T09:00:01Z","key_fingerprint":"CDBC1835BA4F06D5F0BD4A9000B6AE1830FF9F59",
            "checks":[{"id":"packages.files","status":"fail","reason":"1 file differs: /usr/bin/des-ui"},{"id":"signatures.key","status":"pass","reason":"one key"}]}"#;
        let mut desos = Desos::for_test(Some(crate::desos::parse_verify(bytes, None)), Fetch::Unsupported("live".into()), Fetch::Unsupported("live".into()));
        let texts = texts(&mut desos, Vec::new());
        assert!(has(&texts, "DIFFERS") && has(&texts, "packages.files: 1 file differs: /usr/bin/des-ui"), "{texts:?}");
        assert!(!has(&texts, "signatures.key"), "checks that passed are not listed");
        assert!(has(&texts, crate::desos::CAVEAT) && has(&texts, "CDBC 1835 BA4F"));
    }
}
