//! Programs that start with the session, as the XDG autostart rules say: the entries in
//! `~/.config/autostart` and in each `$XDG_CONFIG_DIRS/autostart`, a user's file taking
//! the place of a system file with the same name.
//!
//! They are started once per login session, not again when the shell restarts, on a
//! thread of their own so that a delay holds up nothing. Settings → Session lists them
//! and turns them off the way the rules provide: a user file with `Hidden=true`.

use crate::apps::{self, AppEntry};
use std::collections::BTreeMap;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// This desktop's name in `OnlyShowIn` and `NotShowIn`, as the session sets
/// `XDG_CURRENT_DESKTOP` (session/des-ui-session).
pub const DESKTOP: &str = "des-ui";

/// The longest wait an entry may ask for before it starts.
const MAX_DELAY: Duration = Duration::from_secs(60);

/// Pieces of other desktops that start themselves everywhere unless told not to. This
/// desktop has its own of each, and a second polkit agent, notification daemon or tray
/// would fight it for the same D-Bus name, or show a panel of another desktop on top.
const OTHER_DESKTOPS: &[&str] = &[
    // Authentication agents: the shell is the polkit agent.
    "polkit-gnome-authentication-agent-1",
    "polkit-kde-authentication-agent-1",
    "polkit-mate-authentication-agent-1",
    "lxpolkit",
    "lxqt-policykit-agent",
    "xfce-polkit",
    // Notification daemons: the shell owns org.freedesktop.Notifications.
    "xfce4-notifyd",
    "mate-notification-daemon",
    "notification-daemon",
    "dunst",
    "mako",
    // Panels and shells of other desktops, and their tray bridges.
    "plasmashell",
    "xembedsniproxy",
    "xfce4-panel",
    "lxpanel",
    "mate-panel",
];

/// As `OTHER_DESKTOPS`, for families whose members all have one prefix.
const OTHER_DESKTOP_PREFIXES: &[&str] = &["gnome-shell", "org.gnome.Shell", "org.gnome.SettingsDaemon.", "gsd-"];

/// One autostart entry as read from its file.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The file's name, which is what a user file overrides by.
    pub file_name: String,
    pub name: String,
    pub exec: Vec<String>,
    pub terminal: bool,
    pub hidden: bool,
    pub only_show_in: Option<Vec<String>>,
    pub not_show_in: Vec<String>,
    pub try_exec: Option<String>,
    /// `X-GNOME-Autostart-enabled`, which GNOME's own tools write instead of `Hidden`.
    pub gnome_enabled: bool,
    pub delay: Duration,
    /// `X-GNOME-Autostart-Phase`, ranked so that earlier phases start first.
    pub phase: u8,
    /// Whether the file in force is the user's own.
    pub user: bool,
}

/// Why an entry is not started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Skip {
    /// Turned off, by `Hidden=true` or `X-GNOME-Autostart-enabled=false`.
    Off,
    /// For other desktops only.
    OnlyShowIn,
    NotShowIn,
    /// The program it depends on is not installed.
    TryExec,
    /// Another desktop's own component, which this desktop replaces.
    OtherDesktop,
    /// It wants a terminal of its own, which the session start cannot open.
    Terminal,
}

impl Skip {
    pub fn reason(self) -> &'static str {
        match self {
            Skip::Off => "Off",
            Skip::OnlyShowIn => "For another desktop",
            Skip::NotShowIn => "Not for this desktop",
            Skip::TryExec => "Not installed",
            Skip::OtherDesktop => "Replaced by this desktop",
            Skip::Terminal => "Needs a terminal",
        }
    }
}

/// Where the entries are: the user's directory, then the system's, most specific first.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub user: PathBuf,
    pub system: Vec<PathBuf>,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let config_dirs = std::env::var("XDG_CONFIG_DIRS")
            .ok()
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/etc/xdg".to_string());
        Dirs {
            user: config_home.join("autostart"),
            system: config_dirs
                .split(':')
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("autostart"))
                .collect(),
        }
    }

    /// The first system file of this name, which a user file would override.
    fn system_file(&self, file_name: &str) -> Option<PathBuf> {
        self.system.iter().map(|dir| dir.join(file_name)).find(|path| path.is_file())
    }
}

