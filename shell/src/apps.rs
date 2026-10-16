//! Installed applications: reads the system's desktop entries and starts programs.

use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// One launchable application, from a `.desktop` file.
#[derive(Debug, Clone, PartialEq)]
pub struct AppEntry {
    pub name: String,
    pub desktop_id: String,
    pub icon: Option<String>,
    pub startup_wm_class: Option<String>,
    /// The program and its arguments.
    pub exec: Vec<String>,
    /// Whether it must run inside a terminal.
    pub terminal: bool,
}

/// The directories that hold desktop entries, most specific first.
fn entry_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());

    let mut dirs = vec![data_home.join("applications")];
    dirs.extend(data_dirs.split(':').map(|dir| PathBuf::from(dir).join("applications")));
    // Snap and Flatpak add themselves to XDG_DATA_DIRS only in some sessions.
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.push(data_home.join("flatpak/exports/share/applications"));
    dirs
}

/// Every application the user could start, sorted by name.
pub fn scan() -> Vec<AppEntry> {
    scan_dirs(entry_dirs())
}

fn scan_dirs(dirs: Vec<PathBuf>) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in listing.flatten() {
            let name = file.file_name();
            if !name.to_string_lossy().ends_with(".desktop") {
                continue;
            }
            // An entry in an earlier directory overrides the same file in a later one,
            // even if it only exists to hide the application.
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(mut entry) = std::fs::read_to_string(file.path()).ok().and_then(|text| parse(&text)) {
                entry.desktop_id = name.to_string_lossy().into_owned();
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries.dedup_by(|a, b| a.name == b.name && a.exec == b.exec);
    entries
}

/// Reads one desktop entry. Returns `None` for anything that should not be listed.
pub fn parse(text: &str) -> Option<AppEntry> {
    let (mut name, mut exec, mut terminal) = (None, None, false);
    let (mut icon, mut startup_wm_class) = (None, None);
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
            "NoDisplay" | "Hidden" if value == "true" => return None,
            // Entries meant for one desktop only are that desktop's own settings pages.
            "OnlyShowIn" => return None,
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(split_exec(value)),
            "Terminal" => terminal = value == "true",
            "Icon" if !value.is_empty() => icon = Some(value.to_owned()),
            "StartupWMClass" if !value.is_empty() => startup_wm_class = Some(value.to_owned()),
            _ => {}
        }
    }
    let exec = exec.filter(|exec| !exec.is_empty())?;
    Some(AppEntry {
        name: name?,
        desktop_id: String::new(),
        icon,
        startup_wm_class,
        exec,
        terminal,
    })
}

/// Splits an `Exec` value into arguments, following the desktop entry rules: double
/// quotes group words, a backslash escapes inside them, and `%f`-style placeholders
/// (which stand for files to open) are dropped.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_current = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has_current = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '%' => match chars.next() {
                Some('%') => {
                    current.push('%');
                    has_current = true;
                }
                // A placeholder, or a stray percent sign at the end.
                _ => {}
            },
            c if c.is_whitespace() && !quoted => {
                if has_current {
                    args.push(std::mem::take(&mut current));
                    has_current = false;
                }
            }
            c => {
                current.push(c);
                has_current = true;
            }
        }
    }
    if has_current {
        args.push(current);
    }
    // Dropping a placeholder can leave an empty word behind, as in `--name=%c`.
    args.retain(|arg| !arg.is_empty());
    args
}

