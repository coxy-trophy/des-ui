//! The frame follows the machine you are in. While the terminal in use is running
//! `ssh`, the side panels show that machine: its processor, memory, processes,
//! network and connections, read every two seconds through one bounded `ssh` call
//! of our own. A control connection is kept so the key or the agent is asked once;
//! a password-only host cannot be followed and says so.
//!
//! Nothing here runs anything the user typed: the ssh command line is read only to
//! find out where it went, and the one script sent is fixed.

use crate::sensors::Filesystem;
use crate::sysmon::{HISTORY, ProcInfo};
use crate::term::TermTab;
use eframe::egui;
use std::collections::VecDeque;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// How often the remote machine is asked.
const EVERY: Duration = Duration::from_secs(2);
/// Once a host has refused, how long before asking again.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// How long one call may take, connection and all.
const TIMEOUT: Duration = Duration::from_secs(5);
/// How much of what the host prints is kept.
const MAX_OUTPUT: usize = 256 * 1024;
/// How often the terminal's foreground process is looked at.
const CHECK_TAB_EVERY: Duration = Duration::from_millis(500);
const TOP_PROCESSES: usize = 8;
const FILESYSTEMS: usize = 6;
const MAX_CONNECTIONS: usize = 64;

/// What the host is asked to print: one section per `=name` line. It is POSIX sh,
/// written without single quotes so it survives any login shell on the far side.
const SCRIPT: &str = concat!(
    "echo =hostname; hostname 2>/dev/null || cat /proc/sys/kernel/hostname 2>/dev/null; ",
    "echo =os; (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\") || uname -s; uname -r; ",
    "echo =uptime; cat /proc/uptime 2>/dev/null; ",
    "echo =stat; grep \"^cpu\" /proc/stat 2>/dev/null | head -n 65; ",
    "echo =meminfo; head -n 24 /proc/meminfo 2>/dev/null; ",
    "echo =loadavg; cat /proc/loadavg 2>/dev/null; ",
    "echo =netdev; cat /proc/net/dev 2>/dev/null; ",
    "echo =ip; hostname -I 2>/dev/null; ",
    "echo =ps; ps -eo pid,pcpu,rss,comm 2>/dev/null | tail -n +2 | sort -k2,2nr | head -n 8; ",
    "echo =df; df -P -l 2>/dev/null | tail -n +2 | head -n 40; ",
    "echo =conn; ss -Htn 2>/dev/null || netstat -tn 2>/dev/null | tail -n +3; ",
    "echo =end",
);

/// Where `ssh` went, as read from its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl Target {
    /// `user@host`, or the host alone, for the panel.
    pub fn name(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }

    fn ssh_args(&self, control_dir: &Path) -> Vec<String> {
        let mut args = vec![
            "-o".into(), "BatchMode=yes".into(),
            "-o".into(), "ConnectTimeout=3".into(),
            "-o".into(), "ControlMaster=auto".into(),
            "-o".into(), format!("ControlPath={}", control_dir.join("ssh-%C").display()),
            "-o".into(), "ControlPersist=60".into(),
            "-o".into(), "LogLevel=ERROR".into(),
        ];
        if let Some(port) = self.port {
            args.push("-p".into());
            args.push(port.to_string());
        }
        if let Some(user) = &self.user {
            args.push("-l".into());
            args.push(user.clone());
        }
        args.push(self.host.clone());
        args
    }
}

/// Options of `ssh` that take the next word as their value.
const WITH_VALUE: &[&str] = &[
    "-p", "-l", "-i", "-o", "-J", "-F", "-L", "-R", "-D", "-W", "-b", "-c", "-e", "-m", "-E", "-S", "-Q", "-B", "-I", "-O",
];

