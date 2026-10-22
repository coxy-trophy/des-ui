//! The calendar under the clock: a month, today picked out in the accent, and what
//! the chosen day holds.
//!
//! Events come from `.ics` files in `~/.config/des-ui/calendars/` and from the
//! `webcal://` or `https://` addresses listed one per line in
//! `~/.config/des-ui/calendars.list`, fetched with `curl` at most every half hour and
//! kept in `~/.cache/des-ui/calendars/`. Evolution's data server is not read.
//!
//! The parser knows enough of RFC 5545 for what calendars publish: VEVENT with its
//! start and end (dates, local times and UTC), summary and location, folded lines and
//! escaped text, and repetition by day, week (on given weekdays), month or year with
//! an interval, a count or an end, less the dates struck out with EXDATE. Times given
//! in a named zone (TZID) are taken as this machine's local time.

use crate::notify::Center;
use crate::theme::Theme;
use crate::ui;
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use eframe::egui::{self, Align, Area, Id, Key, Layout, Modifiers, Order, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, SystemTime};

/// No calendar file, local or fetched, is read past this.
const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Nor more than this many local files.
const MAX_FILES: usize = 64;
/// Nor more events kept from all of them together.
const MAX_EVENTS: usize = 20_000;
/// A fetched calendar is asked for again after this.
const FETCH_EVERY: StdDuration = StdDuration::from_secs(30 * 60);
/// How far ahead an event is announced.
const WARN_AHEAD: i64 = 15;
/// No rule is followed past this many steps, whatever it says.
const MAX_STEPS: usize = 20_000;

/// When something happens, as the file says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// A whole day.
    Date(NaiveDate),
    /// A time on this machine's clock (also what a TZID time is taken as).
    Local(NaiveDateTime),
    /// A time in UTC, ending in `Z`.
    Utc(NaiveDateTime),
}

impl When {
    /// The moment on this machine's clock; a whole day starts at midnight.
    pub fn local(self) -> NaiveDateTime {
        match self {
            When::Date(date) => date.and_time(NaiveTime::MIN),
            When::Local(time) => time,
            When::Utc(time) => Utc.from_utc_datetime(&time).with_timezone(&Local).naive_local(),
        }
    }

    fn all_day(self) -> bool {
        matches!(self, When::Date(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<When>,
    /// For weekly rules: the weekdays it falls on.
    pub by_day: Vec<Weekday>,
}

/// One VEVENT, as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub summary: String,
    pub location: String,
    pub start: When,
    pub end: Option<When>,
    pub rule: Option<Rule>,
    pub except: Vec<When>,
}

/// One time an event happens.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub summary: String,
    pub location: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
}

/// Joins the lines a long property was folded into.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.chars().next() {
            Some(' ' | '\t') if !lines.is_empty() => lines.last_mut().unwrap().push_str(&line[1..]),
            _ => lines.push(line.to_string()),
        }
    }
    lines
}

/// Undoes the escapes TEXT values carry.
pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A DATE or DATE-TIME value; `date_only` when VALUE=DATE says so.
pub fn parse_when(value: &str, date_only: bool) -> Option<When> {
    let value = value.trim();
    if date_only || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(When::Date);
    }
    if let Some(utc) = value.strip_suffix('Z') {
        return NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok().map(When::Utc);
    }
    NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok().map(When::Local)
}

fn weekday(code: &str) -> Option<Weekday> {
    // A position such as `1MO` or `-1FR` only matters for monthly rules, which are
    // followed here by the day of the month; the weekday is what is kept.
    let code = code.trim_start_matches(|c: char| c == '+' || c == '-' || c.is_ascii_digit());
    Some(match code {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

pub fn parse_rule(value: &str) -> Option<Rule> {
    let mut rule = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: Vec::new() };
    let mut freq = None;
    for part in value.split(';') {
        let Some((key, value)) = part.split_once('=') else { continue };
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    // Hourly and finer repetition is not shown in a day's agenda.
                    _ => return None,
                })
            }
            "INTERVAL" => rule.interval = value.trim().parse::<u32>().ok().filter(|n| *n > 0)?,
            "COUNT" => rule.count = value.trim().parse().ok(),
            "UNTIL" => rule.until = parse_when(value, false),
            "BYDAY" => rule.by_day = value.split(',').filter_map(|day| weekday(day.trim())).collect(),
            _ => {}
        }
    }
    rule.freq = freq?;
    Some(rule)
}

