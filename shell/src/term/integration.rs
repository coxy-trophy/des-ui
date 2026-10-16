//! Shell integration: how bash, zsh and fish are started so that they read the
//! user's own files first and then the script that marks prompts and commands.
//!
//! The scripts live in the binary and are written to a private directory under the
//! runtime directory when a terminal opens. bash gets them through `--rcfile`, with
//! a file that reads what a login shell would; zsh through a ZDOTDIR whose files
//! read the user's and then hand ZDOTDIR back; fish through a vendor_conf.d on
//! XDG_DATA_DIRS. `DES_UI_SHELL_INTEGRATION=0` leaves every shell as it was.
use portable_pty::CommandBuilder;
use std::ffi::OsString;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const BASH: &str = include_str!("../../assets/shell-integration/des-ui.bash");
pub(super) const ZSH: &str = include_str!("../../assets/shell-integration/des-ui.zsh");
pub(super) const FISH: &str = include_str!("../../assets/shell-integration/des-ui.fish");

pub(super) const OPT_OUT: &str = "DES_UI_SHELL_INTEGRATION";

/// What to pass to the shell: its arguments and its environment.
pub(super) struct Integration {
    pub args: Vec<OsString>,
    pub env: Vec<(&'static str, OsString)>,
}

impl Integration {
    pub fn apply(&self, cmd: &mut CommandBuilder) {
        for arg in &self.args {
            cmd.arg(arg);
        }
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
    }
}

/// The integration for `shell`, with its files in place; `None` leaves the shell
/// to start as a plain login shell.
pub(super) fn prepare(shell: &str) -> Option<Integration> {
    if std::env::var_os(OPT_OUT).is_some_and(|v| v == "0") {
        return None;
    }
    let name = Path::new(shell).file_name()?.to_str()?;
    if !matches!(name, "bash" | "zsh" | "fish") {
        return None;
    }
    let dir = directory()?;
    install(&dir).ok()?;
    Some(for_shell(name, &dir))
}

/// The arguments and environment that make `name` read the files in `dir`.
pub(super) fn for_shell(name: &str, dir: &Path) -> Integration {
    match name {
        "bash" => Integration {
            args: vec!["--rcfile".into(), dir.join("bashrc").into()],
            env: Vec::new(),
        },
        "zsh" => {
            let mut env = vec![("ZDOTDIR", dir.join("zsh").into_os_string())];
            if let Some(user) = std::env::var_os("ZDOTDIR") {
                env.push(("DES_UI_USER_ZDOTDIR", user));
            }
            Integration { args: vec!["-l".into()], env }
        }
        _ => {
            let mut dirs = dir.join("fish-data").into_os_string();
            dirs.push(":");
            dirs.push(std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()));
            Integration { args: vec!["-l".into()], env: vec![("XDG_DATA_DIRS", dirs)] }
        }
    }
}

/// A directory of our own, created 0700, under the runtime directory; `None` when
/// there is none that is private, or whose path could not be quoted for a shell.
fn directory() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => PathBuf::from(runtime).join("des-ui"),
        None => std::env::temp_dir().join(format!("des-ui-{}", rustix::process::geteuid().as_raw())),
    };
    let dir = base.join("shell-integration");
    let plain = dir.to_str()?;
    if !plain.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '\'' | '"' | '\\' | '$' | '`')) {
        return None;
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
    private(&dir).then_some(dir)
}

/// Whether `dir` is a directory of ours that nobody else can write to.
fn private(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|meta| {
        meta.is_dir() && meta.uid() == rustix::process::geteuid().as_raw() && meta.mode() & 0o077 == 0
    })
}