/// Reads `ssh [options] [user@]host [command]`. `None` for anything that is not ssh,
/// and for tunnels (`-N`, `-W`), which reach no shell worth following.
pub fn parse_ssh(argv: &[String]) -> Option<Target> {
    let program = argv.first()?;
    if Path::new(program).file_name()?.to_str()? != "ssh" {
        return None;
    }
    let (mut user, mut port, mut destination) = (None, None, None);
    let mut words = argv[1..].iter();
    while let Some(word) = words.next() {
        if destination.is_some() {
            break;
        }
        if word == "--" {
            destination = words.next().cloned();
            break;
        }
        if let Some(flags) = word.strip_prefix('-').filter(|rest| !rest.is_empty()) {
            // `-N` and `-W` make a tunnel; `-W` also takes a value, so it is caught here.
            if flags.contains('N') || flags.starts_with('W') {
                return None;
            }
            let option = format!("-{}", flags.chars().next()?);
            if WITH_VALUE.contains(&option.as_str()) {
                // The value is the rest of the word, or the next one: `-p2222` or `-p 2222`.
                let value = if flags.len() > 1 { flags[1..].to_string() } else { words.next()?.clone() };
                match option.as_str() {
                    "-p" => port = value.parse().ok(),
                    "-l" => user = Some(value),
                    _ => {}
                }
            }
            continue;
        }
        destination = Some(word.clone());
    }
    let destination = destination?;
    let destination = destination.strip_prefix("ssh://").unwrap_or(&destination);
    let (user_part, rest) = match destination.rsplit_once('@') {
        Some((user, rest)) => (Some(user.to_string()), rest),
        None => (None, destination),
    };
    // A URL may carry the port after the host; a bracketed IPv6 host keeps its colons.
    let (host, url_port) = match rest.strip_prefix('[') {
        Some(bracketed) => {
            let (host, after) = bracketed.split_once(']')?;
            (host.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()))
        }
        None if destination.starts_with("ssh://") || rest.matches(':').count() == 1 => match rest.split_once(':') {
            Some((host, p)) => (host.to_string(), p.parse().ok()),
            None => (rest.to_string(), None),
        },
        None => (rest.to_string(), None),
    };
    if host.is_empty() || host.starts_with('-') || host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(Target { user: user_part.or(user), host, port: url_port.or(port) })
}

/// How following the host stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Asked, not yet answered.
    Connecting,
    /// Answering; the panels show it.
    Live,
    /// The host would not let us in without a password: a key or an agent is needed.
    NoKey,
    /// No answer in time, or no way there.
    Unreachable,
    /// It answered, but not with what Linux prints.
    NotLinux,
}

impl State {
    /// A word for the panel; nothing while live, since the readings speak.
    pub fn word(self) -> &'static str {
        match self {
            State::Connecting => "CONNECTING",
            State::Live => "",
            State::NoKey => "NEEDS A KEY",
            State::Unreachable => "UNREACHABLE",
            State::NotLinux => "NOT LINUX",
        }
    }
}

/// One answer from the host, as printed: counters, not rates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub uptime: u64,
    /// Busy and total jiffies, the whole machine first and then each core.
    pub cpu: Vec<(u64, u64)>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load: Option<[f32; 3]>,
    /// Each interface with its bytes received and sent since boot.
    pub net: Vec<(String, u64, u64)>,
    pub ip: String,
    pub top: Vec<ProcInfo>,
    pub filesystems: Vec<Filesystem>,
    pub connections: Vec<IpAddr>,
}

