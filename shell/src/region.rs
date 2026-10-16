//! The date, the time and the language of this machine: the time zone, whether the
//! network keeps the clock right, setting it by hand, and the language and formats
//! programs use.
//!
//! Reading needs nothing. Changing is `timedatectl` and `localectl` run through
//! `sudo`, as the users' part does, with the password of whoever is making the change
//! asked for each time. A language not yet on the machine is made first, by
//! `locale-gen`.

use crate::settings::{Change, heading, note, switch_row};
use crate::theme::Theme;
use crate::ui;
use crate::users::{Step, field, run_as_admin, step};
use eframe::egui::{self, Ui, vec2};
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TIMEDATECTL: &str = "/usr/bin/timedatectl";
const LOCALECTL: &str = "/usr/bin/localectl";
/// Where the locales to make are listed, one to a line.
const TO_MAKE: &str = "/etc/locale.gen";
/// What the formats are: everything a locale says apart from the language itself.
const FORMATS: [&str; 8] = ["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER", "LC_ADDRESS", "LC_TELEPHONE", "LC_NAME"];
/// How many matches of a search are offered at once.
const OFFERED: usize = 24;

/// Whether the clocks count twelve hours twice instead of twenty-four once. Kept
/// here, apart from the settings, because clocks are drawn where no settings reach.
static TWELVE_HOUR: AtomicBool = AtomicBool::new(false);

pub fn set_twelve_hour(on: bool) {
    TWELVE_HOUR.store(on, Ordering::Relaxed);
}

fn clock_text(time: chrono::NaiveTime, twelve: bool, half: bool) -> String {
    let pattern = match (twelve, half) {
        (false, _) => "%H:%M",
        (true, false) => "%-I:%M",
        (true, true) => "%-I:%M %p",
    };
    time.format(pattern).to_string()
}

/// Hours and minutes for a large clock: 13:45, or 1:45.
pub fn hours_minutes(now: &chrono::DateTime<chrono::Local>) -> String {
    clock_text(now.time(), TWELVE_HOUR.load(Ordering::Relaxed), false)
}

/// The same with the half of the day said, for a time that stands alone: 1:45 PM.
pub fn time_of_day(now: &chrono::DateTime<chrono::Local>) -> String {
    clock_text(now.time(), TWELVE_HOUR.load(Ordering::Relaxed), true)
}

/// How the machine's clock is kept, as `timedatectl show` tells it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clock {
    pub zone: String,
    /// Whether the network sets the time.
    pub network: bool,
    /// Whether there is a time service that could.
    pub can_network: bool,
    pub synced: bool,
}

pub fn parse_clock(shown: &str) -> Clock {
    let mut clock = Clock::default();
    for line in shown.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "Timezone" => clock.zone = value.to_string(),
            "NTP" => clock.network = value == "yes",
            "CanNTP" => clock.can_network = value == "yes",
            "NTPSynchronized" => clock.synced = value == "yes",
            _ => {}
        }
    }
    clock
}

/// A date and time as typed, in the form `timedatectl` takes. `None` when it is none.
pub fn parse_when(text: &str) -> Option<String> {
    let text = text.trim();
    ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|pattern| chrono::NaiveDateTime::parse_from_str(text, pattern).ok())
        .map(|when| when.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// The system's locale file: `LANG` and whatever `LC_` lines are set.
pub fn parse_locale(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split('#').next()?.split_once('=')?;
            let (key, value) = (key.trim(), value.trim().trim_matches('"'));
            let ours = key == "LANG" || key == "LANGUAGE" || key.starts_with("LC_");
            (ours && !value.is_empty()).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn set_in<'a>(locale: &'a [(String, String)], key: &str) -> Option<&'a str> {
    locale.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str())
}

