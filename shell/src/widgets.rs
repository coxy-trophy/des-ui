//! Panel widgets from programs: a file in `~/.config/des-ui/widgets/` names a program,
//! the program prints a line, and the panel draws it in the Signal style. The
//! contract is small on purpose, like a status bar's custom modules: the last line of
//! standard output, as JSON (`text`, `value`, `label`, `series`, `state`, `lines`,
//! `tooltip`) or as plain text.
//!
//! Programs run one at a time on a worker of their own, bounded in time and output,
//! never through a shell, and never twice at once. A program that is slow or fails
//! says "no answer" in its tile and nothing more.

use crate::{apps, sensors, theme::Theme, ui};
use des_common::Panel;
use eframe::egui::{self, Color32, Rect, Sense, Ui, pos2, vec2};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant};

/// How long a program may take; after that it is killed and gives no answer.
const RUN_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_EVERY: Duration = Duration::from_secs(1);
const DEFAULT_EVERY: Duration = Duration::from_secs(10);
/// How often the widgets directory is read again.
const RELOAD_EVERY: Duration = Duration::from_secs(30);
const MAX_WIDGETS: usize = 16;
const MAX_TEXT: usize = 120;
const MAX_LINES: usize = 16;
const MAX_LINE: usize = 80;
const MAX_SERIES: usize = 120;
/// Points kept for a series widget that reports one value at a time.
const HISTORY: usize = 120;
/// Runs waiting for the worker; more than this and the due ones wait for the next frame.
const QUEUE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Meter,
    Series,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Ok,
    Caution,
    Off,
}

/// One `.widget` file.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// The file's name without `.widget`.
    pub id: String,
    pub name: String,
    /// The program and its arguments; `exec` split like a desktop entry's.
    pub exec: Vec<String>,
    pub every: Duration,
    pub panel: Panel,
    pub kind: Kind,
}

/// What a program said, after bounding.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    pub text: Option<String>,
    /// A fraction from 0 to 1, for a meter.
    pub value: Option<f32>,
    pub label: Option<String>,
    pub series: Option<Vec<f32>>,
    pub state: State,
    pub lines: Vec<String>,
    pub tooltip: Option<String>,
}

/// `5s`, `1m`, `2h`, or a plain number of seconds. Never less than a second.
pub fn parse_every(text: &str) -> Option<Duration> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text, "s"),
    };
    let amount: u64 = digits.parse().ok()?;
    let seconds = match unit.trim() {
        "s" | "" => amount,
        "m" => amount.checked_mul(60)?,
        "h" => amount.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(seconds).max(MIN_EVERY))
}

/// Reads one `.widget` file. Without a name or a program there is no widget; other
/// mistakes fall back to the defaults, so one bad line does not lose the widget.
pub fn parse_declaration(text: &str, id: &str) -> Option<Declaration> {
    let mut declaration = Declaration {
        id: id.to_string(),
        name: String::new(),
        exec: Vec::new(),
        every: DEFAULT_EVERY,
        panel: Panel::Right,
        kind: Kind::Text,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "name" => declaration.name = clean(value, 40),
            "exec" => declaration.exec = apps::split_exec(value),
            "every" => declaration.every = parse_every(value).unwrap_or(DEFAULT_EVERY),
            "panel" => declaration.panel = if value == "left" { Panel::Left } else { Panel::Right },
            "kind" => {
                declaration.kind = match value {
                    "meter" => Kind::Meter,
                    "series" => Kind::Series,
                    "list" => Kind::List,
                    _ => Kind::Text,
                }
            }
            _ => {}
        }
    }
    if declaration.name.is_empty() || declaration.exec.is_empty() || declaration.exec[0].is_empty() {
        return None;
    }
    Some(declaration)
}

/// Every widget declared in `dir`, in file name order, at most [`MAX_WIDGETS`].
pub fn declarations_in(dir: &Path) -> Vec<Declaration> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "widget") && path.is_file())
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?;
            let text = std::fs::read_to_string(path).ok()?;
            parse_declaration(&text, id)
        })
        .take(MAX_WIDGETS)
        .collect()
}