fn sections(text: &str) -> Vec<(&str, Vec<&str>)> {
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('=').filter(|name| !name.contains(' ')) {
            sections.push((name, Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        }
    }
    sections
}

fn number(word: Option<&str>) -> u64 {
    word.and_then(|word| word.parse().ok()).unwrap_or(0)
}

/// Reads what [`SCRIPT`] printed. `None` when it is not the answer of a Linux host.
pub fn parse_sample(text: &str) -> Option<Sample> {
    let sections = sections(text);
    let get = |name: &str| sections.iter().find(|(n, _)| *n == name).map(|(_, lines)| lines.as_slice()).unwrap_or(&[]);
    if !sections.iter().any(|(name, _)| *name == "end") {
        return None;
    }
    let mut sample = Sample::default();
    sample.hostname = get("hostname").first().map(|s| s.trim().to_string()).unwrap_or_default();
    let os = get("os");
    sample.os = os.first().map(|s| s.trim().trim_matches('"').to_string()).unwrap_or_default();
    sample.kernel = os.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
    sample.uptime = get("uptime").first().and_then(|line| line.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.0) as u64;

    for line in get("stat") {
        let mut words = line.split_whitespace();
        if !words.next().is_some_and(|w| w.starts_with("cpu")) {
            continue;
        }
        let values: Vec<u64> = words.map(|w| w.parse().unwrap_or(0)).collect();
        if values.len() < 4 {
            continue;
        }
        let total: u64 = values.iter().sum();
        let idle = values[3] + values.get(4).copied().unwrap_or(0);
        sample.cpu.push((total.saturating_sub(idle), total));
    }
    if sample.cpu.is_empty() {
        return None;
    }

    let mut meminfo = std::collections::HashMap::new();
    for line in get("meminfo") {
        if let Some((key, value)) = line.split_once(':') {
            meminfo.insert(key.trim(), number(value.split_whitespace().next()) * 1024);
        }
    }
    let mem = |key: &str| meminfo.get(key).copied();
    sample.mem_total = mem("MemTotal").unwrap_or(0);
    let available = mem("MemAvailable")
        .or_else(|| Some(mem("MemFree")? + mem("Buffers").unwrap_or(0) + mem("Cached").unwrap_or(0)))
        .unwrap_or(0);
    sample.mem_used = sample.mem_total.saturating_sub(available);
    sample.swap_total = mem("SwapTotal").unwrap_or(0);
    sample.swap_used = sample.swap_total.saturating_sub(mem("SwapFree").unwrap_or(0));

    sample.load = get("loadavg").first().and_then(|line| crate::sensors::parse_loadavg(line));

    for line in get("netdev") {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if name == "lo" {
            continue;
        }
        let values: Vec<u64> = rest.split_whitespace().map(|w| w.parse().unwrap_or(0)).collect();
        if values.len() >= 9 {
            sample.net.push((name.to_string(), values[0], values[8]));
        }
    }
    sample.ip = get("ip").first().and_then(|line| line.split_whitespace().next()).unwrap_or("").to_string();

    for line in get("ps").iter().take(TOP_PROCESSES) {
        let mut words = line.split_whitespace();
        let _pid = words.next();
        let cpu: f32 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0.0);
        let rss = number(words.next());
        let name: String = words.collect::<Vec<_>>().join(" ");
        if !name.is_empty() {
            sample.top.push(ProcInfo { name, cpu, mem: rss * 1024 });
        }
    }

    for line in get("df") {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 6 {
            continue;
        }
        let (device, used, available, mount) = (words[0], number(Some(words[2])), number(Some(words[3])), words[5]);
        let skip = ["tmpfs", "devtmpfs", "squashfs", "overlay", "efivarfs"].contains(&device)
            || mount.starts_with("/dev") || mount.starts_with("/run") || mount.starts_with("/sys") || mount.starts_with("/proc") || mount.starts_with("/boot/efi");
        if skip || used + available == 0 {
            continue;
        }
        sample.filesystems.push(Filesystem { mount: mount.to_string(), used: used as f32 / (used + available) as f32 });
    }
    sample.filesystems.sort_by(|a, b| b.used.total_cmp(&a.used));
    sample.filesystems.truncate(FILESYSTEMS);

    for line in get("conn") {
        // ss: `ESTAB 0 0 local:port peer:port`; netstat: `tcp 0 0 local:port peer:port STATE`.
        let Some(peer) = line.split_whitespace().nth(4) else { continue };
        let host = peer.trim_start_matches('[');
        let Some((host, _)) = host.rsplit_once(':') else { continue };
        let host = host.trim_end_matches(']');
        if let Ok(ip) = host.parse::<IpAddr>() {
            if crate::geo::is_public(&ip) && !sample.connections.contains(&ip) && sample.connections.len() < MAX_CONNECTIONS {
                sample.connections.push(ip);
            }
        }
    }
    Some(sample)
}

/// What the panels show of the host: the last sample with its rates and histories.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub host: String,
    pub os: String,
    pub kernel: String,
    pub uptime: u64,
    pub cpu_hist: VecDeque<f32>,
    pub cores: Vec<f32>,
    pub core_hist: Vec<VecDeque<f32>>,
    pub mem_hist: VecDeque<f32>,
    pub load: Option<[f32; 3]>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub top: Vec<ProcInfo>,
    pub iface: String,
    pub ip: String,
    pub rx_hist: VecDeque<f32>,
    pub tx_hist: VecDeque<f32>,
    pub rx_total: u64,
    pub tx_total: u64,
    pub filesystems: Vec<Filesystem>,
    pub connections: Vec<IpAddr>,
}

