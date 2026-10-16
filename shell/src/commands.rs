//! What the palette offers beyond applications and files: SSH hosts, a calculator,
//! the user's processes, systemd units, and commands of the user's own. Nothing here
//! hands a query to a shell: hosts and commands become argument lists, the calculator
//! is its own evaluator, and every program run is named outright with its arguments.
//!
//! Reading takes place on one background worker, bounded in time and in count; the
//! interface only ever sees a finished [`Catalog`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long `systemctl` may take to answer before it is given up on.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
/// How often the units are read again while the palette is in use.
const UNITS_EVERY: Duration = Duration::from_secs(10);
/// How often the SSH hosts and the user's commands are read again.
const SOURCES_EVERY: Duration = Duration::from_secs(30);
const MAX_HOSTS: usize = 256;
const MAX_COMMANDS: usize = 128;
const MAX_UNITS: usize = 512;
const MAX_PROCESSES: usize = 64;

// ---- SSH hosts ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    pub name: String,
    /// The `HostName` given for it, when there is one.
    pub address: Option<String>,
}

/// The named hosts of an SSH configuration: `Host` entries that are names rather than
/// patterns. `include` reads a file an `Include` line names, relative names being
/// under `~/.ssh`; included files are read one level deep and may not include further.
pub fn parse_ssh_config(text: &str, include: &dyn Fn(&str) -> Option<String>) -> Vec<Host> {
    let mut hosts = Vec::new();
    parse_ssh_level(text, Some(include), &mut hosts);
    hosts.truncate(MAX_HOSTS);
    hosts
}

fn parse_ssh_level(text: &str, include: Option<&dyn Fn(&str) -> Option<String>>, hosts: &mut Vec<Host>) {
    // Names the current `Host` line declared, so a `HostName` below it is theirs.
    let mut current: Vec<usize> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((key, value)) => (key.trim().to_lowercase(), value.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for name in value.split_whitespace() {
                    let pattern = name.contains(['*', '?', '!']);
                    if pattern || hosts.iter().any(|host| host.name == name) || hosts.len() >= MAX_HOSTS {
                        continue;
                    }
                    hosts.push(Host { name: name.to_string(), address: None });
                    current.push(hosts.len() - 1);
                }
            }
            // A `Match` block is not a host; nothing in it is ours.
            "match" => current.clear(),
            "hostname" => {
                for &index in &current {
                    hosts[index].address = Some(value.to_string());
                }
            }
            "include" => {
                if let Some(include) = include {
                    for name in value.split_whitespace().take(8) {
                        if let Some(text) = include(name) {
                            parse_ssh_level(&text, None, hosts);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn ssh_hosts() -> Vec<Host> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let dir = home.join(".ssh");
    let Ok(text) = std::fs::read_to_string(dir.join("config")) else { return Vec::new() };
    let include = |name: &str| -> Option<String> {
        let path = if name.starts_with('/') { PathBuf::from(name) } else { dir.join(name.strip_prefix("~/").unwrap_or(name)) };
        let meta = std::fs::metadata(&path).ok()?;
        (meta.is_file() && meta.len() <= 1 << 20).then(|| std::fs::read_to_string(&path).ok()).flatten()
    };
    parse_ssh_config(&text, &include)
}

// ---- The user's own commands ------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct UserCommand {
    /// The file's name without its ending; what pins and sessions would know it by.
    pub id: String,
    pub name: String,
    /// The program and its arguments, already split; no shell sees them.
    pub exec: Vec<String>,
    pub terminal: bool,
    pub keywords: Vec<String>,
}

/// Reads one `.command` file: `key = value` lines with `name`, `exec`, `terminal`
/// (`yes` or `no`) and `keywords` (separated by commas or semicolons).
pub fn parse_command(text: &str, id: &str) -> Option<UserCommand> {
    let mut command = UserCommand { id: id.to_string(), name: String::new(), exec: Vec::new(), terminal: false, keywords: Vec::new() };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim().to_lowercase().as_str() {
            "name" => command.name = value.chars().filter(|c| !c.is_control()).take(80).collect(),
            "exec" => command.exec = crate::apps::split_exec(value),
            "terminal" => command.terminal = matches!(value.to_lowercase().as_str(), "yes" | "true" | "1"),
            "keywords" => command.keywords = value.split([',', ';']).map(|word| word.trim().to_lowercase()).filter(|word| !word.is_empty()).take(16).collect(),
            _ => {}
        }
    }
    if command.name.is_empty() {
        command.name = id.to_string();
    }
    (!command.exec.is_empty() && !command.exec[0].is_empty()).then_some(command)
}

fn commands_dir() -> Option<PathBuf> {
    des_common::input::config_path().and_then(|path| path.parent().map(|dir| dir.join("commands")))
}

pub fn user_commands_in(dir: &Path) -> Vec<UserCommand> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut commands: Vec<UserCommand> = Vec::new();
    for item in listing.flatten() {
        let path = item.path();
        if path.extension().is_none_or(|ending| ending != "command") || !item.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(id) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else { continue };
        if let Some(command) = std::fs::read_to_string(&path).ok().filter(|text| text.len() <= 16 * 1024).and_then(|text| parse_command(&text, &id)) {
            commands.push(command);
        }
        if commands.len() >= MAX_COMMANDS {
            break;
        }
    }
    commands.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    commands
}

// ---- Processes ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub uid: u32,
    pub name: String,
    pub command_line: String,
}