/// Splits `NAME;PARAM=..:VALUE` into its name, whether VALUE=DATE is among the
/// parameters, and the value.
fn property(line: &str) -> Option<(String, bool, &str)> {
    let colon = {
        // A colon inside a quoted parameter is not the separator.
        let mut quoted = false;
        line.char_indices().find(|(_, c)| {
            if *c == '"' {
                quoted = !quoted;
            }
            *c == ':' && !quoted
        })?.0
    };
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let date_only = parts.any(|param| param.trim().eq_ignore_ascii_case("VALUE=DATE"));
    Some((name, date_only, value))
}

/// The events in a calendar file. Those without a start are left out.
pub fn parse(text: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<(Option<When>, Option<When>, Option<Duration>, String, String, Option<Rule>, Vec<When>)> = None;
    for line in unfold(text) {
        let Some((name, date_only, value)) = property(&line) else { continue };
        match name.as_str() {
            "BEGIN" if value.trim().eq_ignore_ascii_case("VEVENT") => {
                current = Some((None, None, None, String::new(), String::new(), None, Vec::new()))
            }
            "END" if value.trim().eq_ignore_ascii_case("VEVENT") => {
                if let Some((Some(start), end, duration, summary, location, rule, except)) = current.take() {
                    let end = end.or_else(|| {
                        let duration = duration?;
                        Some(match start {
                            When::Date(date) => When::Date(date + duration),
                            When::Local(time) => When::Local(time + duration),
                            When::Utc(time) => When::Utc(time + duration),
                        })
                    });
                    events.push(Event { summary, location, start, end, rule, except });
                    if events.len() >= MAX_EVENTS {
                        break;
                    }
                }
            }
            _ => {
                let Some(event) = current.as_mut() else { continue };
                match name.as_str() {
                    "DTSTART" => event.0 = parse_when(value, date_only),
                    "DTEND" => event.1 = parse_when(value, date_only),
                    "DURATION" => event.2 = parse_duration(value),
                    "SUMMARY" => event.3 = unescape(value).trim().to_string(),
                    "LOCATION" => event.4 = unescape(value).trim().to_string(),
                    "RRULE" => event.5 = parse_rule(value),
                    "EXDATE" => event.6.extend(value.split(',').filter_map(|one| parse_when(one, date_only))),
                    _ => {}
                }
            }
        }
    }
    events
}

/// `P1D`, `PT1H30M`, `P2W` and the like.
pub fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (negative, value) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let mut rest = value.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut in_time = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('T') {
            in_time = true;
            rest = after;
            continue;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        let number: i64 = rest[..digits].parse().ok()?;
        let unit = rest[digits..].chars().next()?;
        total += match (unit, in_time) {
            ('W', false) => Duration::weeks(number),
            ('D', false) => Duration::days(number),
            ('H', true) => Duration::hours(number),
            ('M', true) => Duration::minutes(number),
            ('S', true) => Duration::seconds(number),
            _ => return None,
        };
        rest = &rest[digits + unit.len_utf8()..];
    }
    Some(if negative { -total } else { total })
}

/// Adds months, keeping the day; `None` where that day does not exist.
fn add_months(date: NaiveDate, months: i64) -> Option<NaiveDate> {
    let index = date.year() as i64 * 12 + date.month0() as i64 + months;
    NaiveDate::from_ymd_opt(index.div_euclid(12) as i32, index.rem_euclid(12) as u32 + 1, date.day())
}