impl Default for View {
    fn default() -> Self {
        View {
            host: String::new(),
            os: String::new(),
            kernel: String::new(),
            uptime: 0,
            cpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            cores: Vec::new(),
            core_hist: Vec::new(),
            mem_hist: VecDeque::from(vec![0.0; HISTORY]),
            load: None,
            mem_used: 0,
            mem_total: 0,
            swap_used: 0,
            swap_total: 0,
            top: Vec::new(),
            iface: String::new(),
            ip: String::new(),
            rx_hist: VecDeque::from(vec![0.0; HISTORY]),
            tx_hist: VecDeque::from(vec![0.0; HISTORY]),
            rx_total: 0,
            tx_total: 0,
            filesystems: Vec::new(),
            connections: Vec::new(),
        }
    }
}

fn push(hist: &mut VecDeque<f32>, value: f32) {
    hist.push_back(value);
    while hist.len() > HISTORY {
        hist.pop_front();
    }
}

fn percent(before: (u64, u64), after: (u64, u64)) -> f32 {
    let total = after.1.saturating_sub(before.1);
    if total == 0 {
        return 0.0;
    }
    (after.0.saturating_sub(before.0) as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

impl View {
    /// Takes in the next sample. Rates need two samples: the first one fills the
    /// readings that stand on their own and leaves the rates at zero.
    pub fn advance(&mut self, before: Option<&Sample>, after: &Sample, elapsed: Duration) {
        self.host = after.hostname.clone();
        self.os = after.os.clone();
        self.kernel = after.kernel.clone();
        self.uptime = after.uptime;
        self.load = after.load;
        self.mem_total = after.mem_total;
        self.mem_used = after.mem_used;
        self.swap_total = after.swap_total;
        self.swap_used = after.swap_used;
        push(&mut self.mem_hist, after.mem_used as f32 / after.mem_total.max(1) as f32 * 100.0);
        self.filesystems = after.filesystems.clone();
        self.connections = after.connections.clone();
        let ncores = after.cpu.len().saturating_sub(1).max(1) as f32;
        self.top = after.top.iter().map(|p| ProcInfo { name: p.name.clone(), cpu: p.cpu / ncores, mem: p.mem }).collect();

        let seconds = elapsed.as_secs_f32().max(0.001);
        match before {
            Some(before) if before.cpu.len() == after.cpu.len() => {
                push(&mut self.cpu_hist, percent(before.cpu[0], after.cpu[0]));
                self.cores = before.cpu[1..].iter().zip(&after.cpu[1..]).map(|(b, a)| percent(*b, *a)).collect();
            }
            _ => {
                push(&mut self.cpu_hist, 0.0);
                self.cores = vec![0.0; after.cpu.len().saturating_sub(1)];
            }
        }
        self.core_hist.resize_with(self.cores.len(), || VecDeque::from(vec![0.0; HISTORY]));
        for (hist, usage) in self.core_hist.iter_mut().zip(&self.cores) {
            push(hist, *usage);
        }

        // The busiest interface stands for the machine, as it does at home.
        let busiest = after.net.iter().max_by_key(|(_, rx, tx)| rx + tx);
        match busiest {
            Some((name, rx, tx)) => {
                let earlier = before.and_then(|b| b.net.iter().find(|(n, _, _)| n == name));
                let (rx_rate, tx_rate) = match earlier {
                    Some((_, rx0, tx0)) => (rx.saturating_sub(*rx0) as f32 / seconds, tx.saturating_sub(*tx0) as f32 / seconds),
                    None => (0.0, 0.0),
                };
                self.iface = name.clone();
                self.rx_total = *rx;
                self.tx_total = *tx;
                push(&mut self.rx_hist, rx_rate);
                push(&mut self.tx_hist, tx_rate);
            }
            None => {
                self.iface.clear();
                push(&mut self.rx_hist, 0.0);
                push(&mut self.tx_hist, 0.0);
            }
        }
        self.ip = after.ip.clone();
    }
}

/// What one `ssh` call came back with.
#[derive(Debug, PartialEq)]
enum Outcome {
    Exited { code: i32, stdout: String, stderr: String },
    TimedOut,
}

/// Runs a program with a deadline, keeping what it prints on both channels up to a
/// bound; on overrun it is killed.
fn run_capturing(program: &str, args: &[String], timeout: Duration) -> Outcome {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return Outcome::Exited { code: 127, stdout: String::new(), stderr: "no ssh".into() };
    };
    let stdout = read_bounded(child.stdout.take());
    let stderr = read_bounded(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::TimedOut;
            }
        }
    };
    Outcome::Exited {
        code,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    }
}