/// `kill <name>` or `ps <name>`: what the user wants to look for among their processes.
pub fn process_query(query: &str) -> Option<&str> {
    let query = query.trim();
    let (verb, rest) = query.split_once(char::is_whitespace)?;
    let rest = rest.trim();
    (matches!(verb.to_lowercase().as_str(), "kill" | "ps") && !rest.is_empty()).then_some(rest)
}

/// The processes of `uid` whose name or command line mentions `needle`. Root's are
/// never among them, even for root.
pub fn filter_processes(all: &[Proc], uid: u32, needle: &str) -> Vec<Proc> {
    let needle = needle.to_lowercase();
    let mut found: Vec<Proc> = all
        .iter()
        .filter(|proc| proc.uid == uid && proc.uid != 0 && proc.pid != std::process::id())
        .filter(|proc| proc.name.to_lowercase().contains(&needle) || proc.command_line.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    found.sort_by(|a, b| (!a.name.to_lowercase().starts_with(&needle), a.pid).cmp(&(!b.name.to_lowercase().starts_with(&needle), b.pid)));
    found.truncate(MAX_PROCESSES);
    found
}

fn read_processes(system: &mut sysinfo::System) -> Vec<Proc> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::OnlyIfNotSet).with_cmd(UpdateKind::OnlyIfNotSet),
    );
    system
        .processes()
        .values()
        .filter_map(|proc| {
            let uid = **proc.user_id()?;
            let command_line: Vec<String> = proc.cmd().iter().take(16).map(|word| word.to_string_lossy().chars().filter(|c| !c.is_control()).take(120).collect()).collect();
            Some(Proc { pid: proc.pid().as_u32(), uid, name: proc.name().to_string_lossy().into_owned(), command_line: command_line.join(" ") })
        })
        .collect()
}

/// The user that owns a process now, read from the kernel, so a reused PID is never
/// signalled by mistake.
fn owner_of(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Asks the process to end (SIGTERM), provided it still belongs to this user.
pub fn terminate(pid: u32) -> Result<(), String> {
    let me = rustix::process::geteuid().as_raw();
    match owner_of(pid) {
        Some(uid) if uid == me && uid != 0 => {}
        Some(_) => return Err(format!("Process {pid} belongs to another user.")),
        None => return Err(format!("Process {pid} has already ended.")),
    }
    let pid = rustix::process::Pid::from_raw(pid as i32).ok_or_else(|| "Invalid process id.".to_string())?;
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).map_err(|e| format!("Could not signal the process: {e}"))
}