/// Starts an application on its own, detached from the shell.
pub fn launch(entry: &AppEntry) -> std::io::Result<()> {
    let (program, args) = entry
        .exec
        .split_first()
        .ok_or_else(|| std::io::Error::other("nothing to run"))?;
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    // What the application prints is kept, to explain a window that never appears.
    match output_log(&entry.exec.join(" ")) {
        Some((out, err)) => command.stdout(out).stderr(err),
        None => command.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    command
        // Its own process group: closing a terminal tab must not take it down.
        .process_group(0);
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    let mut child = command.spawn()?;
    // Collect the exit status when it comes, so no zombie process is left behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens `apps.log` in the state directory for a new application's output, starting
/// the file over once it has grown large.
fn output_log(command_line: &str) -> Option<(Stdio, Stdio)> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    let dir = state_home.join("des-ui");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("apps.log");
    let large = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 1024 * 1024);
    let mut file = OpenOptions::new()
        .create(true)
        .append(!large)
        .write(true)
        .truncate(large)
        .open(path)
        .ok()?;
    let _ = writeln!(file, "\n=== {} {command_line}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    Some((Stdio::from(file.try_clone().ok()?), Stdio::from(file)))
}

/// Open a local document without a shell and report the viewer's exit asynchronously.
pub fn open_document(path:PathBuf,ctx:eframe::egui::Context)->std::sync::mpsc::Receiver<Result<(),String>>{
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move||{
        let result=open_document_using(std::ffi::OsStr::new("xdg-open"),&path).map_err(|e|format!("Could not open document: {e}. Check the default viewer."));
        let _=tx.send(result);ctx.request_repaint();
    });rx
}
fn open_document_using(program:&std::ffi::OsStr,path:&std::path::Path)->std::io::Result<()>{
    if !path.is_absolute(){return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"document path must be absolute"));}
    let status=Command::new(program).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    if status.success(){Ok(())}else{Err(std::io::Error::other(format!("viewer exited unsuccessfully ({status})")))}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_opener_preserves_path_arguments_and_reports_viewer_failure(){
        use std::os::unix::{fs::PermissionsExt,ffi::OsStringExt};
        let dir=tempfile::tempdir().unwrap();let program=dir.path().join("viewer");
        std::fs::write(&program,"#!/bin/sh\nprintf '%s' \"$1\" > \"$0.args\"\n").unwrap();
        std::fs::set_permissions(&program,std::fs::Permissions::from_mode(0o700)).unwrap();
        let path=dir.path().join(std::ffi::OsString::from_vec(b"literal-$(no-exec)-\xff.txt".to_vec()));
        open_document_using(program.as_os_str(),&path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(std::fs::read(program.with_extension("args")).unwrap(),path.as_os_str().as_bytes());
        std::fs::write(&program,"#!/bin/sh\nexit 7\n").unwrap();
        assert!(open_document_using(program.as_os_str(),&path).is_err());
        assert!(open_document_using(program.as_os_str(),std::path::Path::new("relative")).is_err());
    }

    #[test]
    fn parses_icon_and_window_identity() {
        let entry = parse("[Desktop Entry]\nName=Editor\nExec=editor\nIcon=org.example.Editor\nStartupWMClass=editor-window\n").unwrap();
        assert_eq!(entry.icon.as_deref(), Some("org.example.Editor"));
        assert_eq!(entry.startup_wm_class.as_deref(), Some("editor-window"));
    }

    #[test]
    fn splits_exec_lines() {
        assert_eq!(split_exec("firefox %u"), ["firefox"]);
        assert_eq!(
            split_exec("/usr/bin/brave-browser-stable --profile-directory=Default %U"),
            ["/usr/bin/brave-browser-stable", "--profile-directory=Default"]
        );
        assert_eq!(
            split_exec(r#"env FOO=1 "/opt/My App/run" --title "a \"b\"" 100%%"#),
            ["env", "FOO=1", "/opt/My App/run", "--title", r#"a "b""#, "100%"]
        );
        assert_eq!(split_exec("app --name=%c --flag"), ["app", "--name=", "--flag"]);
        assert_eq!(split_exec("  "), Vec::<String>::new());
    }

    #[test]
    fn parses_an_application() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Text Editor\nName[de]=Texteditor\n\
             Exec=gnome-text-editor %U\nTerminal=false\n\n[Desktop Action new-window]\n\
             Name=New Window\nExec=gnome-text-editor --new-window\n",
        );
        assert_eq!(
            entry,
            Some(AppEntry {
                name: "Text Editor".into(),
                desktop_id: String::new(), icon: None, startup_wm_class: None,
                exec: vec!["gnome-text-editor".into()],
                terminal: false,
            })
        );
    }

    #[test]
    fn skips_what_should_not_be_listed() {
        assert_eq!(parse("[Desktop Entry]\nType=Application\nName=A\nExec=a\nNoDisplay=true\n"), None);
        assert_eq!(parse("[Desktop Entry]\nType=Application\nName=A\nExec=a\nHidden=true\n"), None);
        assert_eq!(parse("[Desktop Entry]\nType=Application\nName=A\nExec=a\nOnlyShowIn=GNOME;\n"), None);
        assert_eq!(parse("[Desktop Entry]\nType=Link\nName=A\nURL=http://example.org\n"), None);
        assert_eq!(parse("[Desktop Entry]\nType=Application\nName=A\n"), None);
        assert_eq!(parse("not a desktop file"), None);
    }

    #[test]
    fn launches_a_program() {
        let marker = std::env::temp_dir().join(format!("des-launch-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let entry = AppEntry {
            name: "touch".into(),
            desktop_id: String::new(), icon: None, startup_wm_class: None,
            exec: vec!["touch".into(), marker.to_string_lossy().into_owned()],
            terminal: false,
        };
        launch(&entry).unwrap();
        let appeared = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            marker.exists()
        });
        let _ = std::fs::remove_file(&marker);
        assert!(appeared);
        assert!(launch(&AppEntry { exec: vec!["/nonexistent/program".into()], ..entry }).is_err());
    }

    #[test]
    fn user_desktop_entries_override_system_and_keep_filename_identity() {
        let fixture = tempfile::tempdir().unwrap();
        let user = fixture.path().join("user");
        let system = fixture.path().join("system");
        for dir in [&user, &system] { std::fs::create_dir(dir).unwrap(); }
        std::fs::write(system.join("editor.desktop"), "[Desktop Entry]\nName=System Editor\nExec=system-editor\n").unwrap();
        std::fs::write(user.join("editor.desktop"), "[Desktop Entry]\nName=User Editor\nExec=user-editor\nIcon=user-editor\n").unwrap();
        std::fs::write(system.join("hidden.desktop"), "[Desktop Entry]\nName=Hidden\nExec=hidden\n").unwrap();
        std::fs::write(user.join("hidden.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();
        let entries = scan_dirs(vec![user, system]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].desktop_id, "editor.desktop");
        assert_eq!(entries[0].name, "User Editor");
        assert_eq!(entries[0].icon.as_deref(), Some("user-editor"));
    }

    #[test]
    fn reads_terminal_flag() {
        let entry = parse("[Desktop Entry]\nType=Application\nName=htop\nExec=htop\nTerminal=true\n").unwrap();
        assert!(entry.terminal);
    }
}