/// The starts of an event's occurrences, in order, from its first, while `keep_going`
/// says so. Counts and ends are honoured; struck-out dates are left to the caller.
fn starts(event: &Event, mut keep_going: impl FnMut(NaiveDateTime) -> bool) -> Vec<NaiveDateTime> {
    let first = event.start.local();
    let Some(rule) = &event.rule else {
        return vec![first];
    };
    let until = rule.until.map(When::local);
    let mut found = Vec::new();
    let mut counted = 0u32;
    let interval = rule.interval as i64;
    let mut step = 0usize;
    // Each candidate is one more in sequence; returns false to stop.
    let mut take = |at: NaiveDateTime, found: &mut Vec<NaiveDateTime>| -> bool {
        if at < first {
            return true;
        }
        if until.is_some_and(|until| at > until) || rule.count.is_some_and(|count| counted >= count) || !keep_going(at) {
            return false;
        }
        counted += 1;
        found.push(at);
        true
    };
    let time = first.time();
    match rule.freq {
        Freq::Weekly if !rule.by_day.is_empty() => {
            let mut days = rule.by_day.clone();
            days.sort_by_key(|day| day.num_days_from_monday());
            days.dedup();
            // Weeks start on Monday unless the rule says otherwise; WKST is not read.
            let week = first.date() - Duration::days(first.weekday().num_days_from_monday() as i64);
            'weeks: for n in 0.. {
                let monday = week + Duration::weeks(n * interval);
                for day in &days {
                    step += 1;
                    if step > MAX_STEPS || !take((monday + Duration::days(day.num_days_from_monday() as i64)).and_time(time), &mut found) {
                        break 'weeks;
                    }
                }
            }
        }
        freq => {
            for n in 0.. {
                step += 1;
                if step > MAX_STEPS {
                    break;
                }
                let date = match freq {
                    Freq::Daily => Some(first.date() + Duration::days(n * interval)),
                    Freq::Weekly => Some(first.date() + Duration::weeks(n * interval)),
                    Freq::Monthly => add_months(first.date(), n * interval),
                    Freq::Yearly => add_months(first.date(), n * interval * 12),
                };
                // The 31st of a short month, or the 29th of February out of a leap
                // year, simply does not happen that time round.
                let Some(date) = date else { continue };
                if !take(date.and_time(time), &mut found) {
                    break;
                }
            }
        }
    }
    found
}

/// Every occurrence that overlaps `from..to`, earliest first.
pub fn expand(events: &[Event], from: NaiveDateTime, to: NaiveDateTime) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for event in events {
        let all_day = event.start.all_day();
        let length = match event.end {
            Some(end) => (end.local() - event.start.local()).max(Duration::zero()),
            None if all_day => Duration::days(1),
            None => Duration::zero(),
        };
        let except: HashSet<NaiveDateTime> = event.except.iter().map(|when| when.local()).collect();
        let except_days: HashSet<NaiveDate> = event.except.iter().filter(|when| when.all_day()).map(|when| when.local().date()).collect();
        for start in starts(event, |at| at < to) {
            // A single event is not stopped by the rule's window; it is here.
            if start >= to || except.contains(&start) || (all_day && except_days.contains(&start.date())) {
                continue;
            }
            let end = start + length;
            // A moment's event counts on the day it happens.
            if end > from || (length.is_zero() && start >= from) {
                out.push(Occurrence { summary: event.summary.clone(), location: event.location.clone(), start, end, all_day });
            }
        }
    }
    out.sort_by(|a, b| (!a.all_day, a.start).cmp(&(!b.all_day, b.start)));
    out
}

/// The weeks of a month, each seven days from `first_day`; days of the neighbouring
/// months are `None`.
pub fn month_grid(year: i32, month: u32, first_day: Weekday) -> Vec<[Option<NaiveDate>; 7]> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else { return Vec::new() };
    let offset = (first.weekday().num_days_from_monday() + 7 - first_day.num_days_from_monday()) % 7;
    let mut weeks = Vec::new();
    let mut day = first - Duration::days(offset as i64);
    loop {
        let mut week = [None; 7];
        for slot in &mut week {
            if day.month() == month && day.year() == year {
                *slot = Some(day);
            }
            day += Duration::days(1);
        }
        weeks.push(week);
        if day.month() != month || day.year() != year {
            break;
        }
    }
    weeks
}

/// Where the week starts for a locale such as `en_US.UTF-8`: Sunday in the countries
/// whose calendars put it first, Monday elsewhere.
pub fn first_weekday(locale: &str) -> Weekday {
    let territory = locale.split(['.', '@']).next().unwrap_or("").split('_').nth(1).unwrap_or("");
    const SUNDAY_FIRST: [&str; 16] = ["US", "CA", "JP", "IL", "BR", "MX", "PH", "KR", "TW", "HK", "ZA", "IN", "SA", "PE", "CO", "AR"];
    if SUNDAY_FIRST.contains(&territory) { Weekday::Sun } else { Weekday::Mon }
}

fn locale_now() -> String {
    ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty())).unwrap_or_default()
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))).map(|dir| dir.join("des-ui"))
}

fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))).map(|dir| dir.join("des-ui/calendars"))
}