// ---- systemd units --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: String,
    /// Belongs to this user's own systemd rather than the system's.
    pub user: bool,
    pub active: String,
    pub sub: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnitOp { Start, Stop, Restart }

impl UnitOp {
    pub fn word(self) -> &'static str {
        match self { UnitOp::Start => "start", UnitOp::Stop => "stop", UnitOp::Restart => "restart" }
    }
}

/// `start x`, `stop x` or `restart x` names the operation; otherwise the query is the
/// name alone and the sensible operation depends on the unit's state.
pub fn unit_query(query: &str) -> (Option<UnitOp>, String) {
    let query = query.trim();
    if let Some((verb, rest)) = query.split_once(char::is_whitespace) {
        let op = match verb.to_lowercase().as_str() { "start" => Some(UnitOp::Start), "stop" => Some(UnitOp::Stop), "restart" => Some(UnitOp::Restart), _ => None };
        if op.is_some() {
            return (op, rest.trim().to_lowercase());
        }
    }
    (None, query.to_lowercase())
}

/// Reads `systemctl list-units --plain --no-legend` output: unit, load, active, sub,
/// then the description.
pub fn parse_units(text: &str, user: bool) -> Vec<Unit> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let _load = words.next()?;
            let active = words.next()?;
            let sub = words.next()?;
            let description: Vec<&str> = words.collect();
            Some(Unit { name: name.to_string(), user, active: active.to_string(), sub: sub.to_string(), description: description.join(" ") })
        })
        .take(MAX_UNITS)
        .collect()
}

/// Runs a program with a deadline. Its output is read on its own thread so a talkative
/// program cannot fill the pipe and wait on us.
fn run_bounded(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let stdout = child.stdout.take().ok_or("no output")?;
    let stderr = child.stderr.take().ok_or("no output")?;
    let out = std::thread::spawn(move || read_bounded(stdout));
    let err = std::thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} did not answer in time."));
            }
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let output = out.join().unwrap_or_default();
    let errors = err.join().unwrap_or_default();
    if status.success() {
        Ok(output)
    } else {
        let reason = errors.lines().next().unwrap_or("").trim();
        Err(if reason.is_empty() { format!("{program} exited with {status}") } else { reason.to_string() })
    }
}

fn read_bounded(from: impl std::io::Read) -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    let _ = from.take(256 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn list_units() -> Vec<Unit> {
    let mut units = Vec::new();
    if let Ok(text) = run_bounded("systemctl", &["--user", "list-units", "--type=service", "--plain", "--no-legend"], COMMAND_TIMEOUT) {
        units.extend(parse_units(&text, true));
    }
    if let Ok(text) = run_bounded("systemctl", &["list-units", "--type=service", "--state=failed,running", "--plain", "--no-legend"], COMMAND_TIMEOUT) {
        units.extend(parse_units(&text, false));
    }
    units.truncate(MAX_UNITS);
    units
}

/// Starts, stops or restarts a unit. User units are the user's to change; system
/// units are asked of `systemctl` as this user, which may refuse, and that refusal is
/// the answer.
pub fn act_on_unit(name: &str, user: bool, op: UnitOp) -> Result<(), String> {
    if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c.is_control()) || name.starts_with('-') {
        return Err("Not a unit name.".into());
    }
    let mut args = Vec::new();
    if user {
        args.push("--user");
    }
    args.extend(["--no-ask-password", op.word(), name]);
    run_bounded("systemctl", &args, Duration::from_secs(20)).map(|_| ())
}

// ---- The calculator -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Calc {
    /// What was understood, written back plainly.
    pub expression: String,
    pub result: String,
}

