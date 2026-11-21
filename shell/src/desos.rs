//! What DesOS says about itself, read for the desktop: whether the system matches
//! its signed build record (`desos-verify`), and the history of system snapshots
//! (`desos-undo`). Everything here is read through the documented interfaces only:
//! `/run/desos/verify.json` (schema `desos-verify-v1`) and `desos-undo --json`
//! (schema 1).
//!
//! Nothing in this file runs on the interface thread. One worker reads files and
//! runs the tool; the interface only looks at what the worker last stored. On a
//! system that is not DesOS the worker stops at once and nothing is ever shown.
//!
//! Drawing lives in `desos_view.rs`.

use eframe::egui;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// The verify result is a few kilobytes; anything far larger is not that file.
const MAX_VERIFY_BYTES: u64 = 1 << 20;
/// The most output read from one run of the tool.
const MAX_OUTPUT_BYTES: usize = 4 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// A diff walks two trees.
const DIFF_TIMEOUT: Duration = Duration::from_secs(90);
/// Includes the time the user takes to type a password.
const PRIVILEGED_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the verify result is read again; the full check rewrites it a while
/// after the session has started.
const VERIFY_EVERY: Duration = Duration::from_secs(30);
const MAX_CHECKS: usize = 128;
const MAX_SNAPSHOTS: usize = 1000;
const MAX_LISTED: usize = 2000;

/// The reason recorded with a snapshot taken from Settings. Fixed, so that nothing
/// typed in the desktop becomes an argument of a command run as root.
pub const SNAPSHOT_REASON: &str = "Taken from DEs-UI settings";
pub const ROLLBACK_REASON: &str = "Chosen in DEs-UI settings";

/// What a pass does not mean, said wherever a result is explained.
pub const CAVEAT: &str = "This is a consistency check made by software on this machine. It does not show that the system is secure: someone with administrator access can change the check, or the key it trusts.";

static PRESENT: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
thread_local! { static PRESENT_IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// True once this machine is known to be DesOS. Everything DesOS-only asks this
/// before it offers itself.
pub fn present() -> bool {
    #[cfg(test)]
    { PRESENT_IN_TEST.with(|flag| flag.get()) }
    #[cfg(not(test))]
    { PRESENT.load(Ordering::Relaxed) }
}

#[cfg(test)]
pub fn set_present_for_test(present: bool) { PRESENT_IN_TEST.with(|flag| flag.set(present)); }

/// Where things are. Tests point these at a temporary directory.
#[derive(Clone, Debug)]
pub struct Paths {
    pub os_release: PathBuf,
    pub verify: PathBuf,
    pub boot_id: PathBuf,
    pub undo: PathBuf,
    pub pkexec: PathBuf,
}

impl Paths {
    /// The real places. A debug build can be pointed at a directory of stand-ins
    /// with DES_UI_DESOS_FIXTURE, to look at these surfaces on a machine that is
    /// not DesOS; a release build never reads it, so nothing in the environment can
    /// change which program is run as root.
    pub fn system() -> Paths {
        if cfg!(debug_assertions) {
            if let Some(dir) = std::env::var_os("DES_UI_DESOS_FIXTURE").map(PathBuf::from).filter(|dir| dir.is_absolute()) {
                return Paths { os_release: dir.join("os-release"), verify: dir.join("verify.json"), boot_id: dir.join("boot_id"), undo: dir.join("desos-undo"), pkexec: dir.join("pkexec") };
            }
        }
        Paths {
            os_release: "/etc/os-release".into(),
            verify: "/run/desos/verify.json".into(),
            boot_id: "/proc/sys/kernel/random/boot_id".into(),
            undo: "/usr/sbin/desos-undo".into(),
            pkexec: "/usr/bin/pkexec".into(),
        }
    }
}

/// True for an os-release whose `ID` is `desos`. `ID_LIKE` does not count: plain
/// Ubuntu with DEs-UI installed from the apt repository is not DesOS.
pub fn is_desos(os_release: &str) -> bool {
    os_release.lines().any(|line| {
        let Some(value) = line.trim().strip_prefix("ID=") else { return false };
        value.trim().trim_matches(['"', '\'']) == "desos"
    })
}

enum ReadFailure { Absent, TooLarge, Other(String) }

/// Reads a regular file of at most `max` bytes.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, ReadFailure> {
    let from = |error: std::io::Error| if error.kind() == std::io::ErrorKind::NotFound { ReadFailure::Absent } else { ReadFailure::Other(error.to_string()) };
    // Anything but a regular file (a pipe would never end) is not read.
    if !std::fs::metadata(path).map_err(from)?.is_file() { return Err(ReadFailure::Other("not a regular file".into())); }
    let file = std::fs::File::open(path).map_err(from)?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(from)?;
    if bytes.len() as u64 > max { return Err(ReadFailure::TooLarge); }
    Ok(bytes)
}