/// The addresses in `calendars.list`, `webcal://` read as `https://`; anything else
/// is left out.
pub fn parse_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter_map(|line| {
            if let Some(rest) = line.strip_prefix("webcal://") {
                Some(format!("https://{rest}"))
            } else if line.starts_with("https://") {
                Some(line.to_string())
            } else {
                None
            }
        })
        .filter(|url| !url.chars().any(|c| c.is_whitespace() || c.is_control()))
        .collect()
}

/// A file name for an address's cached copy.
fn cache_name(url: &str) -> String {
    // FNV-1a: stable across runs, which std's hasher is not.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in url.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}.ics")
}

fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Fetches `url` into `path` when the copy there is older than half an hour.
fn refresh(url: &str, path: &Path) {
    let fresh = std::fs::metadata(path).and_then(|meta| meta.modified()).ok().and_then(|at| SystemTime::now().duration_since(at).ok()).is_some_and(|age| age < FETCH_EVERY);
    if fresh {
        return;
    }
    let partial = path.with_extension("part");
    let fetched = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "10", "--max-filesize", &MAX_BYTES.to_string(), "--proto", "=https", "--proto-redir", "=https", "-o"])
        .arg(&partial)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let small = std::fs::metadata(&partial).is_ok_and(|meta| meta.len() <= MAX_BYTES);
    if fetched.is_ok_and(|status| status.success()) && small {
        let _ = std::fs::rename(&partial, path);
    } else {
        let _ = std::fs::remove_file(&partial);
        // Tried, if not got: do not ask again on every pass.
        if let Ok(file) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = file.set_modified(SystemTime::now());
        }
    }
}

/// Reads every calendar there is, fetching what is due.
fn gather() -> Vec<Event> {
    let mut events = Vec::new();
    if let Some(dir) = config_dir() {
        if let Ok(entries) = std::fs::read_dir(dir.join("calendars")) {
            let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ics"))).collect();
            files.sort();
            for file in files.into_iter().take(MAX_FILES) {
                if let Some(text) = read_bounded(&file) {
                    events.extend(parse(&text));
                }
            }
        }
        let urls = read_bounded(&dir.join("calendars.list")).map(|text| parse_list(&text)).unwrap_or_default();
        if let (false, Some(cache)) = (urls.is_empty(), cache_dir()) {
            if std::fs::create_dir_all(&cache).is_ok() {
                for url in urls.iter().take(MAX_FILES) {
                    let path = cache.join(cache_name(url));
                    refresh(url, &path);
                    if let Some(text) = read_bounded(&path) {
                        events.extend(parse(&text));
                    }
                }
            }
        }
    }
    events.truncate(MAX_EVENTS);
    events
}

/// The pop-up and what it knows.
pub struct Agenda {
    pub open: bool,
    /// The month in view, by its first day.
    month: NaiveDate,
    selected: NaiveDate,
    anchor: Pos2,
    events: Arc<Mutex<Vec<Event>>>,
    /// Waking the reader early; dropping it ends the reader.
    _wake: Option<Sender<()>>,
    /// Occurrences already announced.
    announced: HashSet<(String, NaiveDateTime)>,
    last_check: Option<std::time::Instant>,
    /// The frame it opened in, so the click that opened it does not close it.
    opened_at: u64,
}

impl Agenda {
    /// Starts reading the calendars on a thread of its own, now and every minute.
    pub fn start(ctx: egui::Context) -> Agenda {
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let (wake, woken) = mpsc::channel::<()>();
        let shared = events.clone();
        let started = std::thread::Builder::new().name("des-ui-calendars".into()).spawn(move || loop {
            let found = gather();
            *shared.lock().unwrap() = found;
            ctx.request_repaint();
            match woken.recv_timeout(StdDuration::from_secs(60)) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        });
        let today = Local::now().date_naive();
        Agenda {
            open: false,
            month: today.with_day(1).unwrap_or(today),
            selected: today,
            anchor: Pos2::ZERO,
            events,
            _wake: started.is_ok().then_some(wake),
            announced: HashSet::new(),
            last_check: None,
            opened_at: 0,
        }
    }

    /// Opens on today below `anchor`, or closes.
    pub fn toggle(&mut self, ctx: &egui::Context, anchor: Pos2) {
        self.open = !self.open;
        if self.open {
            let today = Local::now().date_naive();
            self.selected = today;
            self.month = today.with_day(1).unwrap_or(today);
            self.anchor = anchor;
            self.opened_at = ctx.cumulative_pass_nr();
            if let Some(wake) = &self._wake {
                let _ = wake.send(());
            }
        }
    }