/// Reads a pipe to its end on a thread of its own, keeping at most [`MAX_OUTPUT`].
fn read_bounded<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(MAX_OUTPUT as u64).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// What an outcome means, and the sample when there is one.
fn classify(outcome: &Outcome) -> (State, Option<Sample>) {
    match outcome {
        Outcome::TimedOut => (State::Unreachable, None),
        Outcome::Exited { code: 0, stdout, .. } => match parse_sample(stdout) {
            Some(sample) => (State::Live, Some(sample)),
            None => (State::NotLinux, None),
        },
        Outcome::Exited { stderr, .. } => {
            let refused = ["Permission denied", "Host key verification failed", "Too many authentication failures"];
            if refused.iter().any(|s| stderr.contains(s)) { (State::NoKey, None) } else { (State::Unreachable, None) }
        }
    }
}

/// Where the control sockets live: ours alone, made if missing.
fn control_dir() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("des-ui-{}", rustix::process::geteuid().as_raw())))
        .join("des-ui");
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
    dir
}

struct Shared {
    state: State,
    view: View,
    previous: Option<(Sample, Instant)>,
}

/// Follows the host the terminal in use is on.
pub struct Remote {
    target: Option<Target>,
    shared: Option<Arc<Mutex<Shared>>>,
    stop: Option<mpsc::Sender<()>>,
    checked: Option<Instant>,
}

impl Default for Remote {
    fn default() -> Self {
        Self::new()
    }
}

impl Remote {
    pub fn new() -> Remote {
        Remote { target: None, shared: None, stop: None, checked: None }
    }

    /// Looks at the terminal in use, at most twice a second, and follows wherever its
    /// foreground `ssh` went; stops when that ends or another tab takes its place.
    pub fn follow_tab(&mut self, tab: Option<&TermTab>, ctx: &egui::Context) {
        if self.checked.is_some_and(|at| at.elapsed() < CHECK_TAB_EVERY) {
            return;
        }
        self.checked = Some(Instant::now());
        let target = tab.and_then(TermTab::foreground_command_line).and_then(|argv| parse_ssh(&argv));
        self.follow(target, ctx);
    }

    /// Follows this host, or none. Nothing happens when it is the one already followed.
    pub fn follow(&mut self, target: Option<Target>, ctx: &egui::Context) {
        if target == self.target {
            return;
        }
        self.stop = None;
        self.shared = None;
        self.target = target.clone();
        let Some(target) = target else { return };
        let shared = Arc::new(Mutex::new(Shared { state: State::Connecting, view: View::default(), previous: None }));
        let (stop, stopped) = mpsc::channel();
        let writer = shared.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let dir = control_dir();
            let args = target.ssh_args(&dir);
            loop {
                let mut call = args.clone();
                call.push(format!("sh -c '{SCRIPT}'"));
                let asked = Instant::now();
                let outcome = run_capturing("ssh", &call, TIMEOUT);
                if matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                    break;
                }
                let (state, sample) = classify(&outcome);
                if let Ok(mut shared) = writer.lock() {
                    shared.state = state;
                    if let Some(sample) = sample {
                        let elapsed = shared.previous.as_ref().map_or(EVERY, |(_, at)| asked.duration_since(*at));
                        let before = shared.previous.take().map(|(sample, _)| sample);
                        shared.view.advance(before.as_ref(), &sample, elapsed);
                        shared.previous = Some((sample, asked));
                    } else {
                        shared.previous = None;
                    }
                }
                ctx.request_repaint();
                let wait = if state == State::Live { EVERY } else { RETRY_AFTER };
                if stopped.recv_timeout(wait) != Err(mpsc::RecvTimeoutError::Timeout) {
                    break;
                }
            }
            // The control connection has no more to do; close it rather than wait a minute.
            let mut exit = args.clone();
            exit.insert(0, "exit".into());
            exit.insert(0, "-O".into());
            let _ = run_capturing("ssh", &exit, Duration::from_secs(2));
        });
        self.shared = Some(shared);
        self.stop = Some(stop);
    }

    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    /// How following stands; `None` when nothing is followed.
    pub fn state(&self) -> Option<State> {
        self.shared.as_ref().and_then(|shared| shared.lock().ok().map(|shared| shared.state))
    }

    /// The host as the panels should show it, once it has answered.
    pub fn view(&self) -> Option<View> {
        let shared = self.shared.as_ref()?.lock().ok()?;
        (shared.state == State::Live && !shared.view.host.is_empty()).then(|| shared.view.clone())
    }
}