/// Works out a query that is arithmetic, a conversion between units, or a number
/// asked for in another base. A bare number is not a calculation.
pub fn calculate(query: &str) -> Option<Calc> {
    let query = query.trim();
    if query.is_empty() || !query.chars().any(|c| c.is_ascii_digit()) || query.len() > 200 {
        return None;
    }
    let lower = query.to_lowercase();
    // "<expression> [unit] in <unit or base>": the last " in " or " to " divides it.
    let divider = ["in", "to"].iter().find_map(|word| {
        let pattern = format!(" {word} ");
        lower.rfind(&pattern).map(|at| (at, pattern.len()))
    });
    if let Some((at, len)) = divider {
        let (left, right) = (lower[..at].trim(), lower[at + len..].trim());
        if let Some(calc) = convert(left, right) {
            return Some(calc);
        }
    }
    let value = evaluate(&lower)?;
    // A number alone, however it is written, is nothing to work out.
    if lower.trim().parse::<f64>().is_ok() {
        return None;
    }
    Some(Calc { expression: query.to_string(), result: format_number(value) })
}

fn convert(left: &str, right: &str) -> Option<Calc> {
    if let Some(base) = match right { "hex" | "hexadecimal" => Some(16), "bin" | "binary" => Some(2), "oct" | "octal" => Some(8), "dec" | "decimal" => Some(10), _ => None } {
        let value = evaluate(left)?;
        if value.fract() != 0.0 || value.abs() >= 9.0e18 {
            return None;
        }
        let integer = value as i64;
        let magnitude = integer.unsigned_abs();
        let digits = match base { 16 => format!("0x{magnitude:x}"), 2 => format!("0b{magnitude:b}"), 8 => format!("0o{magnitude:o}"), _ => magnitude.to_string() };
        return Some(Calc { expression: format!("{left} in {right}"), result: if integer < 0 { format!("-{digits}") } else { digits } });
    }
    let to = unit(right)?;
    let (expression, from) = match left.rsplit_once(char::is_whitespace) {
        Some((expression, word)) => (expression.trim(), unit(word)?),
        None => return None,
    };
    if from.kind != to.kind {
        return None;
    }
    let value = evaluate(expression)?;
    let result = to.from_base(from.to_base(value));
    Some(Calc { expression: format!("{expression} {} in {}", from.name, to.name), result: format!("{} {}", format_number(result), to.name) })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind { Length, Mass, Time, Data, Temperature, Angle, Volume, Speed }

#[derive(Clone, Copy)]
struct Unit1 { name: &'static str, kind: Kind, factor: f64, offset: f64 }

impl Unit1 {
    fn to_base(self, value: f64) -> f64 { (value + self.offset) * self.factor }
    fn from_base(self, base: f64) -> f64 { base / self.factor - self.offset }
}

const UNITS: &[(&[&str], Kind, f64, f64)] = &[
    (&["m", "meter", "metre"], Kind::Length, 1.0, 0.0),
    (&["km", "kilometer", "kilometre"], Kind::Length, 1000.0, 0.0),
    (&["cm", "centimeter", "centimetre"], Kind::Length, 0.01, 0.0),
    (&["mm", "millimeter", "millimetre"], Kind::Length, 0.001, 0.0),
    (&["mi", "mile"], Kind::Length, 1609.344, 0.0),
    (&["ft", "foot", "feet"], Kind::Length, 0.3048, 0.0),
    (&["inch", "inches"], Kind::Length, 0.0254, 0.0),
    (&["yd", "yard"], Kind::Length, 0.9144, 0.0),
    (&["nmi"], Kind::Length, 1852.0, 0.0),
    (&["kg", "kilogram"], Kind::Mass, 1.0, 0.0),
    (&["g", "gram"], Kind::Mass, 0.001, 0.0),
    (&["mg", "milligram"], Kind::Mass, 1.0e-6, 0.0),
    (&["lb", "lbs", "pound"], Kind::Mass, 0.453_592_37, 0.0),
    (&["oz", "ounce"], Kind::Mass, 0.028_349_523_125, 0.0),
    (&["t", "tonne", "ton"], Kind::Mass, 1000.0, 0.0),
    (&["s", "sec", "second"], Kind::Time, 1.0, 0.0),
    (&["ms", "millisecond"], Kind::Time, 0.001, 0.0),
    (&["min", "minute"], Kind::Time, 60.0, 0.0),
    (&["h", "hr", "hour"], Kind::Time, 3600.0, 0.0),
    (&["d", "day"], Kind::Time, 86_400.0, 0.0),
    (&["week", "wk"], Kind::Time, 604_800.0, 0.0),
    (&["byte", "bytes"], Kind::Data, 1.0, 0.0),
    (&["bit", "bits"], Kind::Data, 0.125, 0.0),
    (&["kb", "kilobyte"], Kind::Data, 1.0e3, 0.0),
    (&["mb", "megabyte"], Kind::Data, 1.0e6, 0.0),
    (&["gb", "gigabyte"], Kind::Data, 1.0e9, 0.0),
    (&["tb", "terabyte"], Kind::Data, 1.0e12, 0.0),
    (&["kib", "kibibyte"], Kind::Data, 1024.0, 0.0),
    (&["mib", "mebibyte"], Kind::Data, 1_048_576.0, 0.0),
    (&["gib", "gibibyte"], Kind::Data, 1_073_741_824.0, 0.0),
    (&["tib", "tebibyte"], Kind::Data, 1_099_511_627_776.0, 0.0),
    (&["kbit", "kilobit"], Kind::Data, 125.0, 0.0),
    (&["mbit", "megabit"], Kind::Data, 125_000.0, 0.0),
    (&["gbit", "gigabit"], Kind::Data, 125_000_000.0, 0.0),
    // Temperatures go through kelvin: an offset before the factor.
    (&["k", "kelvin"], Kind::Temperature, 1.0, 0.0),
    (&["c", "celsius", "°c"], Kind::Temperature, 1.0, 273.15),
    (&["f", "fahrenheit", "°f"], Kind::Temperature, 5.0 / 9.0, 459.67),
    (&["deg", "degree"], Kind::Angle, std::f64::consts::PI / 180.0, 0.0),
    (&["rad", "radian"], Kind::Angle, 1.0, 0.0),
    (&["l", "liter", "litre"], Kind::Volume, 1.0, 0.0),
    (&["ml", "milliliter", "millilitre"], Kind::Volume, 0.001, 0.0),
    (&["gal", "gallon"], Kind::Volume, 3.785_411_784, 0.0),
    (&["kph", "kmh"], Kind::Speed, 1.0 / 3.6, 0.0),
    (&["mph"], Kind::Speed, 0.447_04, 0.0),
    (&["knot", "kn"], Kind::Speed, 0.514_444, 0.0),
];

fn unit(word: &str) -> Option<Unit1> {
    let word = word.trim().to_lowercase();
    let singular = word.strip_suffix('s').filter(|w| w.len() >= 3).unwrap_or(&word);
    UNITS
        .iter()
        .find(|(names, ..)| names.contains(&word.as_str()) || names.contains(&singular))
        .map(|&(names, kind, factor, offset)| Unit1 { name: names[0], kind, factor, offset })
}

fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "undefined".into();
    }
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        return format!("{}", value as i64);
    }
    if value.abs() >= 1.0e15 || value.abs() < 1.0e-6 {
        return format!("{value:.6e}");
    }
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Evaluates arithmetic: `+ - * / % ^`, parentheses, unary minus, `0x` and `0b`
/// literals, `sqrt`, `abs`, `round`, `floor`, `ceil`, and `pi` and `e`.
pub fn evaluate(text: &str) -> Option<f64> {
    let tokens = tokenize(text)?;
    let mut parser = Parser { tokens, at: 0, depth: 0 };
    let value = parser.expression()?;
    (parser.at == parser.tokens.len() && value.is_finite()).then_some(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token { Number(f64), Word(String), Op(char), Open, Close }

fn tokenize(text: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            let radix = match (c, chars.get(i + 1).map(|c| c.to_ascii_lowercase())) {
                ('0', Some('x')) => 16,
                ('0', Some('b')) => 2,
                _ => 10,
            };
            if radix != 10 {
                i += 2;
                let digits_start = i;
                while i < chars.len() && (chars[i].is_digit(radix) || chars[i] == '_') {
                    i += 1;
                }
                let digits: String = chars[digits_start..i].iter().filter(|c| **c != '_').collect();
                let value = u64::from_str_radix(&digits, radix).ok()?;
                tokens.push(Token::Number(value as f64));
            } else {
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                    i += 1;
                }
                // An exponent, when a digit follows it directly or after a sign.
                if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && matches!(chars[j], '+' | '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let digits: String = chars[start..i].iter().filter(|c| **c != '_').collect();
                tokens.push(Token::Number(digits.parse().ok()?));
            }
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            tokens.push(Token::Word(chars[start..i].iter().collect::<String>().to_lowercase()));
        } else {
            i += 1;
            match c {
                '+' | '-' | '*' | '/' | '%' | '^' => tokens.push(Token::Op(c)),
                '×' => tokens.push(Token::Op('*')),
                '÷' => tokens.push(Token::Op('/')),
                '(' => tokens.push(Token::Open),
                ')' => tokens.push(Token::Close),
                ',' => {}
                _ => return None,
            }
        }
        if tokens.len() > 256 {
            return None;
        }
    }
    (!tokens.is_empty()).then_some(tokens)
}