    /// Announces, once, what starts within the next quarter of an hour.
    pub fn pump(&mut self, notices: &mut Center) {
        if self.last_check.is_some_and(|at| at.elapsed() < StdDuration::from_secs(30)) {
            return;
        }
        self.last_check = Some(std::time::Instant::now());
        let now = Local::now().naive_local();
        for (title, body, key) in due(&self.events.lock().unwrap(), now, &self.announced) {
            notices.post(&title, &body, false);
            self.announced.insert(key);
        }
        self.announced.retain(|(_, start)| *start > now - Duration::days(1));
    }

    /// Draws the pop-up when open; returns where, for the compositor to lift it above
    /// applications.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Rect> {
        if !self.open {
            return None;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return None;
        }
        let today = Local::now().date_naive();
        let first_day = first_weekday(&locale_now());
        let events = self.events.lock().unwrap().clone();
        let month_from = self.month.and_time(NaiveTime::MIN);
        let month_to = add_months(self.month, 1).unwrap_or(self.month).and_time(NaiveTime::MIN);
        let in_month = expand(&events, month_from, month_to);
        let busy: HashSet<NaiveDate> = in_month.iter().flat_map(|o| {
            let last = if o.all_day { o.end - Duration::seconds(1) } else { o.end };
            let mut days = Vec::new();
            let mut day = o.start.date();
            while day <= last.date().max(o.start.date()) && days.len() < 62 {
                days.push(day);
                day += Duration::days(1);
            }
            days
        }).collect();
        let day_from = self.selected.and_time(NaiveTime::MIN);
        let on_day = expand(&events, day_from, day_from + Duration::days(1));

        let area = Area::new(Id::new("agenda")).order(Order::Foreground).fixed_pos(self.anchor).show(ctx, |ui| {
            crate::material::surface(ui, egui::Frame::new().corner_radius(ui::SURFACE_RADIUS).stroke(Stroke::new(1.0, ui::LINE_STRONG)).inner_margin(14.0), crate::material::Role::Floating, |ui| {
                let cell = vec2(34.0, 26.0);
                ui.set_width(cell.x * 7.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                ui.horizontal(|ui| {
                    if ui::button(ui, theme, "‹", None, false).on_hover_text("Previous month").clicked() {
                        self.month = add_months(self.month, -1).unwrap_or(self.month);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui::button(ui, theme, "›", None, false).on_hover_text("Next month").clicked() {
                            self.month = add_months(self.month, 1).unwrap_or(self.month);
                        }
                        ui.with_layout(Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                            let title = self.month.format("%B %Y").to_string().to_uppercase();
                            ui::tracked(ui, &title, ui::display_bold(13.0), ui::TEXT, 2.0);
                        });
                    });
                });
                ui.horizontal(|ui| {
                    for n in 0..7 {
                        let day = weekday_after(first_day, n);
                        let (rect, _) = ui.allocate_exact_size(cell, Sense::hover());
                        let name = day.to_string().chars().take(2).collect::<String>().to_uppercase();
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, name, ui::mono(10.0), ui::LABEL);
                    }
                });
                for week in month_grid(self.month.year(), self.month.month(), first_day) {
                    ui.horizontal(|ui| {
                        for slot in week {
                            let (rect, response) = ui.allocate_exact_size(cell, Sense::click());
                            let Some(date) = slot else { continue };
                            let painter = ui.painter();
                            let is_today = date == today;
                            if is_today {
                                painter.rect_filled(rect.shrink(2.0), 0.0, theme.main);
                            } else if response.hovered() {
                                painter.rect_filled(rect.shrink(2.0), 0.0, ui::TRACK);
                            }
                            if date == self.selected {
                                painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0, if is_today { ui::TEXT } else { theme.main }), StrokeKind::Inside);
                            }
                            let ink = if is_today { ui::GROUND } else { ui::TEXT };
                            painter.text(rect.center() - vec2(0.0, 2.0), egui::Align2::CENTER_CENTER, date.day().to_string(), ui::mono(12.0), ink);
                            if busy.contains(&date) {
                                painter.circle_filled(pos(rect), 1.8, if is_today { ui::GROUND } else { theme.main });
                            }
                            if response.clicked() {
                                self.selected = date;
                            }
                        }
                    });
                }
                ui.add_space(8.0);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, ui::TRACK));
                ui.add_space(8.0);
                let heading = self.selected.format("%A %-d %B").to_string().to_uppercase();
                ui::tracked(ui, &heading, ui::display_bold(11.0), theme.main, 2.0);
                if on_day.is_empty() {
                    ui::text(ui, "Nothing on this day.", ui::mono(12.0), ui::LABEL);
                }
                for occurrence in on_day.iter().take(12) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let when = if occurrence.all_day { "ALL DAY".to_string() } else { span(occurrence, self.selected) };
                        ui::text(ui, &when, ui::mono(11.0), ui::SECONDARY);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui::text(ui, &crate::panels::truncate(&occurrence.summary, 28), ui::mono(12.0), ui::TEXT);
                            if !occurrence.location.is_empty() {
                                ui::text(ui, &crate::panels::truncate(&occurrence.location, 28), ui::mono(11.0), ui::LABEL);
                            }
                        });
                    });
                }
                if on_day.len() > 12 {
                    ui::text(ui, &format!("+{} more", on_day.len() - 12), ui::mono(11.0), ui::LABEL);
                }
            });
        });
        let rect = area.response.rect;
        // A click anywhere else closes it, though not the click that opened it.
        let clicked_outside = ctx.input(|input| input.pointer.any_pressed() && input.pointer.interact_pos().is_some_and(|at| !rect.contains(at)));
        if clicked_outside && ctx.cumulative_pass_nr() > self.opened_at + 1 {
            self.open = false;
        }
        Some(rect)
    }
}