use std::os::unix::fs::DirBuilderExt;

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &str) -> Vec<String> {
        words.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn reads_where_ssh_went() {
        let target = parse_ssh(&argv("ssh -v -p 2222 -i ~/.ssh/id_ed25519 -o StrictHostKeyChecking=no deploy@build.example.net")).unwrap();
        assert_eq!(target, Target { user: Some("deploy".into()), host: "build.example.net".into(), port: Some(2222) });
        assert_eq!(target.name(), "deploy@build.example.net");
        assert_eq!(parse_ssh(&argv("/usr/bin/ssh -l root -p2222 box uptime")).unwrap(), Target { user: Some("root".into()), host: "box".into(), port: Some(2222) });
        assert_eq!(parse_ssh(&argv("ssh ssh://me@host:22")).unwrap(), Target { user: Some("me".into()), host: "host".into(), port: Some(22) });
        assert_eq!(parse_ssh(&argv("ssh -- host")).unwrap().host, "host");
        assert_eq!(parse_ssh(&argv("ssh [fe80::1]")).unwrap().host, "fe80::1");
    }

    #[test]
    fn ignores_tunnels_and_other_programs() {
        assert_eq!(parse_ssh(&argv("ssh -N -L 8080:localhost:80 host")), None);
        assert_eq!(parse_ssh(&argv("ssh -W host:22 jump")), None);
        assert_eq!(parse_ssh(&argv("scp file host:")), None);
        assert_eq!(parse_ssh(&argv("ssh")), None);
        assert_eq!(parse_ssh(&argv("ssh -p")), None);
        assert_eq!(parse_ssh(&argv("ssh -- -evil")), None);
    }

    const FIRST: &str = "=hostname\nbuild\n=os\n\"Debian GNU/Linux 13 (trixie)\"\n6.12.0-amd64\n=uptime\n86400.50 170000.00\n=stat\ncpu  1000 0 500 8000 500 0 0 0 0 0\ncpu0 500 0 250 4000 250 0 0 0 0 0\ncpu1 500 0 250 4000 250 0 0 0 0 0\n=meminfo\nMemTotal:        8000000 kB\nMemFree:         1000000 kB\nMemAvailable:    4000000 kB\nSwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n=loadavg\n0.50 0.40 0.30 1/200 1234\n=netdev\nInter-|   Receive\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 100 1 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n  eth0: 1000000 10 0 0 0 0 0 0 500000 5 0 0 0 0 0 0\n=ip\n10.0.0.7 fd00::7\n=ps\n  123 50.0 102400 cargo\n  456  2.5  8192 sshd: me\n=df\n/dev/sda1 100000 60000 40000 60% /\ntmpfs 1000 10 990 1% /run\n/dev/sdb1 100 90 10 90% /data\n=conn\nESTAB 0 0 10.0.0.7:22 203.0.113.5:51234\nESTAB 0 0 [fd00::7]:443 [2001:db8::9]:1234\nESTAB 0 0 10.0.0.7:5432 10.0.0.9:4000\n=end\n";

    fn second() -> String {
        FIRST
            .replace("cpu  1000 0 500 8000 500", "cpu  1600 0 800 8900 500")
            .replace("cpu0 500 0 250 4000 250", "cpu0 1100 0 550 4150 250")
            .replace("eth0: 1000000 10 0 0 0 0 0 0 500000", "eth0: 3000000 10 0 0 0 0 0 0 600000")
    }

    #[test]
    fn reads_what_the_host_printed() {
        let sample = parse_sample(FIRST).unwrap();
        assert_eq!(sample.hostname, "build");
        assert_eq!(sample.os, "Debian GNU/Linux 13 (trixie)");
        assert_eq!(sample.kernel, "6.12.0-amd64");
        assert_eq!(sample.uptime, 86400);
        assert_eq!(sample.cpu.len(), 3);
        assert_eq!(sample.mem_total, 8000000 * 1024);
        assert_eq!(sample.mem_used, 4000000 * 1024);
        assert_eq!(sample.swap_used, 500000 * 1024);
        assert_eq!(sample.load, Some([0.5, 0.4, 0.3]));
        assert_eq!(sample.net, vec![("eth0".to_string(), 1000000, 500000)]);
        assert_eq!(sample.ip, "10.0.0.7");
        assert_eq!(sample.top[0].name, "cargo");
        assert_eq!(sample.top[0].mem, 102400 * 1024);
        assert_eq!(sample.top[1].name, "sshd: me");
        assert_eq!(sample.filesystems.iter().map(|f| f.mount.as_str()).collect::<Vec<_>>(), vec!["/data", "/"], "fullest first, tmpfs left out");
        assert_eq!(sample.connections.len(), 2, "private peers are not links");
        assert!(parse_sample("=hostname\nx\n=end\n").is_none(), "no processor, not Linux");
        assert!(parse_sample(&FIRST.replace("=end", "")).is_none(), "cut short");
    }

    #[test]
    fn two_samples_make_rates() {
        let first = parse_sample(FIRST).unwrap();
        let later = parse_sample(&second()).unwrap();
        let mut view = View::default();
        view.advance(None, &first, Duration::from_secs(2));
        assert_eq!(view.cpu_hist.back(), Some(&0.0), "no rate from one sample");
        assert_eq!(view.cores.len(), 2);
        view.advance(Some(&first), &later, Duration::from_secs(2));
        // 900 more busy jiffies out of 1800 more in all.
        assert!((view.cpu_hist.back().unwrap() - 50.0).abs() < 0.1);
        // Core 0: 900 more busy jiffies out of 1050 more in all.
        assert!((view.cores[0] - 85.714).abs() < 0.01, "{:?}", view.cores);
        assert_eq!(view.cores[1], 0.0);
        assert_eq!(view.iface, "eth0");
        assert!((view.rx_hist.back().unwrap() - 1_000_000.0).abs() < 1.0, "2 MB in 2 s");
        assert!((view.tx_hist.back().unwrap() - 50_000.0).abs() < 1.0);
        assert_eq!(view.rx_total, 3000000);
        assert_eq!(view.host, "build");
        assert_eq!(view.cpu_hist.len(), HISTORY);
        assert_eq!(view.core_hist.len(), 2);
        assert_eq!(view.top[0].cpu, 25.0, "per-core percent shared over the two cores");
    }

    #[test]
    fn tells_a_refusal_from_no_answer() {
        let refused = Outcome::Exited { code: 255, stdout: String::new(), stderr: "me@host: Permission denied (publickey,password).\n".into() };
        assert_eq!(classify(&refused).0, State::NoKey);
        let down = Outcome::Exited { code: 255, stdout: String::new(), stderr: "ssh: connect to host box port 22: Connection refused\n".into() };
        assert_eq!(classify(&down).0, State::Unreachable);
        assert_eq!(classify(&Outcome::TimedOut).0, State::Unreachable);
        let odd = Outcome::Exited { code: 0, stdout: "Welcome to the switch\n".into(), stderr: String::new() };
        assert_eq!(classify(&odd).0, State::NotLinux);
        let fine = Outcome::Exited { code: 0, stdout: FIRST.into(), stderr: String::new() };
        let (state, sample) = classify(&fine);
        assert_eq!(state, State::Live);
        assert_eq!(sample.unwrap().hostname, "build");
    }

    #[test]
    fn a_slow_program_is_cut_off() {
        let outcome = run_capturing("sh", &["-c".to_string(), "sleep 5".to_string()], Duration::from_millis(100));
        assert_eq!(outcome, Outcome::TimedOut);
        let outcome = run_capturing("sh", &["-c".to_string(), "echo out; echo err >&2; exit 3".to_string()], Duration::from_secs(2));
        assert_eq!(outcome, Outcome::Exited { code: 3, stdout: "out\n".into(), stderr: "err\n".into() });
    }

    #[test]
    fn following_the_same_host_again_keeps_the_worker() {
        let ctx = egui::Context::default();
        let mut remote = Remote::new();
        assert_eq!(remote.state(), None);
        remote.follow(parse_ssh(&argv("ssh nowhere.invalid")), &ctx);
        assert!(matches!(remote.state(), Some(State::Connecting | State::Unreachable | State::NoKey)));
        let shared = remote.shared.as_ref().map(Arc::as_ptr);
        remote.follow(parse_ssh(&argv("ssh nowhere.invalid")), &ctx);
        assert_eq!(remote.shared.as_ref().map(Arc::as_ptr), shared, "the same target changes nothing");
        remote.follow(None, &ctx);
        assert_eq!(remote.state(), None);
        assert!(remote.view().is_none());
    }
}