struct Parser { tokens: Vec<Token>, at: usize, depth: usize }

impl Parser {
    fn peek(&self) -> Option<&Token> { self.tokens.get(self.at) }
    fn take(&mut self) -> Option<Token> { let token = self.tokens.get(self.at).cloned(); self.at += 1; token }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.at += 1;
            let right = self.term()?;
            value = if op == '+' { value + right } else { value - right };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        while let Some(Token::Op(op @ ('*' | '/' | '%'))) = self.peek().cloned() {
            self.at += 1;
            let right = self.unary()?;
            value = match op { '*' => value * right, '/' => value / right, _ => value % right };
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Token::Op('-')) => { self.at += 1; self.unary().map(|v| -v) }
            Some(Token::Op('+')) => { self.at += 1; self.unary() }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.at += 1;
            // Right-associative: 2^3^2 is 2^9.
            let exponent = self.unary()?;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let value = match self.take()? {
            Token::Number(value) => value,
            Token::Open => {
                let value = self.expression()?;
                (self.take()? == Token::Close).then_some(value)?
            }
            Token::Word(word) => match word.as_str() {
                "pi" => std::f64::consts::PI,
                "e" => std::f64::consts::E,
                "sqrt" | "abs" | "round" | "floor" | "ceil" => {
                    (self.take()? == Token::Open).then_some(())?;
                    let inner = self.expression()?;
                    (self.take()? == Token::Close).then_some(())?;
                    match word.as_str() { "sqrt" => inner.sqrt(), "abs" => inner.abs(), "round" => inner.round(), "floor" => inner.floor(), _ => inner.ceil() }
                }
                _ => return None,
            },
            _ => return None,
        };
        self.depth -= 1;
        Some(value)
    }
}