/// Reads one autostart entry. `None` for what is not an application to run.
pub fn parse(text: &str, file_name: &str) -> Option<Entry> {
    let mut entry = Entry {
        file_name: file_name.to_string(),
        name: String::new(),
        exec: Vec::new(),
        terminal: false,
        hidden: false,
        only_show_in: None,
        not_show_in: Vec::new(),
        try_exec: None,
        gnome_enabled: true,
        delay: Duration::ZERO,
        phase: phase_rank("Applications"),
        user: false,
    };
    let mut in_main_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !in_main_group {
            continue;
        }
        let value = value.trim();
        match key.trim() {
            "Type" if value != "Application" => return None,
            "Name" => entry.name = value.to_string(),
            "Exec" => entry.exec = apps::split_exec(value),
            "Terminal" => entry.terminal = value == "true",
            "Hidden" => entry.hidden = value == "true",
            "OnlyShowIn" => entry.only_show_in = Some(desktops(value)),
            "NotShowIn" => entry.not_show_in = desktops(value),
            "TryExec" if !value.is_empty() => entry.try_exec = Some(value.to_string()),
            "X-GNOME-Autostart-enabled" => entry.gnome_enabled = value != "false",
            "X-GNOME-Autostart-Delay" => {
                if let Ok(seconds) = value.parse::<f64>() {
                    if seconds.is_finite() && seconds > 0.0 {
                        entry.delay = Duration::from_secs_f64(seconds).min(MAX_DELAY);
                    }
                }
            }
            "X-GNOME-Autostart-Phase" => entry.phase = phase_rank(value),
            _ => {}
        }
    }
    if entry.exec.is_empty() {
        return None;
    }
    if entry.name.is_empty() {
        entry.name = entry.file_name.trim_end_matches(".desktop").to_string();
    }
    Some(entry)
}

fn desktops(value: &str) -> Vec<String> {
    value.split(';').map(str::trim).filter(|name| !name.is_empty()).map(str::to_string).collect()
}

/// GNOME's session phases, in the order it starts them.
fn phase_rank(phase: &str) -> u8 {
    match phase {
        "EarlyInitialization" => 0,
        "PreDisplayServer" => 1,
        "Initialization" => 2,
        "WindowManager" => 3,
        "Panel" => 4,
        "Desktop" => 5,
        _ => 6,
    }
}

/// Whether `entry` is started here, given a way to tell if a program is installed.
pub fn verdict(entry: &Entry, desktop: &str, installed: impl Fn(&str) -> bool) -> Result<(), Skip> {
    if entry.hidden || !entry.gnome_enabled {
        return Err(Skip::Off);
    }
    let named = |names: &[String]| names.iter().any(|name| name.eq_ignore_ascii_case(desktop));
    if entry.only_show_in.as_ref().is_some_and(|names| !named(names)) {
        return Err(Skip::OnlyShowIn);
    }
    if named(&entry.not_show_in) {
        return Err(Skip::NotShowIn);
    }
    if entry.try_exec.as_deref().is_some_and(|program| !installed(program)) {
        return Err(Skip::TryExec);
    }
    if is_other_desktop(entry) {
        return Err(Skip::OtherDesktop);
    }
    if entry.terminal {
        return Err(Skip::Terminal);
    }
    Ok(())
}

fn is_other_desktop(entry: &Entry) -> bool {
    let stem = entry.file_name.trim_end_matches(".desktop");
    let program = entry.exec.first().map(|program| program.rsplit('/').next().unwrap_or(program)).unwrap_or("");
    [stem, program].iter().any(|name| {
        OTHER_DESKTOPS.contains(name) || OTHER_DESKTOP_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
    })
}

/// Whether `program` runs: an absolute path to an executable file, or a name on `PATH`.
pub fn installed(program: &str) -> bool {
    let executable = |path: &Path| path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        return Path::new(program).is_absolute() && executable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(program))))
}