/// What `localectl set-locale` is told, which replaces everything set before: the
/// language, the formats when they differ from it, and whatever else was set by hand.
pub fn locale_args(current: &[(String, String)], language: &str, formats: Option<&str>) -> Vec<String> {
    let mut args = vec![format!("LANG={language}")];
    if let Some(formats) = formats.filter(|formats| *formats != language) {
        args.extend(FORMATS.iter().map(|key| format!("{key}={formats}")));
    }
    // An order of languages to try belongs to the language it was written for.
    let same_language = set_in(current, "LANG") == Some(language);
    for (key, value) in current {
        if key != "LANG" && !FORMATS.contains(&key.as_str()) && (key != "LANGUAGE" || same_language) {
            args.push(format!("{key}={value}"));
        }
    }
    args
}

/// The UTF-8 locales the C library can make, from its list of them.
pub fn supported(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let code = words.next().filter(|code| !code.starts_with('#'))?;
            (words.next()? == "UTF-8").then(|| code.to_string())
        })
        .collect()
}

/// A locale's name as `locale -a` and the lists write it differently: without the
/// case and the dash, `en_GB.UTF-8` and `en_GB.utf8` are the same.
fn plain(code: &str) -> String {
    code.to_lowercase().replace('-', "")
}

/// A locale in words, from the start of its definition: "French · France".
pub fn describe(code: &str, definition: &str) -> String {
    let said = |key: &str| {
        definition.lines().find_map(|line| {
            // The word, then spaces or a tab, then the name in quotes.
            let rest = line.strip_prefix(key).filter(|rest| rest.starts_with(char::is_whitespace))?;
            let value = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
            // Older definitions spell letters out as numbers; those are left alone.
            (!value.is_empty() && !value.contains("<U")).then(|| value.to_string())
        })
    };
    let Some(language) = said("language") else { return code.to_string() };
    let variant = code.split_once('@').map(|(_, variant)| format!(" ({variant})")).unwrap_or_default();
    match said("territory") {
        Some(territory) => format!("{language} · {territory}{variant}"),
        None => format!("{language}{variant}"),
    }
}

/// Whether every word of what was typed is somewhere in a name.
pub fn matches(name: &str, typed: &str) -> bool {
    let name = name.to_lowercase().replace('_', " ");
    typed.to_lowercase().split_whitespace().all(|word| name.contains(&word.replace('_', " ")))
}

/// Whether a locale is already among those to make.
pub fn listed(to_make: &str, code: &str) -> bool {
    to_make.lines().any(|line| line.split_whitespace().next() == Some(code))
}

/// Makes a locale that is not on the machine yet. `locale-gen` makes what is listed;
/// where it takes a name instead, as on Ubuntu, it is given the name too.
pub fn make_steps(code: &str, listed: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if !listed {
        steps.push(Step { args: vec!["/usr/bin/tee".to_string(), "-a".to_string(), TO_MAKE.to_string()], input: Some(format!("{code} UTF-8\n")) });
    }
    steps.push(step(&["/usr/sbin/locale-gen", code]));
    steps
}

#[derive(Debug, Clone, PartialEq)]
struct Locale {
    code: String,
    name: String,
    /// Whether it is on the machine, or has to be made first.
    ready: bool,
}

/// What was read from the system, in the background.
#[derive(Default)]
struct Read {
    clock: Clock,
    locale: Vec<(String, String)>,
    zones: Vec<String>,
    locales: Vec<Locale>,
}

fn said(program: &str, args: &[&str]) -> String {
    Command::new(program).args(args).env("LC_ALL", "C").output().map(|output| String::from_utf8_lossy(&output.stdout).into_owned()).unwrap_or_default()
}