/// The files each shell reads, with `dir` written into the ones that refer to it.
pub(super) fn files(dir: &Path) -> Vec<(PathBuf, String)> {
    let d = dir.display();
    vec![
        (dir.join("des-ui.bash"), BASH.to_string()),
        (dir.join("bashrc"), format!(
            "# DEs-UI starts bash with this file in place of a login shell's: it reads the\n\
             # same files bash itself would, then the prompt marks.\n\
             if [ -f /etc/profile ]; then . /etc/profile; fi\n\
             for __des_ui_file in \"$HOME/.bash_profile\" \"$HOME/.bash_login\" \"$HOME/.profile\"; do\n\
             \x20   if [ -f \"$__des_ui_file\" ]; then . \"$__des_ui_file\"; break; fi\n\
             done\n\
             unset __des_ui_file\n\
             . '{d}/des-ui.bash'\n")),
        (dir.join("des-ui.zsh"), ZSH.to_string()),
        (dir.join("zsh/.zshenv"), format!(
            "# DEs-UI starts zsh with ZDOTDIR here, so that the prompt marks can follow the\n\
             # user's own files. Each file reads the user's, from their ZDOTDIR or home.\n\
             typeset -g __des_ui_zdotdir=${{DES_UI_USER_ZDOTDIR:-$HOME}}\n\
             unset DES_UI_USER_ZDOTDIR\n\
             __des_ui_source_user() {{\n\
             \x20   [[ -f $__des_ui_zdotdir/$1 ]] || return 0\n\
             \x20   ZDOTDIR=$__des_ui_zdotdir\n\
             \x20   builtin source $__des_ui_zdotdir/$1\n\
             \x20   # The user's file may choose another ZDOTDIR; remember it, keep ours for now.\n\
             \x20   __des_ui_zdotdir=${{ZDOTDIR:-$HOME}}\n\
             \x20   ZDOTDIR='{d}/zsh'\n\
             }}\n\
             __des_ui_source_user .zshenv\n")),
        (dir.join("zsh/.zprofile"), "__des_ui_source_user .zprofile\n".to_string()),
        (dir.join("zsh/.zshrc"), format!(
            "__des_ui_source_user .zshrc\n\
             builtin source '{d}/des-ui.zsh'\n\
             # From here on zsh, and everything it starts, sees the user's own ZDOTDIR.\n\
             if [[ $__des_ui_zdotdir == $HOME ]]; then unset ZDOTDIR; else ZDOTDIR=$__des_ui_zdotdir; fi\n\
             unfunction __des_ui_source_user\n\
             unset __des_ui_zdotdir\n")),
        (dir.join("fish-data/fish/vendor_conf.d/des-ui.fish"), FISH.to_string()),
    ]
}

/// Writes the files that have changed, each in one step.
pub(super) fn install(dir: &Path) -> io::Result<()> {
    for (path, content) in files(dir) {
        if std::fs::read_to_string(&path).is_ok_and(|current| current == content) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
        }
        let temp = path.with_extension("tmp");
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        io::Write::write_all(&mut file, content.as_bytes())?;
        std::fs::rename(&temp, &path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn has(shell: &str) -> bool {
        Command::new(shell).arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn scripts_and_wrappers_parse_in_their_shells() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path()).unwrap();
        let checks = [
            ("bash", vec!["des-ui.bash", "bashrc"]),
            ("zsh", vec!["des-ui.zsh", "zsh/.zshenv", "zsh/.zprofile", "zsh/.zshrc"]),
            ("fish", vec!["fish-data/fish/vendor_conf.d/des-ui.fish"]),
        ];
        let mut checked = 0;
        for (shell, paths) in checks {
            if !has(shell) {
                eprintln!("{shell} is not installed here; its script is not checked");
                continue;
            }
            for path in paths {
                let output = Command::new(shell).arg("-n").arg(dir.path().join(path)).output().unwrap();
                assert!(output.status.success(), "{shell} -n {path}: {}", String::from_utf8_lossy(&output.stderr));
                checked += 1;
            }
        }
        assert!(checked > 0, "no shell to check the scripts with");
    }

    #[test]
    fn files_are_written_once_and_each_shell_gets_its_own_way_in() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path()).unwrap();
        let rc = dir.path().join("bashrc");
        let written = std::fs::metadata(&rc).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        install(dir.path()).unwrap();
        assert_eq!(std::fs::metadata(&rc).unwrap().modified().unwrap(), written);
        assert!(std::fs::read_to_string(&rc).unwrap().contains("/.bash_profile"));

        let bash = for_shell("bash", dir.path());
        assert_eq!(bash.args, ["--rcfile".into(), OsString::from(rc)]);
        let zsh = for_shell("zsh", dir.path());
        assert_eq!(zsh.args, ["-l"]);
        assert_eq!(zsh.env[0], ("ZDOTDIR", dir.path().join("zsh").into_os_string()));
        let fish = for_shell("fish", dir.path());
        let dirs = fish.env[0].1.to_string_lossy().into_owned();
        let ours = format!("{}/fish-data:", dir.path().display());
        assert!(dirs.starts_with(&ours) && dirs.len() > ours.len(), "{dirs}");
    }

    #[test]
    fn only_private_directories_and_known_shells_qualify() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("own");
        std::fs::DirBuilder::new().mode(0o700).create(&own).unwrap();
        assert!(private(&own));
        let open = dir.path().join("open");
        std::fs::DirBuilder::new().mode(0o755).create(&open).unwrap();
        assert!(!private(&open));
        assert!(prepare("/bin/sh").is_none());
        assert!(prepare("/usr/bin/nu").is_none());
    }
}