/// Text from the system made safe to draw: no control characters, and no longer
/// than `max` characters.
fn clean(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).map(|c| if c.is_control() { ' ' } else { c }).collect();
    if text.chars().count() > max { out.push('…'); }
    out.trim().to_string()
}

fn text_of(value: &Value, key: &str, max: usize) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(|text| clean(text, max))
}
fn number_of(value: &Value, key: &str) -> Option<u64> { value.get(key).and_then(Value::as_u64) }
fn flag_of(value: &Value, key: &str) -> bool { value.get(key).and_then(Value::as_bool).unwrap_or(false) }

// ---------------------------------------------------------------- verify

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Pass, Unknown, Fail }

impl Verdict {
    /// Anything that is not exactly `pass` or `fail` is unknown.
    fn read(word: Option<&str>) -> Verdict {
        match word { Some("pass") => Verdict::Pass, Some("fail") => Verdict::Fail, _ => Verdict::Unknown }
    }
    fn worst(self, other: Verdict) -> Verdict {
        match (self, other) {
            (Verdict::Fail, _) | (_, Verdict::Fail) => Verdict::Fail,
            (Verdict::Unknown, _) | (_, Verdict::Unknown) => Verdict::Unknown,
            _ => Verdict::Pass,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check { pub id: String, pub status: Verdict, pub reason: String }

#[derive(Clone, Debug, PartialEq)]
pub struct LastFull { pub overall: Verdict, pub finished: Option<String>, pub failed: Vec<String>, pub same_manifest: bool }

/// One result of `desos-verify`, as far as the desktop uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct Verify {
    /// The worse of what the result says overall and what its checks say.
    pub overall: Verdict,
    /// False for the quick check, which looks at much less.
    pub full: bool,
    pub summary: String,
    pub finished: Option<String>,
    pub checks: Vec<Check>,
    pub key_fingerprint: Option<String>,
    /// True when the result names another start of the machine than this one.
    pub stale: bool,
    pub last_full: Option<LastFull>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Integrity {
    /// No result yet since this start.
    NotRun,
    /// A result exists but could not be used; why, in a sentence.
    Unreadable(String),
    Result(Verify),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone { Plain, Quiet, Caution }

/// The one line of the system panel: a few words, how to colour them, and the
/// explanation shown on demand.
#[derive(Clone, Debug, PartialEq)]
pub struct Line { pub value: &'static str, pub tone: Tone, pub detail: String }

/// Reads a `desos-verify-v1` result. `boot_id` is this start's id, when known.
pub fn parse_verify(bytes: &[u8], boot_id: Option<&str>) -> Integrity {
    if bytes.len() as u64 > MAX_VERIFY_BYTES {
        return Integrity::Unreadable("The stored check result is larger than expected and was not read.".into());
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Integrity::Unreadable("The stored check result could not be read.".into());
    };
    if !value.is_object() {
        return Integrity::Unreadable("The stored check result could not be read.".into());
    }
    match value.get("schema").and_then(Value::as_str) {
        Some("desos-verify-v1") => {}
        other => {
            let named = other.map_or("unnamed".to_string(), |name| clean(name, 40));
            return Integrity::Unreadable(format!("The stored check result is in a format this desktop does not know ({named})."));
        }
    }
    let checks: Vec<Check> = value.get("checks").and_then(Value::as_array).map(|list| {
        list.iter().filter(|item| item.is_object()).take(MAX_CHECKS).map(|item| Check {
            id: text_of(item, "id", 80).unwrap_or_else(|| "unnamed".into()),
            status: Verdict::read(item.get("status").and_then(Value::as_str)),
            reason: text_of(item, "reason", 300).unwrap_or_default(),
        }).collect()
    }).unwrap_or_default();
    let stated = Verdict::read(value.get("overall").and_then(Value::as_str));
    let overall = checks.iter().fold(stated, |worst, check| worst.worst(check.status));
    let theirs = value.get("boot_id").and_then(Value::as_str);
    let last_full = value.get("last_full").filter(|full| full.is_object()).map(|full| LastFull {
        overall: Verdict::read(full.get("overall").and_then(Value::as_str)),
        finished: text_of(full, "finished", 40),
        failed: full.get("failed").and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).take(MAX_CHECKS).map(|id| clean(id, 80)).collect()).unwrap_or_default(),
        same_manifest: flag_of(full, "same_manifest"),
    });
    Integrity::Result(Verify {
        overall,
        // An unrecognised mode is given no more credit than the quick one.
        full: value.get("mode").and_then(Value::as_str) == Some("full"),
        summary: text_of(&value, "summary", 240).unwrap_or_default(),
        finished: text_of(&value, "finished", 40),
        checks,
        key_fingerprint: text_of(&value, "key_fingerprint", 80),
        stale: matches!((theirs, boot_id), (Some(theirs), Some(ours)) if theirs.trim() != ours.trim()),
        last_full,
    })
}

/// Reads the result file. Absent means the check has not run yet this start.
pub fn read_verify(paths: &Paths) -> Integrity {
    let boot_id = read_capped(&paths.boot_id, 256).ok().map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string());
    match read_capped(&paths.verify, MAX_VERIFY_BYTES) {
        Ok(bytes) => parse_verify(&bytes, boot_id.as_deref()),
        Err(ReadFailure::Absent) => Integrity::NotRun,
        Err(ReadFailure::TooLarge) => Integrity::Unreadable("The stored check result is larger than expected and was not read.".into()),
        Err(ReadFailure::Other(why)) => Integrity::Unreadable(format!("The stored check result could not be read ({}).", clean(&why, 120))),
    }
}

impl Verify {
    /// A full check that failed against the same signed record, reported by a
    /// later quick check. The quick check does not look at what the full one found.
    fn failed_full(&self) -> Option<&LastFull> {
        self.last_full.as_ref().filter(|full| !self.full && full.same_manifest && full.overall == Verdict::Fail)
    }
    fn passed_full(&self) -> bool {
        self.full || self.last_full.as_ref().is_some_and(|full| full.same_manifest && full.overall == Verdict::Pass)
    }
    fn reasons(&self, status: Verdict) -> Vec<String> {
        self.checks.iter().filter(|check| check.status == status).map(|check| format!("{}: {}", check.id, check.reason)).collect()
    }
}

impl Integrity {
    pub fn line(&self) -> Line {
        let (value, tone, mut detail): (&'static str, Tone, Vec<String>) = match self {
            Integrity::NotRun => ("NOT CHECKED", Tone::Quiet, vec!["The check against the signed build record has not run yet since this start.".into()]),
            Integrity::Unreadable(why) => ("UNREADABLE", Tone::Quiet, vec![why.clone()]),
            Integrity::Result(verify) if verify.stale => ("UNCONFIRMED", Tone::Quiet, vec!["The stored check result is from an earlier start of this machine, so it says nothing about this one.".into()]),
            Integrity::Result(verify) => {
                let finished = verify.finished.as_deref().map(when).map_or(String::new(), |finished| format!(", {finished}"));
                let kind = if verify.full { "Full check" } else { "Quick check" };
                match verify.overall {
                    Verdict::Fail => {
                        let mut lines = vec![format!("Differs from the signed build record. {kind}{finished}.")];
                        lines.extend(verify.reasons(Verdict::Fail));
                        ("DIFFERS", Tone::Caution, lines)
                    }
                    _ if verify.failed_full().is_some() => {
                        let full = verify.failed_full().expect("checked above");
                        let at = full.finished.as_deref().map(when).map_or(String::new(), |when| format!(" ({when})"));
                        let mut lines = vec![format!("Differs from the signed build record: the last full check{at} failed, and the quick check since does not look at the same things.")];
                        if !full.failed.is_empty() { lines.push(format!("Failed: {}", full.failed.join(", "))); }
                        ("DIFFERS", Tone::Caution, lines)
                    }
                    Verdict::Unknown => {
                        let mut lines = vec![format!("Could not be confirmed against the signed build record. {kind}{finished}.")];
                        lines.extend(verify.reasons(Verdict::Unknown));
                        ("UNCONFIRMED", Tone::Quiet, lines)
                    }
                    Verdict::Pass if verify.full => ("MATCHES", Tone::Plain, vec![format!(
                        "Matches the signed build record: the installed DesOS packages, kernel image, kernel configuration and release identity are the ones that were signed. {kind}{finished}.")]),
                    Verdict::Pass if verify.passed_full() => ("MATCHES", Tone::Plain, vec![format!(
                        "Matches the signed build record. {kind}{finished}; the last full check, which also re-reads every installed DesOS file, matched too.")]),
                    Verdict::Pass => ("QUICK MATCH", Tone::Plain, vec![format!(
                        "Matches the signed build record as far as the quick check looks: signatures, package versions and release identity. Installed files are compared by the full check, which has not reported yet. {kind}{finished}.")]),
                }
            }
        };
        detail.push(CAVEAT.to_string());
        Line { value, tone, detail: detail.join("\n") }
    }
}

/// `2026-11-12T10:03:22Z` as a local day and time; anything else as it came.
pub fn when(stamp: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(stamp) {
        Ok(time) => time.with_timezone(&chrono::Local).format("%-d %b %Y %H:%M").to_string(),
        Err(_) => clean(stamp, 40),
    }
}

// ---------------------------------------------------------------- undo

/// Why the tool gave no usable answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    Absent,
    Timeout,
    Oversized,
    Malformed,
    UnknownSchema(String),
    /// The user did not authorise the action, or closed the question.
    Denied,
    Failed(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Absent => f.write_str("The DesOS undo tool is not installed on this system."),
            Problem::Timeout => f.write_str("The DesOS undo tool did not answer in time."),
            Problem::Oversized => f.write_str("The DesOS undo tool answered with more than this desktop reads."),
            Problem::Malformed => f.write_str("The DesOS undo tool gave an answer this desktop could not read."),
            Problem::UnknownSchema(schema) => write!(f, "The DesOS undo tool answers in a format (schema {schema}) this desktop does not know. Update DEs-UI."),
            Problem::Denied => f.write_str("Not authorised. Nothing was changed."),
            Problem::Failed(why) => write!(f, "The DesOS undo tool could not be run: {why}"),
        }
    }
}