// ---- The worker ---------------------------------------------------------------

/// Everything the palette may offer from here, as read by the worker.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub hosts: Vec<Host>,
    pub commands: Vec<UserCommand>,
    /// Filled only when the query asks for processes.
    pub processes: Vec<Proc>,
    pub units: Vec<Unit>,
}

struct Job { generation: u64, query: String }

/// One background thread reading hosts, commands, units and processes. Results carry
/// the generation of the query that asked for them; older answers are dropped.
pub struct Worker {
    tx: SyncSender<Job>,
    generation: Arc<AtomicU64>,
    ready: Arc<Mutex<Option<(u64, Catalog)>>>,
}

impl Default for Worker {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        let generation = Arc::new(AtomicU64::new(0));
        let ready = Arc::new(Mutex::new(None));
        let (current, result) = (generation.clone(), ready.clone());
        std::thread::spawn(move || {
            let mut sources: Option<(Instant, Vec<Host>, Vec<UserCommand>)> = None;
            let mut units: Option<(Instant, Vec<Unit>)> = None;
            let mut system = sysinfo::System::new();
            while let Ok(job) = rx.recv() {
                if sources.as_ref().is_none_or(|(read, ..)| read.elapsed() >= SOURCES_EVERY) {
                    let commands = commands_dir().map(|dir| user_commands_in(&dir)).unwrap_or_default();
                    sources = Some((Instant::now(), ssh_hosts(), commands));
                }
                if current.load(Ordering::Relaxed) != job.generation {
                    continue;
                }
                let (hosts, commands) = sources.as_ref().map(|(_, hosts, commands)| (hosts.clone(), commands.clone())).unwrap_or_default();
                let mut catalog = Catalog { hosts, commands, ..Default::default() };
                if !job.query.trim().is_empty() {
                    if units.as_ref().is_none_or(|(read, _)| read.elapsed() >= UNITS_EVERY) {
                        units = Some((Instant::now(), list_units()));
                    }
                    catalog.units = units.as_ref().map(|(_, units)| units.clone()).unwrap_or_default();
                    if let Some(needle) = process_query(&job.query) {
                        if current.load(Ordering::Relaxed) != job.generation {
                            continue;
                        }
                        let me = rustix::process::geteuid().as_raw();
                        catalog.processes = filter_processes(&read_processes(&mut system), me, needle);
                    }
                }
                if current.load(Ordering::Relaxed) == job.generation {
                    *result.lock().unwrap_or_else(|e| e.into_inner()) = Some((job.generation, catalog));
                }
            }
        });
        Worker { tx, generation, ready }
    }
}