/// Every entry in force, the user's file in place of a system file of the same name,
/// sorted by name.
pub fn scan(dirs: &Dirs) -> Vec<Entry> {
    let mut found: BTreeMap<String, Entry> = BTreeMap::new();
    let sources = std::iter::once((&dirs.user, true)).chain(dirs.system.iter().map(|dir| (dir, false)));
    for (dir, user) in sources {
        let Ok(listing) = std::fs::read_dir(dir) else {
            continue;
        };
        for file in listing.flatten() {
            let file_name = file.file_name().to_string_lossy().into_owned();
            if !file_name.ends_with(".desktop") || found.contains_key(&file_name) {
                continue;
            }
            // A file in an earlier directory wins even when it does not parse: a user
            // file that is not an application hides the system one all the same.
            let entry = std::fs::read_to_string(file.path()).ok().and_then(|text| parse(&text, &file_name));
            let entry = entry.map(|entry| Entry { user, ..entry }).unwrap_or_else(|| Entry {
                file_name: file_name.clone(),
                name: file_name.trim_end_matches(".desktop").to_string(),
                exec: Vec::new(),
                terminal: false,
                hidden: true,
                only_show_in: None,
                not_show_in: Vec::new(),
                try_exec: None,
                gnome_enabled: true,
                delay: Duration::ZERO,
                phase: 6,
                user,
            });
            found.insert(file_name, entry);
        }
    }
    let mut entries: Vec<Entry> = found.into_values().collect();
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

/// What starts at login, in order: earlier phases first, then shorter delays.
pub fn plan(entries: &[Entry], desktop: &str, installed: impl Fn(&str) -> bool) -> Vec<Entry> {
    let mut runnable: Vec<Entry> = entries.iter().filter(|entry| verdict(entry, desktop, &installed).is_ok()).cloned().collect();
    runnable.sort_by_key(|entry| (entry.phase, entry.delay));
    runnable
}

/// One line of the list in Settings → Session.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub file_name: String,
    pub name: String,
    /// Whether the user has it on; what the switch shows.
    pub on: bool,
    /// Why it would not start even when on, other than being off.
    pub blocked: Option<Skip>,
}

pub fn list(dirs: &Dirs) -> Vec<Listed> {
    scan(dirs)
        .into_iter()
        // A file that hides a system entry without naming a program is still listed.
        .filter(|entry| !entry.exec.is_empty() || dirs.system_file(&entry.file_name).is_some())
        .map(|entry| {
            let on = !entry.hidden && entry.gnome_enabled;
            let blocked = match verdict(&entry, DESKTOP, installed) {
                Ok(()) | Err(Skip::Off) => None,
                Err(skip) => Some(skip),
            };
            Listed { file_name: entry.file_name, name: entry.name, on, blocked }
        })
        .collect()
}

/// Sets `key` in the `[Desktop Entry]` group, adding it at the group's end when missing.
fn set_key(text: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let (mut in_main, mut done) = (false, false);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_main && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_main = trimmed == "[Desktop Entry]";
        } else if in_main && trimmed.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !text.lines().any(|line| line.trim() == "[Desktop Entry]") {
            out.insert(0, "[Desktop Entry]".to_string());
        }
        // The group runs to the end of the file, or the key goes after the last group.
        out.push(format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

/// The text without `key` in its main group, for comparing a user file with the system
/// file it came from.
fn without_key(text: &str, key: &str) -> String {
    let mut in_main = false;
    text.lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_main = trimmed == "[Desktop Entry]";
                return true;
            }
            !(in_main && trimmed.split_once('=').is_some_and(|(k, _)| k.trim() == key))
        })
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Turns an entry off or on. Off writes the user's file with `Hidden=true`, copying the
/// system file when there is no user file yet. On removes a user file that was only
/// that, and otherwise clears `Hidden` and GNOME's switch in it.
pub fn set_enabled(dirs: &Dirs, file_name: &str, on: bool) -> io::Result<()> {
    if file_name.contains('/') || !file_name.ends_with(".desktop") {
        return Err(io::Error::other("not an autostart entry"));
    }
    let user_path = dirs.user.join(file_name);
    let system_text = dirs.system_file(file_name).map(std::fs::read_to_string).transpose()?;
    let user_text = match std::fs::read_to_string(&user_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if on {
        let Some(user_text) = user_text else {
            return Ok(());
        };
        if let Some(system_text) = &system_text {
            let stripped = without_key(&without_key(&user_text, "Hidden"), "X-GNOME-Autostart-enabled");
            if stripped == without_key(&without_key(system_text, "Hidden"), "X-GNOME-Autostart-enabled") && !parse(system_text, file_name).is_some_and(|entry| entry.hidden) {
                return std::fs::remove_file(&user_path);
            }
        }
        let text = set_key(&user_text, "Hidden", "false");
        let text = if parse(&text, file_name).is_some_and(|entry| !entry.gnome_enabled) { set_key(&text, "X-GNOME-Autostart-enabled", "true") } else { text };
        return write_atomically(&user_path, &text);
    }
    let base = user_text.or(system_text).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such autostart entry"))?;
    std::fs::create_dir_all(&dirs.user)?;
    write_atomically(&user_path, &set_key(&base, "Hidden", "true"))
}

/// Adds a user entry that runs `command_line`, split like an `Exec` line and never given
/// to a shell. Returns the new file.
pub fn add(dirs: &Dirs, command_line: &str) -> io::Result<PathBuf> {
    let command_line = command_line.trim();
    if command_line.is_empty() || command_line.chars().any(char::is_control) {
        return Err(io::Error::other("enter one command line"));
    }
    let argv = apps::split_exec(command_line);
    let program = argv.first().ok_or_else(|| io::Error::other("enter one command line"))?;
    let base = program.rsplit('/').next().unwrap_or(program);
    let stem: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();
    let stem = if stem.trim_matches('-').is_empty() { "command".to_string() } else { stem };
    std::fs::create_dir_all(&dirs.user)?;
    let path = (0..100)
        .map(|n| dirs.user.join(if n == 0 { format!("{stem}.desktop") } else { format!("{stem}-{n}.desktop") }))
        .find(|path| !path.exists())
        .ok_or_else(|| io::Error::other("too many entries with that name"))?;
    let text = format!("[Desktop Entry]\nType=Application\nName={base}\nExec={command_line}\nX-DEs-UI-Added=true\n");
    write_atomically(&path, &text)?;
    Ok(path)
}

fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("no directory"))?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut file, text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// What tells one login session from the next: logind's session id, else this boot.
fn session_id() -> Option<String> {
    let id = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok())?;
    let id: String = id.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    (!id.is_empty()).then_some(id)
}

