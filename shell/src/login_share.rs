//! What a user's session leaves for the login screen: their wallpaper and accent.
//!
//! The login screen runs as its own account and cannot read anyone's home, so the
//! session copies a screen-sized JPEG of the wallpaper and a line of settings to a
//! shared folder, the way the user's picture is shared. The folder is sticky and
//! open to all, so each user can replace only their own files; the login screen uses
//! a file only when it is a plain file owned by the account it is named after.
//! What is shared is what the login screen shows anyway.
use eframe::egui::Color32;
use des_common::appearance::WallpaperFit;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The shared folder, made by the installer.
pub const LOGIN_DIR: &str = "/var/lib/des-ui/wallpapers";
/// No more than this is read back from the shared folder.
const LIMIT: u64 = 12 * 1024 * 1024;

/// What the login screen should show for one user.
#[derive(Debug, Clone, PartialEq)]
pub struct Shared {
    pub wallpaper: Option<PathBuf>,
    pub accent: Option<Color32>,
    pub fit: WallpaperFit,
}

/// Keeps the shared copy in step with the session's appearance.
#[derive(Default)]
pub struct Sharer {
    last: Option<(Option<PathBuf>, [u8; 3], bool)>,
}

impl Sharer {
    /// Shares the wallpaper and accent whenever either changes, away from the UI
    /// thread. Nothing is written by tests or where the folder is missing.
    pub fn sync(&mut self, wallpaper: Option<&Path>, accent: Color32, fit: WallpaperFit) {
        let now = (wallpaper.map(Path::to_path_buf), [accent.r(), accent.g(), accent.b()], fit == WallpaperFit::Fill);
        if self.last.as_ref() == Some(&now) { return; }
        self.last = Some(now.clone());
        if cfg!(test) { return; }
        let Ok(user) = std::env::var("USER") else { return };
        if !Path::new(LOGIN_DIR).is_dir() || !valid_name(&user) { return; }
        let _ = std::thread::Builder::new().name("des-ui-login-share".into()).spawn(move || {
            if let Err(why) = write(Path::new(LOGIN_DIR), &user, now.0.as_deref(), now.1, now.2) {
                eprintln!("des-ui: cannot share the wallpaper with the login screen: {why}");
            }
        });
    }
}

/// A user name that is safe as a file name.
fn valid_name(user: &str) -> bool {
    !user.is_empty() && user.len() <= 64 && !user.starts_with('.') && user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'))
}

/// Writes `<user>.jpg` and `<user>.conf` into `dir`, each through a new file renamed
/// into place; without a wallpaper the picture is removed and the accent still shared.
fn write(dir: &Path, user: &str, wallpaper: Option<&Path>, accent: [u8; 3], fill: bool) -> Result<(), String> {
    let picture = dir.join(format!("{user}.jpg"));
    match wallpaper {
        Some(path) => {
            let image = crate::wallpaper::decode(&crate::wallpaper::read_bounded(path)?)?;
            // Screen sized: larger pictures shrink, smaller ones stay as they are.
            let image = if image.width() > 2560 || image.height() > 2560 { image.thumbnail(2560, 2560) } else { image }.to_rgb8();
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90).encode_image(&image).map_err(|e| e.to_string())?;
            replace(dir, &picture, &bytes)?;
        }
        None => match std::fs::remove_file(&picture) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        },
    }
    let settings = format!("accent=#{:02x}{:02x}{:02x}\nfit={}\n", accent[0], accent[1], accent[2], if fill { "fill" } else { "fit" });
    replace(dir, &dir.join(format!("{user}.conf")), settings.as_bytes())
}

/// Writes a new file readable by all next to `target` and renames it over `target`.
fn replace(dir: &Path, target: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let temporary = dir.join(format!(".{}.{}.{stamp}", target.file_name().and_then(|n| n.to_str()).unwrap_or("share"), std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(&temporary).map_err(|e| format!("{}: {e}", temporary.display()))?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all()).and_then(|_| std::fs::rename(&temporary, target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{}: {e}", target.display()));
    }
    Ok(())
}

/// The uid of `user` in a passwd file.
fn uid_of(passwd: &str, user: &str) -> Option<u32> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == user).then_some(())?;
        fields.nth(1)?.parse().ok()
    })
}