fn widgets_dir() -> Option<PathBuf> {
    des_common::input::config_path().and_then(|path| path.parent().map(|dir| dir.join("widgets")))
}

/// Text fit for a tile: no control characters, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// The last non-empty line of what a program printed, read as JSON when it is an
/// object and as the text itself otherwise.
pub fn parse_output(stdout: &str) -> Reading {
    let Some(line) = stdout.lines().rev().find(|line| !line.trim().is_empty()) else {
        return Reading::default();
    };
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(line) else {
        return Reading { text: Some(clean(line, MAX_TEXT)), ..Reading::default() };
    };
    let string = |key: &str, max: usize| fields.get(key).and_then(|v| v.as_str()).map(|s| clean(s, max)).filter(|s| !s.is_empty());
    let number = |v: &serde_json::Value| v.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
    Reading {
        text: string("text", MAX_TEXT),
        value: fields.get("value").and_then(number).map(|v| v.clamp(0.0, 1.0)),
        label: string("label", 40),
        series: fields.get("series").and_then(|v| v.as_array()).map(|values| {
            values.iter().filter_map(number).take(MAX_SERIES).collect()
        }),
        state: match fields.get("state").and_then(|v| v.as_str()) {
            Some("caution") => State::Caution,
            Some("off") => State::Off,
            _ => State::Ok,
        },
        lines: fields
            .get("lines")
            .and_then(|v| v.as_array())
            .map(|lines| lines.iter().filter_map(|l| l.as_str()).map(|l| clean(l, MAX_LINE)).take(MAX_LINES).collect())
            .unwrap_or_default(),
        tooltip: string("tooltip", 200),
    }
}

/// When each widget runs. Pure, so the timing can be tested without programs: a
/// widget is due when its time has come and it is not already running, and its next
/// time is counted from when it finished, so a slow program delays only itself.
#[derive(Debug, Default)]
pub struct Schedule {
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    id: String,
    every: Duration,
    due: Instant,
    running: bool,
}

impl Schedule {
    /// Keeps the timing of widgets that are still declared; new ones are due at once.
    pub fn set(&mut self, declarations: &[Declaration], now: Instant) {
        let mut entries = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let kept = self.entries.iter().position(|entry| entry.id == declaration.id).map(|at| self.entries.remove(at));
            entries.push(match kept {
                Some(mut entry) => {
                    entry.every = declaration.every;
                    entry
                }
                None => Entry { id: declaration.id.clone(), every: declaration.every, due: now, running: false },
            });
        }
        self.entries = entries;
    }

    /// The widgets to run now, each marked as running so it is not handed out twice.
    pub fn due(&mut self, now: Instant) -> Vec<String> {
        self.entries
            .iter_mut()
            .filter(|entry| !entry.running && now >= entry.due)
            .map(|entry| {
                entry.running = true;
                entry.id.clone()
            })
            .collect()
    }

    /// A run ended, with or without an answer.
    pub fn finished(&mut self, id: &str, now: Instant) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.running = false;
            entry.due = now + entry.every;
        }
    }

    /// Handed out but not yet finished: when a run could not even be queued.
    pub fn release(&mut self, id: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.running = false;
        }
    }

    #[cfg(test)]
    fn running(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.id == id && entry.running)
    }
}

/// A widget as the panel sees it.
pub struct Slot {
    pub declaration: Declaration,
    /// The last answer; `None` before the first, or after a run that gave none.
    pub reading: Option<Reading>,
    /// Whether the last run gave no answer at all.
    pub failed: bool,
    /// Values a series widget reported one at a time.
    pub history: VecDeque<f32>,
}

struct Job {
    id: String,
    exec: Vec<String>,
}

/// Every declared widget, their readings, and the worker that runs them.
pub struct Widgets {
    ctx: egui::Context,
    slots: Vec<Slot>,
    schedule: Schedule,
    jobs: Option<SyncSender<Job>>,
    results: Receiver<(String, Option<Reading>)>,
    last_reload: Option<Instant>,
    dir: Option<PathBuf>,
}