/// Claims this session's start. True only the first time, so a restarted shell does not
/// start everything again.
pub fn claim_once(runtime_dir: &Path, session: &str) -> bool {
    let dir = runtime_dir.join("des-ui");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(format!("autostart-done-{session}"))).is_ok()
}

/// Starts this session's autostart entries, once per login session, on a thread of
/// their own. Call when the shell runs as the session, not in a window.
pub fn start() {
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_absolute()) else {
        eprintln!("des-ui: no XDG_RUNTIME_DIR; autostart entries are not started");
        return;
    };
    let Some(session) = session_id() else {
        eprintln!("des-ui: no session id; autostart entries are not started");
        return;
    };
    if !claim_once(&runtime_dir, &session) {
        return;
    }
    let entries = plan(&scan(&Dirs::from_env()), DESKTOP, installed);
    std::thread::spawn(move || run(entries, std::thread::sleep, |entry| apps::launch(entry)));
}

/// Launches each entry when its delay is up, counted from the start of the session.
fn run(entries: Vec<Entry>, mut sleep: impl FnMut(Duration), mut launch: impl FnMut(&AppEntry) -> io::Result<()>) {
    let mut waited = Duration::ZERO;
    let mut entries = entries;
    entries.sort_by_key(|entry| (entry.delay, entry.phase));
    for entry in entries {
        if entry.delay > waited {
            sleep(entry.delay - waited);
            waited = entry.delay;
        }
        let app = AppEntry {
            name: entry.name.clone(),
            desktop_id: entry.file_name.clone(),
            icon: None,
            startup_wm_class: None,
            exec: entry.exec.clone(),
            terminal: false,
        };
        if let Err(error) = launch(&app) {
            eprintln!("des-ui: autostart {}: {error}", entry.file_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        let dirs = Dirs { user: root.join("user"), system: vec![root.join("system"), root.join("vendor")] };
        for dir in std::iter::once(&dirs.user).chain(&dirs.system) {
            std::fs::create_dir_all(dir).unwrap();
        }
        dirs
    }

    fn entry(text: &str) -> Entry {
        parse(text, "thing.desktop").unwrap()
    }

    const PLAIN: &str = "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing --start\n";

    #[test]
    fn the_user_file_overrides_the_system_one_and_earlier_system_dirs_win() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        std::fs::write(dirs.system[0].join("a.desktop"), PLAIN).unwrap();
        std::fs::write(dirs.system[1].join("a.desktop"), "[Desktop Entry]\nName=Vendor\nExec=vendor\n").unwrap();
        std::fs::write(dirs.system[1].join("b.desktop"), "[Desktop Entry]\nName=Bee\nExec=bee\n").unwrap();
        std::fs::write(dirs.user.join("b.desktop"), "[Desktop Entry]\nName=Mine\nExec=mine\n").unwrap();
        let found = scan(&dirs);
        assert_eq!(found.len(), 2);
        let a = found.iter().find(|entry| entry.file_name == "a.desktop").unwrap();
        assert_eq!((a.name.as_str(), a.user), ("Thing", false));
        let b = found.iter().find(|entry| entry.file_name == "b.desktop").unwrap();
        assert_eq!((b.name.as_str(), b.user), ("Mine", true));
        // A user file that is not an application still hides the system entry.
        std::fs::write(dirs.user.join("a.desktop"), "[Desktop Entry]\nType=Link\n").unwrap();
        let a = scan(&dirs).into_iter().find(|entry| entry.file_name == "a.desktop").unwrap();
        assert!(a.hidden && a.user);
    }

    #[test]
    fn every_rule_decides_what_starts() {
        let yes = |_: &str| true;
        let no = |_: &str| false;
        assert_eq!(verdict(&entry(PLAIN), DESKTOP, yes), Ok(()));
        assert_eq!(verdict(&entry(&format!("{PLAIN}Hidden=true\n")), DESKTOP, yes), Err(Skip::Off));
        assert_eq!(verdict(&entry(&format!("{PLAIN}X-GNOME-Autostart-enabled=false\n")), DESKTOP, yes), Err(Skip::Off));
        assert_eq!(verdict(&entry(&format!("{PLAIN}OnlyShowIn=GNOME;KDE;\n")), DESKTOP, yes), Err(Skip::OnlyShowIn));
        assert_eq!(verdict(&entry(&format!("{PLAIN}OnlyShowIn=GNOME;des-ui;\n")), DESKTOP, yes), Ok(()));
        assert_eq!(verdict(&entry(&format!("{PLAIN}NotShowIn=des-ui;\n")), DESKTOP, yes), Err(Skip::NotShowIn));
        assert_eq!(verdict(&entry(&format!("{PLAIN}NotShowIn=KDE;\n")), DESKTOP, yes), Ok(()));
        assert_eq!(verdict(&entry(&format!("{PLAIN}TryExec=thing\n")), DESKTOP, no), Err(Skip::TryExec));
        assert_eq!(verdict(&entry(&format!("{PLAIN}TryExec=thing\n")), DESKTOP, yes), Ok(()));
        assert_eq!(verdict(&entry(&format!("{PLAIN}Terminal=true\n")), DESKTOP, yes), Err(Skip::Terminal));
        // Not applications, and applications with nothing to run, are not entries at all.
        assert_eq!(parse("[Desktop Entry]\nType=Link\nExec=x\n", "x.desktop"), None);
        assert_eq!(parse("[Desktop Entry]\nName=Empty\n", "x.desktop"), None);
        // Keys outside the main group do not count.
        assert!(!entry(&format!("{PLAIN}[Desktop Action Hide]\nHidden=true\n")).hidden);
    }

    #[test]
    fn delays_are_clamped_and_order_the_start() {
        assert_eq!(entry(&format!("{PLAIN}X-GNOME-Autostart-Delay=5\n")).delay, Duration::from_secs(5));
        assert_eq!(entry(&format!("{PLAIN}X-GNOME-Autostart-Delay=3600\n")).delay, MAX_DELAY);
        assert_eq!(entry(&format!("{PLAIN}X-GNOME-Autostart-Delay=-2\n")).delay, Duration::ZERO);
        assert_eq!(entry(&format!("{PLAIN}X-GNOME-Autostart-Delay=nan\n")).delay, Duration::ZERO);

        let late = Entry { file_name: "late.desktop".into(), delay: Duration::from_secs(10), ..entry(PLAIN) };
        let soon = Entry { file_name: "soon.desktop".into(), delay: Duration::from_secs(2), ..entry(PLAIN) };
        let now = Entry { file_name: "now.desktop".into(), ..entry(PLAIN) };
        let (mut slept, mut started) = (Vec::new(), Vec::new());
        run(vec![late, soon, now], |d| slept.push(d), |app| {
            started.push(app.desktop_id.clone());
            Ok(())
        });
        assert_eq!(started, ["now.desktop", "soon.desktop", "late.desktop"]);
        // Waits add up to each entry's delay from the start, not after the one before.
        assert_eq!(slept, [Duration::from_secs(2), Duration::from_secs(8)]);
    }

    #[test]
    fn other_desktops_components_are_left_out() {
        let named = |file: &str, exec: &str| Entry { file_name: file.into(), exec: vec![exec.into()], ..entry(PLAIN) };
        let yes = |_: &str| true;
        assert_eq!(verdict(&named("polkit-gnome.desktop", "/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1"), DESKTOP, yes), Err(Skip::OtherDesktop));
        assert_eq!(verdict(&named("dunst.desktop", "dunst"), DESKTOP, yes), Err(Skip::OtherDesktop));
        assert_eq!(verdict(&named("org.gnome.Shell.desktop", "something"), DESKTOP, yes), Err(Skip::OtherDesktop));
        assert_eq!(verdict(&named("gsd-power.desktop", "/usr/libexec/gsd-power"), DESKTOP, yes), Err(Skip::OtherDesktop));
        assert_eq!(verdict(&named("nextcloud.desktop", "nextcloud"), DESKTOP, yes), Ok(()));
        let planned = plan(&[named("mako.desktop", "mako"), named("a.desktop", "a")], DESKTOP, yes);
        assert_eq!(planned.iter().map(|entry| entry.file_name.as_str()).collect::<Vec<_>>(), ["a.desktop"]);
    }

    #[test]
    fn a_session_is_claimed_once() {
        let root = tempfile::tempdir().unwrap();
        assert!(claim_once(root.path(), "c2"));
        assert!(!claim_once(root.path(), "c2"));
        assert!(claim_once(root.path(), "c3"));
    }

    #[test]
    fn turning_a_system_entry_off_and_on_leaves_no_trace() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        std::fs::write(dirs.system[0].join("thing.desktop"), PLAIN).unwrap();
        set_enabled(&dirs, "thing.desktop", false).unwrap();
        let written = std::fs::read_to_string(dirs.user.join("thing.desktop")).unwrap();
        assert!(written.starts_with("[Desktop Entry]\nType=Application\nName=Thing\nExec=thing --start\n"));
        assert!(written.lines().any(|line| line == "Hidden=true"));
        assert!(!list(&dirs)[0].on);

        set_enabled(&dirs, "thing.desktop", true).unwrap();
        assert!(!dirs.user.join("thing.desktop").exists());
        assert!(list(&dirs)[0].on);
    }

    #[test]
    fn turning_on_a_changed_user_file_keeps_the_changes() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        std::fs::write(dirs.system[0].join("thing.desktop"), PLAIN).unwrap();
        std::fs::write(dirs.user.join("thing.desktop"), "[Desktop Entry]\nName=Thing\nExec=thing --quiet\nHidden=true\nX-GNOME-Autostart-enabled=false\n[Desktop Action A]\nName=A\n").unwrap();
        set_enabled(&dirs, "thing.desktop", true).unwrap();
        let text = std::fs::read_to_string(dirs.user.join("thing.desktop")).unwrap();
        let entry = parse(&text, "thing.desktop").unwrap();
        assert!(!entry.hidden && entry.gnome_enabled);
        assert_eq!(entry.exec, ["thing", "--quiet"]);
        assert!(text.contains("[Desktop Action A]\nName=A"));
        // Only a name that is a file in the directory is touched.
        assert!(set_enabled(&dirs, "../config.desktop", false).is_err());
        assert!(set_enabled(&dirs, "missing.desktop", false).is_err());
    }

    #[test]
    fn add_writes_an_entry_that_runs_the_command() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let path = add(&dirs, "/usr/bin/syncthing serve --no-browser").unwrap();
        assert_eq!(path, dirs.user.join("syncthing.desktop"));
        let entry = parse(&std::fs::read_to_string(&path).unwrap(), "syncthing.desktop").unwrap();
        assert_eq!(entry.name, "syncthing");
        assert_eq!(entry.exec, ["/usr/bin/syncthing", "serve", "--no-browser"]);
        assert_eq!(verdict(&entry, DESKTOP, |_| true), Ok(()));
        // A second one gets a name of its own.
        assert_eq!(add(&dirs, "syncthing").unwrap(), dirs.user.join("syncthing-1.desktop"));
        assert!(add(&dirs, "  ").is_err());
        assert!(add(&dirs, "a\nHidden=true").is_err());
    }
}