fn read_locales() -> Vec<Locale> {
    let ready: Vec<String> = said("locale", &["-a"]).lines().map(plain).collect();
    let mut codes = supported(&std::fs::read_to_string("/usr/share/i18n/SUPPORTED").unwrap_or_default());
    if codes.is_empty() {
        // No list of what could be made: offer what is there.
        codes = said("locale", &["-a"]).lines().filter(|code| code.contains("utf8")).map(|code| code.replace("utf8", "UTF-8")).collect();
    }
    let mut locales: Vec<Locale> = codes
        .into_iter()
        .map(|code| {
            // The language and the country are said near the top of the definition.
            let file = format!("/usr/share/i18n/locales/{}", code.replace(".UTF-8", ""));
            let top: String = std::fs::File::open(file)
                .map(|file| BufReader::new(file).lines().map_while(Result::ok).take(200).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            Locale { name: describe(&code, &top), ready: ready.contains(&plain(&code)), code }
        })
        .collect();
    locales.sort_by(|a, b| a.name.cmp(&b.name));
    locales
}

/// A change waiting for the password of whoever is making it.
struct Pending {
    /// What is about to happen, in words: "move to Europe/Paris".
    what: String,
    steps: Vec<Step>,
}

#[derive(Default)]
struct Outcome {
    working: bool,
    /// How the last change went: what to say, and whether it failed.
    said: Option<(String, bool)>,
    /// Set when something changed, so that the system is read again.
    changed: bool,
}

/// The date and region part of the settings.
pub struct Region {
    ctx: egui::Context,
    read: Arc<Mutex<Read>>,
    read_at: Option<Instant>,
    zone_typed: String,
    when_typed: String,
    language_typed: String,
    formats_typed: String,
    pending: Option<Pending>,
    my_password: String,
    outcome: Arc<Mutex<Outcome>>,
    /// What the fields say is wrong, before anything is run.
    trouble: Option<String>,
}

impl Region {
    pub fn new(ctx: egui::Context) -> Region {
        let read: Arc<Mutex<Read>> = Arc::default();
        // The lists do not change while the machine runs, and are long: read once.
        let (into, repaint) = (read.clone(), ctx.clone());
        std::thread::spawn(move || {
            let zones: Vec<String> = said(TIMEDATECTL, &["list-timezones"]).lines().map(str::to_string).collect();
            into.lock().unwrap().zones = zones;
            let locales = read_locales();
            into.lock().unwrap().locales = locales;
            repaint.request_repaint();
        });
        Region {
            ctx,
            read,
            read_at: None,
            zone_typed: String::new(),
            when_typed: String::new(),
            language_typed: String::new(),
            formats_typed: String::new(),
            pending: None,
            my_password: String::new(),
            outcome: Arc::default(),
            trouble: None,
        }
    }

    /// Reads how the clock and the language are set now.
    fn read(&mut self) {
        self.read_at = Some(Instant::now());
        let (into, ctx) = (self.read.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let clock = parse_clock(&said(TIMEDATECTL, &["show"]));
            let file = std::fs::read_to_string("/etc/locale.conf").or_else(|_| std::fs::read_to_string("/etc/default/locale")).unwrap_or_default();
            let mut read = into.lock().unwrap();
            read.clock = clock;
            read.locale = parse_locale(&file);
            drop(read);
            ctx.request_repaint();
        });
    }

    fn ask(&mut self, what: String, steps: Vec<Step>) {
        self.trouble = None;
        self.my_password.clear();
        self.outcome.lock().unwrap().said = None;
        self.pending = Some(Pending { what, steps });
    }

    /// Runs what is pending with the password given.
    fn go(&mut self) {
        let Some(pending) = self.pending.take() else { return };
        let password = std::mem::take(&mut self.my_password);
        let (outcome, ctx) = (self.outcome.clone(), self.ctx.clone());
        outcome.lock().unwrap().working = true;
        std::thread::spawn(move || {
            let sudo = std::env::var("DES_SUDO").unwrap_or_else(|_| "sudo".to_string());
            let result = run_as_admin(&sudo, &password, &pending.steps);
            let mut outcome = outcome.lock().unwrap();
            outcome.working = false;
            outcome.changed = true;
            outcome.said = Some(match result {
                Ok(()) => (format!("Done: {}.", pending.what), false),
                Err(why) => (format!("Could not {}. {why}", pending.what), true),
            });
            drop(outcome);
            ctx.request_repaint();
        });
    }

    /// Changes the language, the formats, or both, making first what is not there.
    fn ask_locale(&mut self, what: String, current: &[(String, String)], language: &str, formats: Option<&str>, locales: &[Locale]) {
        let to_make = std::fs::read_to_string(TO_MAKE).unwrap_or_default();
        let mut steps = Vec::new();
        for code in [Some(language), formats].into_iter().flatten() {
            // One that is not on any list was set by hand, and is taken as it stands.
            if locales.iter().any(|locale| locale.code == code && !locale.ready) {
                steps.extend(make_steps(code, listed(&to_make, code)));
            }
        }
        let mut args = vec![LOCALECTL.to_string(), "set-locale".to_string()];
        args.extend(locale_args(current, language, formats));
        steps.push(Step { args, input: None });
        self.ask(what, steps);
    }

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, twelve_hour: bool, changes: &mut Vec<Change>) {
        let (working, said) = {
            let mut outcome = self.outcome.lock().unwrap();
            if std::mem::take(&mut outcome.changed) {
                self.read_at = None;
                if outcome.said.as_ref().is_some_and(|(_, failed)| !failed) {
                    for typed in [&mut self.zone_typed, &mut self.when_typed, &mut self.language_typed, &mut self.formats_typed] {
                        typed.clear();
                    }
                }
            }
            (outcome.working, outcome.said.clone())
        };
        if self.read_at.is_none_or(|at| at.elapsed() > Duration::from_secs(5)) {
            self.read();
        }
        let (clock, locale, zones, locales) = {
            let read = self.read.lock().unwrap();
            let zones: Vec<String> = if self.zone_typed.trim().is_empty() { Vec::new() } else { read.zones.iter().filter(|zone| matches(zone, &self.zone_typed)).cloned().collect() };
            (read.clock.clone(), read.locale.clone(), zones, read.locales.clone())
        };

        ui::section(ui, "DATE & TIME", |ui| {
            if working {
                ui::meta(ui, "WORKING", theme.main);
            }
        });
        let now = chrono::Local::now();
        let seconds = now.format(if twelve_hour { "%-I:%M:%S %p" } else { "%H:%M:%S" });
        ui::text(ui, &format!("{}  ·  {seconds}  ·  {}", now.format("%A %d %B %Y"), clock.zone.replace('_', " ")), ui::mono(13.0), ui::TEXT);
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        ui.add_space(12.0);

        // A change waiting for a password comes first: nothing else can be done meanwhile.
        if let Some(what) = self.pending.as_ref().map(|pending| pending.what.clone()) {
            let mut decided = None;
            ui::tile(ui, theme, true, |ui| {
                ui::tracked(ui, &format!("TO {}, TYPE YOUR OWN PASSWORD", what.to_uppercase()), ui::display_bold(12.0), ui::TEXT, 1.5);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let entered = field(ui, theme, &mut self.my_password, "your password", true, 260.0);
                    let ready = !self.my_password.is_empty();
                    if (ui::button(ui, theme, "CONFIRM", None, ready).clicked() || entered) && ready {
                        decided = Some(true);
                    }
                    if ui::button(ui, theme, "CANCEL", None, false).clicked() {
                        decided = Some(false);
                    }
                });
            });
            match decided {
                Some(true) => self.go(),
                Some(false) => {
                    self.pending = None;
                    self.my_password.clear();
                }
                None => {}
            }
            return;
        }
        if let Some((text, failed)) = &said {
            ui::text(ui, text, ui::mono(12.0), if *failed { ui::CAUTION } else { theme.main });
            ui.add_space(10.0);
        }
        if let Some(trouble) = &self.trouble {
            ui::text(ui, trouble, ui::mono(12.0), ui::CAUTION);
            ui.add_space(10.0);
        }

        if clock.can_network {
            if switch_row(ui, theme, "Set the time from the network", "Time servers keep the clock right, and correct it when it drifts", clock.network) {
                let (verb, value) = if clock.network { ("stop setting", "false") } else { ("start setting", "true") };
                self.ask(format!("{verb} the time from the network"), vec![step(&[TIMEDATECTL, "set-ntp", value])]);
            }
            ui.add_space(8.0);
            if clock.network {
                note(ui, if clock.synced { "The clock agrees with the time servers." } else { "Waiting to hear from a time server." });
            }
        } else {
            note(ui, "No time service is installed here, so the clock is set by hand.");
        }
        if !clock.network {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let entered = field(ui, theme, &mut self.when_typed, &now.format("%Y-%m-%d %H:%M").to_string(), false, 220.0);
                let ready = !self.when_typed.trim().is_empty();
                if (ui::button(ui, theme, "SET THE CLOCK", None, ready).clicked() || entered) && ready {
                    match parse_when(&self.when_typed) {
                        Some(when) => self.ask(format!("set the clock to {}", &when[..16]), vec![step(&[TIMEDATECTL, "set-time", &when])]),
                        None => self.trouble = Some("Write the date and time as 2026-03-09 14:30, in 24 hours.".to_string()),
                    }
                }
            });
        }
        ui.add_space(22.0);

        heading(ui, "TIME ZONE", "Type part of a city or a region to find another.");
        ui.horizontal(|ui| field(ui, theme, &mut self.zone_typed, "city or region", false, 300.0));
        ui.add_space(8.0);
        let mut zone_picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for zone in zones.iter().take(OFFERED) {
                let chosen = *zone == clock.zone;
                if ui::chip(ui, theme, &zone.replace('_', " ").to_uppercase(), None, chosen).clicked() && !chosen {
                    zone_picked = Some(zone.clone());
                }
            }
        });
        if let Some(zone) = zone_picked {
            self.ask(format!("move to the time zone {}", zone.replace('_', " ")), vec![step(&[TIMEDATECTL, "set-timezone", &zone])]);
        }
        searched(ui, &self.zone_typed, zones.len(), "No time zone has that in its name.");
        ui.add_space(22.0);

        heading(ui, "CLOCK", "How the hours are counted on the desktop and the lock screen.");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (twelve, label) in [(false, "24 HOURS"), (true, "12 HOURS")] {
                let chosen = twelve_hour == twelve;
                if ui::chip(ui, theme, label, None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("clock", twelve.then(|| "12".to_string())));
                }
            }
        });
        ui.add_space(22.0);

        let language = set_in(&locale, "LANG").unwrap_or("C").to_string();
        let formats = set_in(&locale, "LC_TIME").filter(|formats| *formats != language).map(str::to_string);
        let name_of = |code: &str| locales.iter().find(|locale| plain(&locale.code) == plain(code)).map_or(code.to_string(), |locale| locale.name.clone());
        let found = |typed: &str| -> Vec<&Locale> {
            if typed.trim().is_empty() { Vec::new() } else { locales.iter().filter(|locale| matches(&format!("{} {}", locale.name, locale.code), typed)).collect() }
        };

        heading(ui, "LANGUAGE", &format!("What programs speak, where they know it. Now {}.", name_of(&language)));
        ui.horizontal(|ui| field(ui, theme, &mut self.language_typed, "language or country", false, 300.0));
        ui.add_space(8.0);
        let offered = found(&self.language_typed);
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for locale in offered.iter().take(OFFERED) {
                let chosen = plain(&locale.code) == plain(&language);
                if ui::chip(ui, theme, &locale.name.to_uppercase(), None, chosen).on_hover_text(locale.code.as_str()).clicked() && !chosen {
                    picked = Some((*locale).clone());
                }
            }
        });
        searched(ui, &self.language_typed, offered.len(), "No language has that in its name.");
        if let Some(to) = picked {
            self.ask_locale(format!("change the language to {}", to.name), &locale, &to.code, formats.as_deref(), &locales);
        }
        ui.add_space(22.0);

        let now_formats = formats.as_deref().map_or("the same as the language".to_string(), name_of);
        heading(ui, "FORMATS", &format!("How dates, numbers, money and paper sizes are written. Now {now_formats}."));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            field(ui, theme, &mut self.formats_typed, "language or country", false, 300.0);
        });
        ui.add_space(8.0);
        let offered = found(&self.formats_typed);
        // `None` is the language's own formats.
        let mut picked: Option<Option<Locale>> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let same = formats.is_none();
            if ui::chip(ui, theme, "SAME AS THE LANGUAGE", None, same).clicked() && !same {
                picked = Some(None);
            }
            for locale in offered.iter().take(OFFERED) {
                let chosen = formats.as_deref().is_some_and(|formats| plain(formats) == plain(&locale.code));
                if ui::chip(ui, theme, &locale.name.to_uppercase(), None, chosen).on_hover_text(locale.code.as_str()).clicked() && !chosen {
                    picked = Some(Some((*locale).clone()));
                }
            }
        });
        searched(ui, &self.formats_typed, offered.len(), "No language has that in its name.");
        match picked {
            Some(Some(to)) => self.ask_locale(format!("write dates and numbers as in {}", to.name), &locale, &language, Some(&to.code), &locales),
            Some(None) => self.ask_locale("write dates and numbers as the language does".to_string(), &locale, &language, None, &locales),
            None => {}
        }
        ui.add_space(14.0);
        note(ui, "A change of language or formats shows from the next time you sign in.");
        ui.add_space(4.0);
        note(ui, "One that is not on this machine yet is made first, which takes a moment.");
        ui.add_space(16.0);
    }
}