/// One answer of `desos-undo --json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub ok: bool,
    pub supported: bool,
    pub code: Option<String>,
    pub message: String,
    pub data: Value,
}

/// The answer is the last line of standard output.
pub fn parse_envelope(stdout: &[u8]) -> Result<Envelope, Problem> {
    if stdout.len() > MAX_OUTPUT_BYTES { return Err(Problem::Oversized); }
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().rev().find(|line| !line.trim().is_empty()).ok_or(Problem::Malformed)?;
    let value: Value = serde_json::from_str(line.trim()).map_err(|_| Problem::Malformed)?;
    if !value.is_object() { return Err(Problem::Malformed); }
    match value.get("schema") {
        Some(schema) if schema.as_u64() == Some(1) => {}
        Some(schema) => return Err(Problem::UnknownSchema(clean(&schema.to_string(), 24))),
        None => return Err(Problem::Malformed),
    }
    let (Some(ok), Some(supported)) = (value.get("ok").and_then(Value::as_bool), value.get("supported").and_then(Value::as_bool)) else {
        return Err(Problem::Malformed);
    };
    Ok(Envelope {
        ok,
        supported,
        code: text_of(&value, "code", 40),
        message: text_of(&value, "message", 2000).unwrap_or_default(),
        data: value.get("data").filter(|data| data.is_object()).cloned().unwrap_or_else(|| Value::Object(Default::default())),
    })
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PackageChange { pub name: String, pub version: Option<String>, pub from: Option<String>, pub to: Option<String> }

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Packages { pub installed: Vec<PackageChange>, pub removed: Vec<PackageChange>, pub upgraded: Vec<PackageChange>, pub downgraded: Vec<PackageChange> }

impl Packages {
    fn read(value: Option<&Value>) -> Option<Packages> {
        let value = value.filter(|value| value.is_object())?;
        let list = |key: &str| -> Vec<PackageChange> {
            value.get(key).and_then(Value::as_array).map(|items| items.iter().take(MAX_LISTED).filter_map(|item| Some(PackageChange {
                name: text_of(item, "name", 120)?,
                version: text_of(item, "version", 80),
                from: text_of(item, "from", 80),
                to: text_of(item, "to", 80),
            })).collect()).unwrap_or_default()
        };
        Some(Packages { installed: list("installed"), removed: list("removed"), upgraded: list("upgraded"), downgraded: list("downgraded") })
    }

    pub fn is_empty(&self) -> bool {
        self.installed.is_empty() && self.removed.is_empty() && self.upgraded.is_empty() && self.downgraded.is_empty()
    }

    /// "upgraded nano 7.2-1 → 8.0-1 · installed 3 packages".
    pub fn summary(&self) -> String {
        let part = |verb: &str, changes: &[PackageChange]| match changes {
            [] => None,
            [one] => Some(format!("{verb} {}", one.describe())),
            [one, two] => Some(format!("{verb} {} and {}", one.name, two.name)),
            many => Some(format!("{verb} {} packages", many.len())),
        };
        [part("upgraded", &self.upgraded), part("installed", &self.installed), part("removed", &self.removed), part("downgraded", &self.downgraded)]
            .into_iter().flatten().collect::<Vec<_>>().join(" · ")
    }

    /// One line for each change, for the diff view.
    pub fn lines(&self) -> Vec<String> {
        let each = |verb: &'static str, changes: &[PackageChange]| changes.iter().map(move |change| format!("{verb} {}", change.describe())).collect::<Vec<_>>();
        [each("Upgraded", &self.upgraded), each("Downgraded", &self.downgraded), each("Installed", &self.installed), each("Removed", &self.removed)].concat()
    }
}

impl PackageChange {
    fn describe(&self) -> String {
        match (&self.from, &self.to, &self.version) {
            (Some(from), Some(to), _) => format!("{} {from} → {to}", self.name),
            (_, _, Some(version)) => format!("{} {version}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Pre, Post, Manual, BootGood, Rollback, Unknown }

impl Kind {
    fn read(word: Option<&str>) -> Kind {
        match word { Some("pre") => Kind::Pre, Some("post") => Kind::Post, Some("manual") => Kind::Manual, Some("boot-good") => Kind::BootGood, Some("rollback") => Kind::Rollback, _ => Kind::Unknown }
    }
    /// When the snapshot was taken, in the words of the list.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Pre => "BEFORE A CHANGE",
            Kind::Post => "AFTER A CHANGE",
            Kind::Manual => "TAKEN BY HAND",
            Kind::BootGood => "GOOD START",
            Kind::Rollback => "REPLACED BY GOING BACK",
            Kind::Unknown => "SNAPSHOT",
        }
    }
}

/// One snapshot, from its `info.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub id: u64,
    pub kind: Kind,
    pub reason: String,
    pub created: Option<String>,
    pub kernel: Option<String>,
    pub packages: Option<Packages>,
    pub planned: Option<Packages>,
    pub protected: bool,
}

impl Snapshot {
    /// None for an entry without a usable id; the rest is optional.
    fn read(value: &Value) -> Option<Snapshot> {
        Some(Snapshot {
            id: number_of(value, "id").filter(|id| *id >= 1)?,
            kind: Kind::read(value.get("kind").and_then(Value::as_str)),
            reason: text_of(value, "reason", 300).unwrap_or_default(),
            created: text_of(value, "created", 40),
            kernel: text_of(value, "kernel", 80),
            packages: Packages::read(value.get("packages")),
            planned: Packages::read(value.get("planned")),
            protected: flag_of(value, "protected"),
        })
    }

    /// What changed, from the snapshot's own record: what the package database
    /// shows, or failing that what apt announced.
    pub fn what_changed(&self) -> String {
        match (&self.packages, &self.planned) {
            (Some(actual), _) if !actual.is_empty() => actual.summary(),
            (_, Some(planned)) if !planned.is_empty() => format!("planned: {}", planned.summary()),
            _ => String::new(),
        }
    }

    /// "#14 · AFTER A CHANGE · 12 Nov 2026 10:03".
    pub fn crumb(&self) -> String {
        let mut parts = vec![format!("#{}", self.id), self.kind.label().to_string()];
        if let Some(created) = &self.created { parts.push(when(created).to_uppercase()); }
        if self.protected { parts.push("KEPT".into()); }
        parts.join(" · ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rollback { pub target: u64, pub undo: Option<u64>, pub time: Option<String>, pub automatic: bool }

/// `desos-undo status`, as far as the desktop uses it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Status {
    /// A rollback was made and the old system is still running.
    pub restart_pending: bool,
    /// The machine was started read-only from this snapshot.
    pub booted_snapshot: Option<u64>,
    pub rollback: Option<Rollback>,
    pub good_id: Option<u64>,
}

impl Status {
    fn read(data: &Value) -> Status {
        let root = data.get("root");
        Status {
            restart_pending: root.is_some_and(|root| flag_of(root, "restart_pending")),
            booted_snapshot: root.and_then(|root| number_of(root, "booted_snapshot")),
            rollback: data.get("rollback").filter(|rollback| rollback.is_object()).and_then(|rollback| Some(Rollback {
                target: number_of(rollback, "target")?,
                undo: number_of(rollback, "undo"),
                time: text_of(rollback, "time", 40),
                automatic: flag_of(rollback, "automatic"),
            })),
            good_id: data.get("boot").and_then(|boot| number_of(boot, "good_id")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Diff {
    pub from: u64,
    /// "current", or another snapshot's number.
    pub to: String,
    pub paths: Vec<String>,
    pub packages: Packages,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    /// The tool shortened the file lists.
    pub truncated: bool,
}

impl Diff {
    fn read(data: &Value) -> Option<Diff> {
        let strings = |value: Option<&Value>| -> Vec<String> {
            value.and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).take(MAX_LISTED).map(|item| clean(item, 400)).collect()).unwrap_or_default()
        };
        let files = data.get("files");
        let listed = |key: &str| strings(files.and_then(|files| files.get(key)));
        let count = |key: &str| files.and_then(|files| files.get(key)).and_then(Value::as_array).map_or(0, Vec::len);
        Some(Diff {
            from: number_of(data, "from")?,
            to: match data.get("to") { Some(Value::Number(id)) => id.to_string(), _ => "current".into() },
            paths: strings(data.get("paths")),
            packages: Packages::read(data.get("packages")).unwrap_or_default(),
            added: listed("added"),
            removed: listed("removed"),
            modified: listed("modified"),
            truncated: files.is_some_and(|files| flag_of(files, "truncated")) || ["added", "removed", "modified"].iter().any(|key| count(key) > MAX_LISTED),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty() && self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice { pub id: u64, pub kind: String, pub message: String, pub acknowledged: bool }

/// The notices of an answer that the user has not yet acknowledged.
pub fn unread_notices(envelope: &Envelope) -> Vec<Notice> {
    if !envelope.ok || !envelope.supported { return Vec::new(); }
    envelope.data.get("notices").and_then(Value::as_array).map(|items| items.iter().take(50).filter_map(|item| Some(Notice {
        id: number_of(item, "id")?,
        kind: text_of(item, "kind", 40).unwrap_or_default(),
        message: text_of(item, "message", 1200).filter(|message| !message.is_empty())?,
        acknowledged: flag_of(item, "acknowledged"),
    })).filter(|notice| !notice.acknowledged).collect()).unwrap_or_default()
}

/// Something asked for, and what came of it.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Fetch<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    /// Undo does not work here (live session, ext4, …); the tool's own reason.
    Unsupported(String),
    Failed(String),
}

fn fetched<T>(answer: Result<Envelope, Problem>, read: impl FnOnce(&Value) -> Option<T>) -> Fetch<T> {
    match answer {
        Err(problem) => Fetch::Failed(problem.to_string()),
        Ok(envelope) if !envelope.supported => Fetch::Unsupported(if envelope.message.is_empty() { "System snapshots are not available on this installation.".into() } else { envelope.message }),
        Ok(envelope) if !envelope.ok => Fetch::Failed(if envelope.message.is_empty() { "The DesOS undo tool reported a failure.".into() } else { envelope.message }),
        Ok(envelope) => read(&envelope.data).map_or(Fetch::Failed(Problem::Malformed.to_string()), Fetch::Ready),
    }
}

pub fn status_from(answer: Result<Envelope, Problem>) -> Fetch<Status> { fetched(answer, |data| Some(Status::read(data))) }

/// The snapshots, newest first. An entry that cannot be read is left out.
pub fn snapshots_from(answer: Result<Envelope, Problem>) -> Fetch<Vec<Snapshot>> {
    fetched(answer, |data| {
        let mut snapshots: Vec<Snapshot> = data.get("snapshots")?.as_array()?.iter().take(MAX_SNAPSHOTS).filter_map(Snapshot::read).collect();
        snapshots.sort_by(|a, b| b.id.cmp(&a.id));
        Some(snapshots)
    })
}

pub fn diff_from(answer: Result<Envelope, Problem>) -> Fetch<Diff> { fetched(answer, Diff::read) }

/// What a privileged action came to: whether it was done, and what to tell the user.
pub fn outcome_from(answer: Result<Envelope, Problem>) -> (bool, String) {
    match answer {
        Err(problem) => (false, problem.to_string()),
        Ok(envelope) => {
            let done = envelope.ok && envelope.supported;
            let fallback = if done { "Done." } else { "Nothing was changed." };
            (done, if envelope.message.is_empty() { fallback.to_string() } else { envelope.message })
        }
    }
}

// ---------------------------------------------------------------- commands

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Status,
    List,
    Diff(u64),
    Notices,
    AckNotices,
    /// As root, through pkexec.
    Snapshot,
    /// As root, through pkexec. Takes effect at the next start.
    Rollback(u64),
}

impl Action {
    pub fn privileged(self) -> bool { matches!(self, Action::Snapshot | Action::Rollback(_)) }
    fn timeout(self) -> Duration {
        match self { Action::Diff(_) => DIFF_TIMEOUT, action if action.privileged() => PRIVILEGED_TIMEOUT, _ => READ_TIMEOUT }
    }
}

/// The program and its arguments for an action. Always a fixed argument list with
/// at most a snapshot number in it; never a shell string.
pub fn command(paths: &Paths, action: Action) -> (PathBuf, Vec<String>) {
    let undo = paths.undo.to_string_lossy().into_owned();
    let words = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    match action {
        Action::Status => (paths.undo.clone(), words(&["--json", "status"])),
        Action::List => (paths.undo.clone(), words(&["--json", "list"])),
        Action::Diff(id) => (paths.undo.clone(), words(&["--json", "diff", &id.to_string()])),
        Action::Notices => (paths.undo.clone(), words(&["--json", "notices"])),
        Action::AckNotices => (paths.undo.clone(), words(&["--json", "notices", "--ack"])),
        Action::Snapshot => (paths.pkexec.clone(), words(&[&undo, "--json", "snapshot", "--reason", SNAPSHOT_REASON])),
        Action::Rollback(id) => (paths.pkexec.clone(), words(&[&undo, "--json", "rollback", &id.to_string(), "--reason", ROLLBACK_REASON])),
    }
}

pub struct Ran { pub code: Option<i32>, pub stdout: Vec<u8> }

/// Runs a program and collects at most `MAX_OUTPUT_BYTES` of its output within
/// `timeout`. A program that overruns either is killed when `kill` is set; one
/// running as root through pkexec is left to finish, since stopping pkexec would
/// not stop it and half a rollback is worse than a slow one.
pub fn run(program: &Path, arguments: &[String], timeout: Duration, kill: bool) -> Result<Ran, Problem> {
    let mut child = Command::new(program).args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { Problem::Absent } else { Problem::Failed(error.to_string()) })?;
    let Some(pipe) = child.stdout.take() else { return Err(Problem::Failed("no output pipe".into())); };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.take(MAX_OUTPUT_BYTES as u64 + 1).read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    let deadline = Instant::now() + timeout;
    let give_up = |mut child: std::process::Child, problem: Problem| -> Problem {
        if kill { let _ = child.kill(); }
        // Reaped off this thread, whenever it ends.
        std::thread::spawn(move || { let _ = child.wait(); });
        problem
    };
    let stdout = match receiver.recv_timeout(timeout) {
        Ok(bytes) => bytes,
        Err(_) => return Err(give_up(child, Problem::Timeout)),
    };
    if stdout.len() > MAX_OUTPUT_BYTES { return Err(give_up(child, Problem::Oversized)); }
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Ran { code: status.code(), stdout }),
            Ok(None) if Instant::now() >= deadline => return Err(give_up(child, Problem::Timeout)),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(give_up(child, Problem::Failed(error.to_string()))),
        }
    }
}

/// Runs one action and reads its answer.
pub fn ask(paths: &Paths, action: Action) -> Result<Envelope, Problem> {
    let (program, arguments) = command(paths, action);
    let ran = run(&program, &arguments, action.timeout(), !action.privileged())?;
    if action.privileged() {
        // pkexec's own exit statuses: 126 not authorised or dismissed, 127 could not ask.
        match ran.code {
            Some(126) => return Err(Problem::Denied),
            Some(127) if ran.stdout.is_empty() => return Err(Problem::Failed("authorisation could not be requested".into())),
            _ => {}
        }
    }
    parse_envelope(&ran.stdout).map_err(|problem| match (problem, ran.code) {
        (Problem::Malformed, Some(code)) if code != 0 && ran.stdout.iter().all(u8::is_ascii_whitespace) => Problem::Failed(format!("it ended with status {code}")),
        (problem, _) => problem,
    })
}

// ---------------------------------------------------------------- worker

#[derive(Default)]
struct Shared {
    present: bool,
    integrity: Option<Integrity>,
    status: Fetch<Status>,
    snapshots: Fetch<Vec<Snapshot>>,
    diff: Option<(u64, Fetch<Diff>)>,
    /// The privileged action under way.
    working: Option<Action>,
    /// How the last privileged action ended.
    outcome: Option<(bool, String)>,
    /// Notices read at session start, waiting to be shown.
    notices: Vec<Notice>,
}

/// What Settings draws from, copied out so nothing is held while drawing.
#[derive(Clone, Default)]
pub struct View {
    pub integrity: Option<Integrity>,
    pub status: Fetch<Status>,
    pub snapshots: Fetch<Vec<Snapshot>>,
    pub diff: Option<(u64, Fetch<Diff>)>,
    pub working: Option<Action>,
    pub outcome: Option<(bool, String)>,
}

/// Does one action and stores what came of it.
fn perform(paths: &Paths, shared: &Mutex<Shared>, action: Action) {
    let store = |change: &mut dyn FnMut(&mut Shared)| change(&mut shared.lock().unwrap());
    match action {
        Action::Status => { let status = status_from(ask(paths, action)); store(&mut |shared| shared.status = status.clone()); }
        Action::List => { let snapshots = snapshots_from(ask(paths, action)); store(&mut |shared| shared.snapshots = snapshots.clone()); }
        Action::Diff(id) => { let diff = diff_from(ask(paths, action)); store(&mut |shared| shared.diff = Some((id, diff.clone()))); }
        Action::Notices => {
            let notices = ask(paths, action).map(|envelope| unread_notices(&envelope)).unwrap_or_default();
            store(&mut |shared| shared.notices = notices.clone());
        }
        Action::AckNotices => { let _ = ask(paths, action); }
        Action::Snapshot | Action::Rollback(_) => {
            let outcome = outcome_from(ask(paths, action));
            store(&mut |shared| { shared.outcome = Some(outcome.clone()); });
            // What is listed must be what is there now.
            perform(paths, shared, Action::Status);
            perform(paths, shared, Action::List);
            store(&mut |shared| shared.working = None);
        }
    }
}

/// The worker: finds out whether this is DesOS, and if so keeps the verify result
/// fresh and does what the interface asks, one thing at a time.
fn serve(paths: Paths, shared: Arc<Mutex<Shared>>, jobs: mpsc::Receiver<Action>, changed: impl Fn()) {
    let desos = read_capped(&paths.os_release, 64 * 1024).is_ok_and(|bytes| is_desos(&String::from_utf8_lossy(&bytes)));
    if !desos { return; }
    let refresh = |shared: &Mutex<Shared>| {
        let integrity = Some(read_verify(&paths));
        let mut shared = shared.lock().unwrap();
        let differs = shared.integrity != integrity;
        shared.integrity = integrity;
        differs
    };
    refresh(&shared);
    shared.lock().unwrap().present = true;
    PRESENT.store(true, Ordering::Relaxed);
    changed();
    loop {
        match jobs.recv_timeout(VERIFY_EVERY) {
            Ok(action) => { perform(&paths, &shared, action); refresh(&shared); changed(); }
            Err(mpsc::RecvTimeoutError::Timeout) => { if refresh(&shared) { changed(); } }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The desktop's side: what was last read, and a way to ask for more.
#[derive(Default)]
pub struct Desos {
    shared: Arc<Mutex<Shared>>,
    jobs: Option<mpsc::Sender<Action>>,
    asked_notices: bool,
    /// The snapshot whose changes are shown.
    pub(crate) selected: Option<u64>,
    /// The snapshot the user asked to go back to, awaiting a second, explicit yes.
    pub(crate) confirming: Option<u64>,
    /// When the snapshot list was last asked for.
    pub(crate) refreshed: Option<Instant>,
}

impl Desos {
    /// Starts the worker. In tests nothing is started: no test reads this machine
    /// or runs its tools.
    pub fn start(ctx: egui::Context) -> Desos {
        let mut desos = Desos::default();
        if cfg!(test) { return desos; }
        let (sender, receiver) = mpsc::channel();
        let shared = desos.shared.clone();
        std::thread::spawn(move || serve(Paths::system(), shared, receiver, move || ctx.request_repaint()));
        desos.jobs = Some(sender);
        desos
    }

    pub fn present(&self) -> bool { self.shared.lock().unwrap().present }

    /// A DesOS machine in a given state, with no worker behind it.
    #[cfg(test)]
    pub fn for_test(integrity: Option<Integrity>, status: Fetch<Status>, snapshots: Fetch<Vec<Snapshot>>) -> Desos {
        let desos = Desos::default();
        *desos.shared.lock().unwrap() = Shared { present: true, integrity, status, snapshots, ..Default::default() };
        desos
    }

    /// The system panel's line; None where there is nothing to say (not DesOS).
    pub fn integrity_line(&self) -> Option<Line> {
        let shared = self.shared.lock().unwrap();
        if !shared.present { return None; }
        shared.integrity.as_ref().map(Integrity::line)
    }

    pub fn view(&self) -> View {
        let shared = self.shared.lock().unwrap();
        View { integrity: shared.integrity.clone(), status: shared.status.clone(), snapshots: shared.snapshots.clone(), diff: shared.diff.clone(), working: shared.working, outcome: shared.outcome.clone() }
    }

    /// Asks the worker for something. Returns at once.
    pub fn request(&mut self, action: Action) {
        let Some(jobs) = &self.jobs else { return };
        {
            let mut shared = self.shared.lock().unwrap();
            if !shared.present { return; }
            match action {
                // What is already shown stays until its replacement arrives.
                Action::Status => if !matches!(shared.status, Fetch::Ready(_)) { shared.status = Fetch::Loading; },
                Action::List => if !matches!(shared.snapshots, Fetch::Ready(_)) { shared.snapshots = Fetch::Loading; },
                Action::Diff(id) => shared.diff = Some((id, Fetch::Loading)),
                Action::Snapshot | Action::Rollback(_) => {
                    // One privileged action at a time.
                    if shared.working.is_some() { return; }
                    shared.working = Some(action);
                    shared.outcome = None;
                }
                Action::Notices | Action::AckNotices => {}
            }
        }
        let _ = jobs.send(action);
    }

    /// Call every frame of a signed-in, unlocked session: the first time on DesOS
    /// it asks for the notices; when they arrive it hands them over once and tells
    /// the tool they were shown.
    pub fn session_notices(&mut self) -> Vec<Notice> {
        if !self.asked_notices {
            if !self.present() { return Vec::new(); }
            self.asked_notices = true;
            self.request(Action::Notices);
            return Vec::new();
        }
        let notices = std::mem::take(&mut self.shared.lock().unwrap().notices);
        if !notices.is_empty() { self.request(Action::AckNotices); }
        notices
    }
}

/// Asks, from wherever the integrity line is drawn, for its explanation to open.
pub fn request_details(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("desos-details-request"), true));
}

/// The request made this frame, if any.
pub fn take_details_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(egui::Id::new("desos-details-request"))).unwrap_or(false)
}

#[cfg(test)]
#[path = "desos_tests.rs"]
mod tests;