impl Widgets {
    /// Starts the worker; the directory is read on the first `pump`.
    pub fn new(ctx: egui::Context) -> Widgets {
        let (jobs, job_rx) = mpsc::sync_channel::<Job>(QUEUE);
        let (results_tx, results) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("widgets".into())
            .spawn(move || {
                for job in job_rx {
                    let args: Vec<&str> = job.exec[1..].iter().map(String::as_str).collect();
                    let reading = sensors::run_bounded(Path::new(&job.exec[0]), &args, RUN_TIMEOUT).map(|out| parse_output(&out));
                    if results_tx.send((job.id, reading)).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            })
            .ok();
        Widgets { ctx, slots: Vec::new(), schedule: Schedule::default(), jobs: Some(jobs), results, last_reload: None, dir: widgets_dir() }
    }

    /// Reads the directory again, keeping the readings of widgets that are unchanged.
    pub fn reload(&mut self) {
        let declarations = self.dir.as_deref().map(declarations_in).unwrap_or_default();
        self.set_declarations(declarations);
        self.last_reload = Some(Instant::now());
    }

    fn set_declarations(&mut self, declarations: Vec<Declaration>) {
        let now = Instant::now();
        self.schedule.set(&declarations, now);
        let mut old = std::mem::take(&mut self.slots);
        self.slots = declarations
            .into_iter()
            .map(|declaration| match old.iter().position(|slot| slot.declaration == declaration) {
                Some(at) => old.remove(at),
                None => Slot { declaration, reading: None, failed: false, history: VecDeque::new() },
            })
            .collect();
    }

    /// Once a frame: reads the directory when due, takes in answers, starts due runs.
    pub fn pump(&mut self) {
        let now = Instant::now();
        if self.last_reload.is_none_or(|at| now.duration_since(at) >= RELOAD_EVERY) {
            self.reload();
        }
        while let Ok((id, reading)) = self.results.try_recv() {
            self.deliver(&id, reading);
        }
        for id in self.schedule.due(now) {
            let Some(slot) = self.slots.iter().find(|slot| slot.declaration.id == id) else { continue };
            let job = Job { id: id.clone(), exec: slot.declaration.exec.clone() };
            match self.jobs.as_ref().map(|jobs| jobs.try_send(job)) {
                Some(Ok(())) => {}
                Some(Err(TrySendError::Full(_))) | None => self.schedule.release(&id),
                Some(Err(TrySendError::Disconnected(_))) => {
                    self.jobs = None;
                    self.schedule.release(&id);
                }
            }
        }
        if self.slots.iter().any(|slot| slot.reading.is_none() && !slot.failed) {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// An answer for `id`, or none.
    fn deliver(&mut self, id: &str, reading: Option<Reading>) {
        self.schedule.finished(id, Instant::now());
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.declaration.id == id) else { return };
        match reading {
            Some(reading) => {
                if slot.declaration.kind == Kind::Series {
                    if let Some(value) = reading.series.as_ref().and_then(|s| s.last().copied()).or(reading.value) {
                        if slot.history.len() >= HISTORY {
                            slot.history.pop_front();
                        }
                        slot.history.push_back(value);
                    }
                }
                slot.reading = Some(reading);
                slot.failed = false;
            }
            None => slot.failed = true,
        }
    }

    /// The widgets that belong on `panel`.
    pub fn on(&self, panel: Panel) -> impl Iterator<Item = &Slot> {
        self.slots.iter().filter(move |slot| slot.declaration.panel == panel)
    }

    /// A set of widgets with no worker, for drawing in tests.
    #[cfg(test)]
    fn for_test(ctx: egui::Context, declarations: Vec<Declaration>) -> Widgets {
        let (_, results) = mpsc::channel();
        let mut widgets = Widgets { ctx, slots: Vec::new(), schedule: Schedule::default(), jobs: None, results, last_reload: Some(Instant::now()), dir: None };
        widgets.set_declarations(declarations);
        widgets
    }
}

/// Draws the widgets of `panel`, each in a tile under its name.
pub fn show(ui: &mut Ui, theme: &Theme, widgets: &Widgets, panel: Panel) {
    for slot in widgets.on(panel) {
        let response = ui
            .scope(|ui| {
                ui::tile(ui, theme, false, |ui| {
                    ui::meta(ui, &slot.declaration.name.to_uppercase(), ui::LABEL);
                    match &slot.reading {
                        None => ui::meta(ui, if slot.failed { "no answer" } else { "…" }, ui::LABEL),
                        Some(reading) => draw(ui, theme, slot.declaration.kind, reading, &slot.history),
                    }
                })
            })
            .response;
        if let Some(tooltip) = slot.reading.as_ref().and_then(|reading| reading.tooltip.as_deref()) {
            response.on_hover_text(tooltip);
        }
        ui.add_space(6.0);
    }
}

fn draw(ui: &mut Ui, theme: &Theme, kind: Kind, reading: &Reading, history: &VecDeque<f32>) {
    let color = match reading.state {
        State::Ok => ui::TEXT,
        State::Caution => theme.caution,
        State::Off => ui::LABEL,
    };
    let accent = if reading.state == State::Caution { theme.caution } else { theme.main };
    match kind {
        Kind::Text => {
            if let Some(label) = &reading.label {
                ui::meta(ui, label, ui::SECONDARY);
            }
            ui::text(ui, reading.text.as_deref().unwrap_or("—"), ui::mono(12.0), color);
        }
        Kind::Meter => {
            let value = reading.value.unwrap_or(0.0);
            ui.horizontal(|ui| {
                if let Some(label) = &reading.label {
                    ui::meta(ui, label, ui::SECONDARY);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let text = reading.text.clone().unwrap_or_else(|| format!("{:.0}%", value * 100.0));
                    ui::text(ui, &text, ui::mono(12.0), color);
                });
            });
            meter_bar(ui, value, if reading.state == State::Off { ui::LABEL } else { accent });
        }
        Kind::Series => {
            let values: VecDeque<f32> = match &reading.series {
                Some(series) if series.len() > 1 => series.iter().copied().collect(),
                _ => history.clone(),
            };
            let max = values.iter().copied().fold(0.0_f32, f32::max).max(f32::EPSILON);
            ui::sparkline(ui, &values, max, vec2(ui.available_width(), 28.0), accent, true);
            if let Some(text) = &reading.text {
                ui::text(ui, text, ui::mono(11.0), color);
            }
        }
        Kind::List => {
            if reading.lines.is_empty() {
                ui::text(ui, reading.text.as_deref().unwrap_or("—"), ui::mono(11.0), color);
            }
            for line in &reading.lines {
                ui::text(ui, line, ui::mono(11.0), color);
            }
        }
    }
}

/// A thin bar in the panel's track colour, the fraction filled in `color`.
fn meter_bar(ui: &mut Ui, frac: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, ui::LINE);
    let filled = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * frac.clamp(0.0, 1.0), rect.bottom()));
    painter.rect_filled(filled, 0.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(id: &str, every: u64) -> Declaration {
        Declaration { id: id.into(), name: id.into(), exec: vec!["true".into()], every: Duration::from_secs(every), panel: Panel::Left, kind: Kind::Text }
    }

    #[test]
    fn reads_a_declaration_with_defaults_and_bounds() {
        let text = "# a widget\nname = Load\nexec = cat /proc/loadavg\n";
        let widget = parse_declaration(text, "load").unwrap();
        assert_eq!(widget.name, "Load");
        assert_eq!(widget.exec, vec!["cat", "/proc/loadavg"]);
        assert_eq!(widget.every, DEFAULT_EVERY);
        assert_eq!((widget.panel, widget.kind), (Panel::Right, Kind::Text));

        let text = "name = Disk\nexec = \"my prog\" --json\nevery = 0s\npanel = left\nkind = meter\nnonsense\nkind\n";
        let widget = parse_declaration(text, "disk").unwrap();
        assert_eq!(widget.exec, vec!["my prog", "--json"]);
        assert_eq!(widget.every, MIN_EVERY);
        assert_eq!((widget.panel, widget.kind), (Panel::Left, Kind::Meter));

        assert!(parse_declaration("exec = true\n", "x").is_none(), "a name is needed");
        assert!(parse_declaration("name = x\n", "x").is_none(), "a program is needed");
        assert_eq!(parse_every("5s"), Some(Duration::from_secs(5)));
        assert_eq!(parse_every("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_every("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_every("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_every("0"), Some(MIN_EVERY));
        assert_eq!(parse_every("soon"), None);
        assert_eq!(parse_every("5 days"), None);
        assert_eq!(parse_declaration("name = Ctrl\u{7}X\nexec = true\n", "c").unwrap().name, "CtrlX");
    }

    #[test]
    fn reads_a_directory_in_name_order_and_only_widget_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.widget"), "name = B\nexec = true\n").unwrap();
        std::fs::write(dir.path().join("a.widget"), "name = A\nexec = true\n").unwrap();
        std::fs::write(dir.path().join("c.txt"), "name = C\nexec = true\n").unwrap();
        std::fs::write(dir.path().join("bad.widget"), "exec = true\n").unwrap();
        let ids: Vec<String> = declarations_in(dir.path()).into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(declarations_in(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn reads_json_and_plain_output_within_bounds() {
        let reading = parse_output("noise\n{\"text\":\"ok\",\"value\":1.7,\"label\":\"root\",\"series\":[1,2,\"x\",3],\"state\":\"caution\",\"lines\":[\"a\",\"b\"],\"tooltip\":\"hi\"}\n\n");
        assert_eq!(reading.text.as_deref(), Some("ok"));
        assert_eq!(reading.value, Some(1.0));
        assert_eq!(reading.label.as_deref(), Some("root"));
        assert_eq!(reading.series, Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(reading.state, State::Caution);
        assert_eq!(reading.lines, vec!["a", "b"]);
        assert_eq!(reading.tooltip.as_deref(), Some("hi"));

        let plain = parse_output("line one\nup 3 days\n");
        assert_eq!(plain.text.as_deref(), Some("up 3 days"));
        assert_eq!(plain.state, State::Ok);
        assert!(plain.value.is_none() && plain.lines.is_empty());

        assert_eq!(parse_output("[1,2]").text.as_deref(), Some("[1,2]"), "a JSON array is text");
        assert_eq!(parse_output("").text, None);
        assert_eq!(parse_output("\u{1b}[31mred\u{7}").text.as_deref(), Some("[31mred"), "control characters go");

        let long = format!("{{\"text\":\"{}\",\"lines\":[{}],\"series\":[{}],\"value\":-3,\"state\":\"weird\"}}",
            "x".repeat(500), vec![format!("\"{}\"", "y".repeat(200)); 40].join(","), vec!["1"; 500].join(","));
        let bounded = parse_output(&long);
        assert_eq!(bounded.text.unwrap().len(), MAX_TEXT);
        assert_eq!(bounded.lines.len(), MAX_LINES);
        assert!(bounded.lines.iter().all(|line| line.len() == MAX_LINE));
        assert_eq!(bounded.series.unwrap().len(), MAX_SERIES);
        assert_eq!(bounded.value, Some(0.0));
        assert_eq!(bounded.state, State::Ok);
        assert!(parse_output("{\"value\":\"NaN\"}").value.is_none());
    }

    #[test]
    fn a_widget_runs_at_most_once_per_period_and_never_overlaps() {
        let start = Instant::now();
        let mut schedule = Schedule::default();
        schedule.set(&[declaration("fast", 1), declaration("slow", 1)], start);
        assert_eq!(schedule.due(start), vec!["fast", "slow"], "new widgets run at once");
        assert!(schedule.due(start + Duration::from_millis(900)).is_empty(), "both are still running");

        // The fast one finishes; the slow one is still going at the next second.
        schedule.finished("fast", start + Duration::from_millis(100));
        assert!(schedule.due(start + Duration::from_millis(500)).is_empty(), "not due before its second");
        assert_eq!(schedule.due(start + Duration::from_millis(1100)), vec!["fast"], "fast runs again on time");
        assert!(schedule.running("slow"), "slow was never handed out twice");

        // The slow one finishes late; its next run counts from then, the fast one's does not move.
        schedule.finished("slow", start + Duration::from_millis(2500));
        schedule.finished("fast", start + Duration::from_millis(1200));
        let due = schedule.due(start + Duration::from_millis(2300));
        assert_eq!(due, vec!["fast"]);
        schedule.finished("fast", start + Duration::from_millis(2400));
        assert_eq!(schedule.due(start + Duration::from_millis(3500)), vec!["fast", "slow"]);

        // A run that could not be queued is handed out again next time.
        schedule.finished("fast", start + Duration::from_millis(3600));
        schedule.finished("slow", start + Duration::from_millis(3600));
        let due = schedule.due(start + Duration::from_millis(4700));
        assert_eq!(due.len(), 2);
        schedule.release("slow");
        assert_eq!(schedule.due(start + Duration::from_millis(4700)), vec!["slow"]);
    }

    #[test]
    fn declarations_keep_their_timing_and_readings_when_unchanged() {
        let ctx = egui::Context::default();
        let mut widgets = Widgets::for_test(ctx, vec![declaration("a", 5), declaration("b", 5)]);
        widgets.deliver("a", Some(Reading { text: Some("kept".into()), ..Reading::default() }));
        widgets.deliver("b", None);
        assert!(widgets.slots[1].failed);
        let mut changed = declaration("b", 5);
        changed.kind = Kind::List;
        widgets.set_declarations(vec![declaration("a", 5), changed, declaration("c", 5)]);
        assert_eq!(widgets.slots[0].reading.as_ref().and_then(|r| r.text.clone()).as_deref(), Some("kept"));
        assert!(widgets.slots[1].reading.is_none() && !widgets.slots[1].failed, "a changed widget starts afresh");
        assert_eq!(widgets.slots.len(), 3);
        assert_eq!(widgets.on(Panel::Left).count(), 3);
        assert_eq!(widgets.on(Panel::Right).count(), 0);
    }

    #[test]
    fn series_widgets_keep_a_bounded_history_of_single_values() {
        let ctx = egui::Context::default();
        let mut series = declaration("s", 1);
        series.kind = Kind::Series;
        let mut widgets = Widgets::for_test(ctx, vec![series]);
        for i in 0..(HISTORY + 10) {
            widgets.deliver("s", Some(Reading { value: Some(i as f32 / 200.0), ..Reading::default() }));
        }
        assert_eq!(widgets.slots[0].history.len(), HISTORY);
        assert_eq!(widgets.slots[0].history.back().copied(), Some((HISTORY + 9) as f32 / 200.0));
    }

    #[test]
    fn every_kind_draws_without_panic() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let theme = Theme::load(crate::theme::DEFAULT_NAME);
        let mut declarations = Vec::new();
        for (id, kind) in [("t", Kind::Text), ("m", Kind::Meter), ("s", Kind::Series), ("l", Kind::List), ("none", Kind::Text), ("failed", Kind::Meter)] {
            let mut widget = declaration(id, 5);
            widget.kind = kind;
            declarations.push(widget);
        }
        let mut widgets = Widgets::for_test(ctx.clone(), declarations);
        let full = Reading {
            text: Some("42".into()),
            value: Some(0.8),
            label: Some("root".into()),
            series: Some(vec![1.0, 3.0, 2.0]),
            state: State::Caution,
            lines: vec!["one".into(), "two".into()],
            tooltip: Some("more".into()),
        };
        for id in ["t", "m", "s", "l"] {
            widgets.deliver(id, Some(full.clone()));
        }
        widgets.deliver("failed", None);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(220.0);
                show(ui, &theme, &widgets, Panel::Left);
                show(ui, &theme, &widgets, Panel::Right);
            });
        });
    }
}