fn pos(rect: Rect) -> Pos2 {
    egui::pos2(rect.center().x, rect.bottom() - 4.0)
}

fn weekday_after(first: Weekday, n: u32) -> Weekday {
    let mut day = first;
    for _ in 0..n {
        day = day.succ();
    }
    day
}

/// `09:00–10:30`, in the clock the user chose; times beyond the day are marked.
fn span(occurrence: &Occurrence, day: NaiveDate) -> String {
    let clock = |at: NaiveDateTime| {
        let shown = Local.from_local_datetime(&at).single().map(|at| crate::region::hours_minutes(&at)).unwrap_or_else(|| format!("{:02}:{:02}", at.hour(), at.minute()));
        if at.date() == day { shown } else { format!("{shown}*") }
    };
    if occurrence.end == occurrence.start { clock(occurrence.start) } else { format!("{}–{}", clock(occurrence.start), clock(occurrence.end)) }
}

/// What to announce at `now`: timed events starting within the next quarter hour and
/// not yet announced, as (title, body, key).
pub fn due(events: &[Event], now: NaiveDateTime, announced: &HashSet<(String, NaiveDateTime)>) -> Vec<(String, String, (String, NaiveDateTime))> {
    expand(events, now, now + Duration::minutes(WARN_AHEAD))
        .into_iter()
        .filter(|o| !o.all_day && o.start > now && o.start <= now + Duration::minutes(WARN_AHEAD))
        .filter(|o| !announced.contains(&(o.summary.clone(), o.start)))
        .map(|o| {
            let minutes = ((o.start - now).num_seconds() + 59) / 60;
            let title = if o.summary.is_empty() { "An event".to_string() } else { o.summary.clone() };
            let mut body = format!("In {minutes} minute{}.", if minutes == 1 { "" } else { "s" });
            if !o.location.is_empty() {
                body.push_str(&format!(" {}", o.location));
            }
            (title, body, (o.summary, o.start))
        })
        .collect()
}

/// Asks for the pop-up from wherever the clock is drawn; the shell opens it below
/// `anchor` on its next pass.
pub fn request(ctx: &egui::Context, anchor: Pos2) {
    ctx.data_mut(|data| data.insert_temp(Id::new("agenda-request"), anchor));
}