impl Worker {
    /// The generation counter shared with the palette; bumping it drops older answers.
    pub fn generation(&self) -> &Arc<AtomicU64> { &self.generation }

    /// Asks for the catalog for this query. False when the worker is still busy with
    /// the previous request; ask again next frame.
    pub fn request(&self, generation: u64, query: &str) -> bool {
        self.tx.try_send(Job { generation, query: query.to_string() }).is_ok()
    }

    pub fn take(&self, generation: u64) -> Option<Catalog> {
        let mut ready = self.ready.lock().unwrap_or_else(|e| e.into_inner());
        match ready.take() {
            Some((version, catalog)) if version == generation => Some(catalog),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_keeps_named_hosts_follows_one_include_and_skips_patterns() {
        let included = "Host work\n  HostName work.example.org\nInclude deeper\n";
        let include = |name: &str| (name == "config.d/work").then(|| included.to_string());
        let text = "# comment\nHost home *.lan bastion\n    HostName 10.0.0.2\nHost=aliased\nMatch host x\n  HostName no\nInclude config.d/work\nHost !neg ?odd\n";
        let hosts = parse_ssh_config(text, &include);
        let names: Vec<&str> = hosts.iter().map(|host| host.name.as_str()).collect();
        assert_eq!(names, ["home", "bastion", "aliased", "work"]);
        assert_eq!(hosts[0].address.as_deref(), Some("10.0.0.2"));
        assert_eq!(hosts[1].address.as_deref(), Some("10.0.0.2"));
        assert_eq!(hosts[2].address, None);
        assert_eq!(hosts[3].address.as_deref(), Some("work.example.org"));
    }

    #[test]
    fn calculator_respects_precedence_bases_and_refuses_nonsense() {
        assert_eq!(evaluate("2+3*4"), Some(14.0));
        assert_eq!(evaluate("(2+3)*4"), Some(20.0));
        assert_eq!(evaluate("2^3^2"), Some(512.0));
        assert_eq!(evaluate("-2^2"), Some(-4.0));
        assert_eq!(evaluate("10 % 4 + 0x10 + 0b11"), Some(21.0));
        assert_eq!(evaluate("sqrt(16) + abs(-2) + round(2.4)"), Some(8.0));
        assert_eq!(evaluate("1e3 + 1_000"), Some(2000.0));
        assert_eq!(evaluate("2 +"), None);
        assert_eq!(evaluate("rm -rf /"), None);
        assert_eq!(evaluate("$(id)"), None);
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(calculate("2+2").unwrap().result, "4");
        assert_eq!(calculate("7/2").unwrap().result, "3.5");
        assert_eq!(calculate("42"), None);
        assert_eq!(calculate("firefox"), None);
        assert_eq!(calculate("255 in hex").unwrap().result, "0xff");
        assert_eq!(calculate("0xff in bin").unwrap().result, "0b11111111");
        assert_eq!(calculate("-8 in oct").unwrap().result, "-0o10");
        assert_eq!(calculate("2.5 in hex"), None);
    }

    #[test]
    fn calculator_converts_units_of_one_kind_only() {
        assert_eq!(calculate("10 km in mi").unwrap().result, "6.213712 mi");
        assert_eq!(calculate("72 f in c").unwrap().result, "22.222222 c");
        assert_eq!(calculate("100 c to f").unwrap().result, "212 f");
        assert_eq!(calculate("3 GiB in GB").unwrap().result, "3.221225 gb");
        assert_eq!(calculate("2 hours in minutes").unwrap().result, "120 min");
        assert_eq!(calculate("1.5 miles in km").unwrap().result, "2.414016 km");
        assert_eq!(calculate("10 km in kg"), None);
        assert_eq!(calculate("10 km in furlongs"), None);
    }

    #[test]
    fn command_files_split_their_programs_without_a_shell() {
        let command = parse_command("# mine\nname = Deploy\nexec = \"/opt/my tool\" --fast %u $(evil)\nterminal = yes\nkeywords = ship, release\n", "deploy").unwrap();
        assert_eq!(command.exec, ["/opt/my tool", "--fast", "$(evil)"]);
        assert!(command.terminal);
        assert_eq!(command.keywords, ["ship", "release"]);
        assert_eq!(command.name, "Deploy");
        assert!(parse_command("name = Nothing\n", "x").is_none());
        assert_eq!(parse_command("exec = ls\n", "list").unwrap().name, "list");
        let dir = std::env::temp_dir().join(format!("des-commands-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.command"), "name = Beta\nexec = beta\n").unwrap();
        std::fs::write(dir.join("a.command"), "name = Alpha\nexec = alpha\n").unwrap();
        std::fs::write(dir.join("ignored.txt"), "name = No\nexec = no\n").unwrap();
        let names: Vec<String> = user_commands_in(&dir).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Alpha", "Beta"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn processes_are_filtered_to_the_users_own_and_never_roots() {
        let all = vec![
            Proc { pid: 1, uid: 0, name: "systemd".into(), command_line: "/sbin/init".into() },
            Proc { pid: 20, uid: 1000, name: "node".into(), command_line: "node server.js".into() },
            Proc { pid: 21, uid: 1000, name: "python3".into(), command_line: "python3 node_tool.py".into() },
            Proc { pid: 22, uid: 1001, name: "node".into(), command_line: "node other".into() },
        ];
        let mine: Vec<u32> = filter_processes(&all, 1000, "node").iter().map(|p| p.pid).collect();
        assert_eq!(mine, [20, 21]);
        assert!(filter_processes(&all, 0, "systemd").is_empty());
        assert_eq!(process_query("kill node"), Some("node"));
        assert_eq!(process_query("ps  python"), Some("python"));
        assert_eq!(process_query("kill"), None);
        assert_eq!(process_query("skill node"), None);
        assert!(terminate(1).is_err());
    }

    #[test]
    fn units_parse_and_queries_name_their_operation() {
        let text = "ssh.service loaded active running OpenBSD Secure Shell server\nbroken.service loaded failed failed A thing that broke\n";
        let units = parse_units(text, false);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].description, "OpenBSD Secure Shell server");
        assert_eq!((units[1].active.as_str(), units[1].sub.as_str()), ("failed", "failed"));
        assert_eq!(unit_query("restart ssh"), (Some(UnitOp::Restart), "ssh".into()));
        assert_eq!(unit_query("Stop Foo.service"), (Some(UnitOp::Stop), "foo.service".into()));
        assert_eq!(unit_query("ssh"), (None, "ssh".into()));
        assert!(act_on_unit("", false, UnitOp::Start).is_err());
        assert!(act_on_unit("-x", false, UnitOp::Start).is_err());
    }
}