/// A plain file in `dir`, owned by `uid` and not too large.
fn owned(dir: &Path, name: &str, uid: u32) -> Option<PathBuf> {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    (meta.file_type().is_file() && meta.uid() == uid && meta.len() <= LIMIT).then_some(path)
}

/// What `user` left for the login screen in `dir`, trusting only their own files.
pub fn for_user_in(dir: &Path, passwd: &str, user: &str) -> Option<Shared> {
    if !valid_name(user) { return None; }
    let uid = uid_of(passwd, user)?;
    let wallpaper = owned(dir, &format!("{user}.jpg"), uid);
    let settings = owned(dir, &format!("{user}.conf"), uid).and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    let mut accent = None;
    let mut fit = WallpaperFit::Fill;
    for line in settings.lines().take(16) {
        match line.split_once('=') {
            Some(("accent", value)) => accent = Color32::from_hex(value.trim()).ok().map(|c| Color32::from_rgb(c.r(), c.g(), c.b())),
            Some(("fit", "fit")) => fit = WallpaperFit::Fit,
            _ => {}
        }
    }
    (wallpaper.is_some() || accent.is_some()).then_some(Shared { wallpaper, accent, fit })
}

/// What `user` left for the login screen.
pub fn for_user(user: &str) -> Option<Shared> {
    // Debug builds can be pointed elsewhere, to preview the login screen.
    let dir = if cfg!(debug_assertions) { std::env::var_os("DESUI_LOGIN_DIR").map(PathBuf::from) } else { None };
    for_user_in(&dir.unwrap_or_else(|| PathBuf::from(LOGIN_DIR)), &std::fs::read_to_string("/etc/passwd").ok()?, user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> u32 { rustix::process::getuid().as_raw() }

    #[test]
    fn shares_and_reads_back_a_users_wallpaper_and_accent() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        image::RgbImage::from_pixel(64, 36, image::Rgb([200, 40, 120])).save(&source).unwrap();
        let share = dir.path().join("share");
        std::fs::create_dir(&share).unwrap();
        write(&share, "ada", Some(&source), [241, 0, 166], true).unwrap();
        let passwd = format!("root:x:0:0::/root:/bin/sh\nada:x:{}:100::/home/ada:/bin/sh\n", me());
        let shared = for_user_in(&share, &passwd, "ada").unwrap();
        assert_eq!(shared.accent, Some(Color32::from_rgb(241, 0, 166)));
        assert_eq!(shared.fit, WallpaperFit::Fill);
        let picture = image::open(shared.wallpaper.unwrap()).unwrap();
        assert_eq!((picture.width(), picture.height()), (64, 36));
        // No temporary files are left behind.
        assert_eq!(std::fs::read_dir(&share).unwrap().count(), 2);

        // Without a wallpaper, the picture goes and the accent stays.
        write(&share, "ada", None, [79, 216, 206], false).unwrap();
        let shared = for_user_in(&share, &passwd, "ada").unwrap();
        assert_eq!((shared.wallpaper, shared.accent, shared.fit), (None, Some(Color32::from_rgb(79, 216, 206)), WallpaperFit::Fit));
    }

    #[test]
    fn files_owned_by_someone_else_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ada.conf"), "accent=#ff0000\n").unwrap();
        // The files belong to the account running the test, not to ada's uid.
        let passwd = format!("ada:x:{}:100::/home/ada:/bin/sh\n", me() + 1);
        assert_eq!(for_user_in(dir.path(), &passwd, "ada"), None);
        assert_eq!(for_user_in(dir.path(), &passwd, "nobody-here"), None);
    }

    #[test]
    fn links_and_odd_names_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("elsewhere"), "accent=#ff0000\n").unwrap();
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("ada.conf")).unwrap();
        let passwd = format!("ada:x:{}:100::/home/ada:/bin/sh\n", me());
        assert_eq!(for_user_in(dir.path(), &passwd, "ada"), None, "a link is not followed");
        for name in ["", "../ada", ".hidden", "a/b"] { assert!(!valid_name(name), "{name}"); }
        assert!(valid_name("coxy"));
    }
}