/// What to say under a search: that nothing was found, or that there is more.
fn searched(ui: &mut Ui, typed: &str, found: usize, nothing: &str) {
    if typed.trim().is_empty() {
        return;
    }
    if found == 0 {
        note(ui, nothing);
    } else if found > OFFERED {
        ui.add_space(8.0);
        note(ui, &format!("And {} more: type more to narrow them down.", found - OFFERED));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_how_the_clock_is_kept() {
        let shown = "Timezone=America/New_York\nLocalRTC=no\nCanNTP=yes\nNTP=no\nNTPSynchronized=no\nTimeUSec=Tue 2026-10-13 12:51:04 EDT\n";
        assert_eq!(parse_clock(shown), Clock { zone: "America/New_York".into(), network: false, can_network: true, synced: false });
        assert_eq!(parse_clock("NTP=yes\nNTPSynchronized=yes\n"), Clock { zone: String::new(), network: true, can_network: false, synced: true });
        assert_eq!(parse_clock(""), Clock::default());
    }

    #[test]
    fn takes_a_typed_date_and_time_or_refuses_it() {
        assert_eq!(parse_when(" 2026-03-09 14:30 ").as_deref(), Some("2026-03-09 14:30:00"));
        assert_eq!(parse_when("2026-03-09 14:30:15").as_deref(), Some("2026-03-09 14:30:15"));
        assert_eq!(parse_when("2026-02-30 10:00"), None, "no such day");
        assert_eq!(parse_when("2026-03-09 25:00"), None);
        assert_eq!(parse_when("tomorrow"), None);
        assert_eq!(parse_when("2026-03-09 14:30; reboot"), None);
    }

    #[test]
    fn writes_the_time_in_either_count_of_hours() {
        let at = |hour, minute| chrono::NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        assert_eq!(clock_text(at(13, 5), false, true), "13:05");
        assert_eq!(clock_text(at(13, 5), true, false), "1:05");
        assert_eq!(clock_text(at(13, 5), true, true), "1:05 PM");
        assert_eq!(clock_text(at(0, 30), true, true), "12:30 AM");
        assert_eq!(clock_text(at(0, 30), false, false), "00:30");
    }

    #[test]
    fn reads_the_locale_file_and_builds_what_replaces_it() {
        let current = parse_locale("# set by the installer\nLANG=\"en_US.UTF-8\"\nLANGUAGE=en_US:en\nLC_TIME=fr_FR.UTF-8\nLC_MESSAGES=C\nPATH=/bin\nLC_PAPER=\n");
        let pair = |key: &str, value: &str| (key.to_string(), value.to_string());
        assert_eq!(current, [pair("LANG", "en_US.UTF-8"), pair("LANGUAGE", "en_US:en"), pair("LC_TIME", "fr_FR.UTF-8"), pair("LC_MESSAGES", "C")]);

        // Formats that differ are all set; what was set by hand is kept.
        let args = locale_args(&current, "en_US.UTF-8", Some("de_DE.UTF-8"));
        assert_eq!(args[0], "LANG=en_US.UTF-8");
        assert_eq!(args.len(), 1 + FORMATS.len() + 2);
        assert!(args.contains(&"LC_TIME=de_DE.UTF-8".to_string()) && args.contains(&"LC_PAPER=de_DE.UTF-8".to_string()));
        assert!(args.contains(&"LANGUAGE=en_US:en".to_string()) && args.contains(&"LC_MESSAGES=C".to_string()));

        // The language's own formats are not written out, and the old ones go.
        assert_eq!(locale_args(&current, "en_US.UTF-8", None), ["LANG=en_US.UTF-8", "LANGUAGE=en_US:en", "LC_MESSAGES=C"]);
        assert_eq!(locale_args(&current, "en_US.UTF-8", Some("en_US.UTF-8")).len(), 3);
        // Another language leaves the old order of languages behind.
        assert_eq!(locale_args(&current, "fr_FR.UTF-8", None), ["LANG=fr_FR.UTF-8", "LC_MESSAGES=C"]);
        assert_eq!(locale_args(&[], "ja_JP.UTF-8", None), ["LANG=ja_JP.UTF-8"]);
    }

    #[test]
    fn lists_and_names_the_locales_that_can_be_made() {
        let list = "# comment\naa_ER UTF-8\nca_ES.UTF-8 UTF-8\nca_ES ISO-8859-1\nca_ES@valencia UTF-8\nen_GB.UTF-8 UTF-8\nen_GB.ISO-8859-15 ISO-8859-15\n";
        assert_eq!(supported(list), ["aa_ER", "ca_ES.UTF-8", "ca_ES@valencia", "en_GB.UTF-8"]);
        assert_eq!(plain("en_GB.UTF-8"), plain("en_GB.utf8"));
        assert_ne!(plain("en_GB.UTF-8"), plain("en_GB"));

        let definition = "LC_IDENTIFICATION\ntitle      \"English locale for Britain\"\nlanguage   \"British English\"\nterritory  \"United Kingdom\"\n";
        assert_eq!(describe("en_GB.UTF-8", definition), "British English · United Kingdom");
        assert_eq!(describe("sr_RS@latin", "language \"Serbian\"\nterritory \"Serbia\"\n"), "Serbian · Serbia (latin)");
        assert_eq!(describe("bi_VU", "language\t\"Bislama\"\nterritory\t\"Vanuatu\"\nlang_name \"Bislama\"\n"), "Bislama · Vanuatu");
        assert_eq!(describe("eo", "language \"Esperanto\"\nterritory \"\"\n"), "Esperanto");
        assert_eq!(describe("xx_YY.UTF-8", ""), "xx_YY.UTF-8");
        assert_eq!(describe("xx_YY.UTF-8", "language \"<U0041>\"\n"), "xx_YY.UTF-8");
    }

    #[test]
    fn finds_names_by_any_words_typed() {
        assert!(matches("America/New_York", "new york"));
        assert!(matches("America/New_York", "YORK america"));
        assert!(matches("America/New_York", "New_York"));
        assert!(!matches("America/New_York", "paris"));
        assert!(matches("British English · United Kingdom en_GB.UTF-8", "en_gb"));
        assert!(matches("anything", "  "));
    }

    #[test]
    fn makes_a_locale_by_listing_it_once_and_generating() {
        let to_make = "# en_GB.UTF-8 UTF-8\nen_US.UTF-8 UTF-8\n";
        assert!(listed(to_make, "en_US.UTF-8"));
        assert!(!listed(to_make, "en_GB.UTF-8"), "commented out");
        assert!(!listed(to_make, "en_US"));

        let steps = make_steps("fr_FR.UTF-8", false);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].args, ["/usr/bin/tee", "-a", "/etc/locale.gen"]);
        assert_eq!(steps[0].input.as_deref(), Some("fr_FR.UTF-8 UTF-8\n"));
        assert_eq!(steps[1].args, ["/usr/sbin/locale-gen", "fr_FR.UTF-8"]);
        assert_eq!(make_steps("fr_FR.UTF-8", true).len(), 1);
    }
}