/// The request made this frame, if any.
pub fn take_request(ctx: &egui::Context) -> Option<Pos2> {
    ctx.data_mut(|data| data.remove_temp::<Pos2>(Id::new("agenda-request")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        date(y, m, d).and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn reads_folded_escaped_all_day_and_timed_events() {
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Planning\\, review \\; notes\\nsecond\r\n line\r\nDTSTART;VALUE=DATE:20261020\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Call\r\nLOCATION:Room \\\\ 4\r\nDTSTART;TZID=\"Europe/Paris:x\":20261021T090000\r\nDTEND:20261021T093000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Standup\r\nDTSTART:20261022T080000Z\r\nDURATION:PT15M\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:No start\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse(text);
        assert_eq!(events.len(), 3, "an event without a start is left out");
        assert_eq!(events[0].summary, "Planning, review ; notes\nsecondline");
        assert_eq!(events[0].start, When::Date(date(2026, 10, 20)));
        assert_eq!(events[1].start, When::Local(at(2026, 10, 21, 9, 0)), "a TZID time is taken as local; a quoted colon is not the separator");
        assert_eq!(events[1].location, "Room \\ 4");
        assert_eq!(events[2].start, When::Utc(at(2026, 10, 22, 8, 0)));
        assert_eq!(events[2].end, Some(When::Utc(at(2026, 10, 22, 8, 15))));
        // UTC is moved onto this machine's clock.
        let local = Utc.from_utc_datetime(&at(2026, 10, 22, 8, 0)).with_timezone(&Local).naive_local();
        assert_eq!(events[2].start.local(), local);
    }

    #[test]
    fn all_day_events_last_their_day_and_moments_count_on_theirs() {
        let events = parse("BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261020\nSUMMARY:Holiday\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20261021T120000\nSUMMARY:Ping\nEND:VEVENT\n");
        let day = expand(&events, at(2026, 10, 20, 0, 0), at(2026, 10, 21, 0, 0));
        assert_eq!(day.len(), 1);
        assert!(day[0].all_day);
        assert_eq!(day[0].end, at(2026, 10, 21, 0, 0));
        assert!(expand(&events, at(2026, 10, 21, 0, 0), at(2026, 10, 22, 0, 0)).iter().any(|o| o.summary == "Ping"));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("P2W"), Some(Duration::weeks(2)));
        assert_eq!(parse_duration("-PT5M"), Some(Duration::minutes(-5)));
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_duration("PT1D"), None);
    }

    fn event(rule: &str, start: When) -> Event {
        Event { summary: "x".into(), location: String::new(), start, end: None, rule: parse_rule(rule), except: Vec::new() }
    }

    fn days(occurrences: &[Occurrence]) -> Vec<u32> {
        occurrences.iter().map(|o| o.start.day()).collect()
    }

    #[test]
    fn daily_with_interval_count_and_until() {
        let start = When::Local(at(2026, 10, 1, 9, 0));
        let month = (at(2026, 10, 1, 0, 0), at(2026, 11, 1, 0, 0));
        assert_eq!(days(&expand(&[event("FREQ=DAILY;INTERVAL=3;COUNT=4", start)], month.0, month.1)), [1, 4, 7, 10]);
        assert_eq!(days(&expand(&[event("FREQ=DAILY;UNTIL=20261005T090000", start)], month.0, month.1)), [1, 2, 3, 4, 5]);
        // A count is counted from the first occurrence, not from the window.
        assert_eq!(days(&expand(&[event("FREQ=DAILY;COUNT=5", start)], at(2026, 10, 4, 0, 0), month.1)), [4, 5]);
        assert!(parse_rule("FREQ=HOURLY").is_none());
        assert!(parse_rule("FREQ=DAILY;INTERVAL=0").is_none());
    }

    #[test]
    fn weekly_on_given_days_with_struck_out_dates() {
        let mut e = event("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH", When::Local(at(2026, 10, 1, 9, 0)));
        // 1 October 2026 is a Thursday.
        let found = expand(&[e.clone()], at(2026, 10, 1, 0, 0), at(2026, 11, 1, 0, 0));
        assert_eq!(days(&found), [1, 13, 15, 27, 29]);
        e.except.push(When::Local(at(2026, 10, 13, 9, 0)));
        assert_eq!(days(&expand(&[e], at(2026, 10, 1, 0, 0), at(2026, 11, 1, 0, 0))), [1, 15, 27, 29]);
        let plain = event("FREQ=WEEKLY;COUNT=3", When::Date(date(2026, 10, 2)));
        assert_eq!(days(&expand(&[plain], at(2026, 10, 1, 0, 0), at(2026, 11, 1, 0, 0))), [2, 9, 16]);
    }

    #[test]
    fn monthly_and_yearly_skip_days_that_do_not_exist() {
        let monthly = event("FREQ=MONTHLY;COUNT=4", When::Date(date(2026, 1, 31)));
        let months: Vec<u32> = expand(&[monthly], at(2026, 1, 1, 0, 0), at(2027, 1, 1, 0, 0)).iter().map(|o| o.start.month()).collect();
        assert_eq!(months, [1, 3, 5, 7], "no 31st in February, April or June");
        let yearly = event("FREQ=YEARLY", When::Date(date(2024, 2, 29)));
        let years: Vec<i32> = expand(&[yearly], at(2024, 1, 1, 0, 0), at(2033, 1, 1, 0, 0)).iter().map(|o| o.start.year()).collect();
        assert_eq!(years, [2024, 2028, 2032]);
        let mut struck = event("FREQ=YEARLY;UNTIL=20290101", When::Date(date(2026, 5, 1)));
        struck.except.push(When::Date(date(2027, 5, 1)));
        let years: Vec<i32> = expand(&[struck], at(2026, 1, 1, 0, 0), at(2035, 1, 1, 0, 0)).iter().map(|o| o.start.year()).collect();
        assert_eq!(years, [2026, 2028]);
    }

    #[test]
    fn a_rule_without_end_stops_at_the_window() {
        let endless = event("FREQ=DAILY", When::Local(at(2000, 1, 1, 9, 0)));
        let found = expand(&[endless], at(2026, 10, 1, 0, 0), at(2026, 10, 2, 0, 0));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn month_grids_for_every_first_weekday_and_leap_years() {
        // March 2026 starts on a Sunday; Monday-first weeks need a sixth row.
        let march = month_grid(2026, 3, Weekday::Mon);
        assert_eq!(march.len(), 6);
        assert_eq!(march[0][6], Some(date(2026, 3, 1)));
        assert_eq!(march[0][..6], [None; 6]);
        assert_eq!(month_grid(2026, 3, Weekday::Sun).len(), 5);
        assert_eq!(month_grid(2026, 3, Weekday::Sun)[0][0], Some(date(2026, 3, 1)));
        // February 2026 starts on a Sunday and has 28 days: four rows when Sunday is first.
        assert_eq!(month_grid(2026, 2, Weekday::Sun).len(), 4);
        // A leap February.
        let feb = month_grid(2028, 2, Weekday::Mon);
        let count = feb.iter().flatten().flatten().count();
        assert_eq!(count, 29);
        assert_eq!(month_grid(2027, 2, Weekday::Mon).iter().flatten().flatten().count(), 28);
        // Every month of a year, from every first weekday: each day once, in order.
        for first in [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun] {
            for month in 1..=12 {
                let grid = month_grid(2026, month, first);
                let all: Vec<NaiveDate> = grid.iter().flatten().flatten().copied().collect();
                assert_eq!(all.first().unwrap().day(), 1);
                assert!(all.windows(2).all(|pair| pair[1] - pair[0] == Duration::days(1)));
                for week in &grid {
                    for (n, slot) in week.iter().enumerate() {
                        if let Some(day) = slot {
                            assert_eq!(day.weekday(), weekday_after(first, n as u32));
                        }
                    }
                }
            }
        }
        assert!(month_grid(2026, 13, Weekday::Mon).is_empty());
    }

    #[test]
    fn the_week_starts_where_the_locale_says() {
        assert_eq!(first_weekday("en_US.UTF-8"), Weekday::Sun);
        assert_eq!(first_weekday("en_GB.UTF-8"), Weekday::Mon);
        assert_eq!(first_weekday("de_DE@euro"), Weekday::Mon);
        assert_eq!(first_weekday("ja_JP"), Weekday::Sun);
        assert_eq!(first_weekday("C"), Weekday::Mon);
    }

    #[test]
    fn calendar_list_accepts_only_https_and_webcal() {
        let list = "# mine\nwebcal://example.org/a.ics\nhttps://example.org/b.ics  # work\nhttp://example.org/c.ics\nfile:///etc/passwd\n\nhttps://bad host\n";
        assert_eq!(parse_list(list), ["https://example.org/a.ics", "https://example.org/b.ics"]);
        assert_eq!(cache_name("https://a"), cache_name("https://a"));
        assert_ne!(cache_name("https://a"), cache_name("https://b"));
    }

    #[test]
    fn announces_what_starts_soon_once() {
        let events = parse("BEGIN:VEVENT\nDTSTART:20261021T091000\nSUMMARY:Call\nLOCATION:Room 4\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20261021T100000\nSUMMARY:Later\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20261021\nSUMMARY:All day\nEND:VEVENT\n");
        let now = at(2026, 10, 21, 9, 0);
        let mut announced = HashSet::new();
        let found = due(&events, now, &announced);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "Call");
        assert_eq!(found[0].1, "In 10 minutes. Room 4");
        announced.insert(found[0].2.clone());
        assert!(due(&events, now, &announced).is_empty(), "once only");
    }
}
