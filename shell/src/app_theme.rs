//! Light or dark for applications. The desktop's own look is not touched.
//!
//! Applications learn the choice in three ways, so each is set:
//! - the desktop portal's `org.freedesktop.appearance color-scheme`, which the GTK
//!   portal backend reads from GNOME's `color-scheme` setting and announces to
//!   running applications at once (GTK 4 and libadwaita, Firefox, Chromium and
//!   Electron apps, recent Qt);
//! - GNOME's `gtk-theme` setting, `Adwaita-dark` or `Adwaita`, for GTK 3 apps;
//! - `gtk-application-prefer-dark-theme` in `~/.config/gtk-3.0/settings.ini`, for
//!   GTK 3 apps that do not read GNOME's settings.

use crate::config::AppTheme;
use std::path::PathBuf;

fn color_scheme(theme: AppTheme) -> &'static str {
    match theme {
        AppTheme::Light => "prefer-light",
        AppTheme::Dark => "prefer-dark",
    }
}

fn gtk_theme(theme: AppTheme) -> &'static str {
    match theme {
        AppTheme::Light => "Adwaita",
        AppTheme::Dark => "Adwaita-dark",
    }
}

/// Only the stock themes are swapped; a theme the user chose themselves stays.
fn swaps_gtk_theme(current: &str) -> bool {
    matches!(current.trim().trim_matches('\''), "" | "Adwaita" | "Adwaita-dark" | "Default")
}

/// `settings.ini` with the dark preference set, keeping everything else in it.
fn settings_ini(existing: &str, theme: AppTheme) -> String {
    const KEY: &str = "gtk-application-prefer-dark-theme";
    let line = format!("{KEY}={}", u8::from(theme == AppTheme::Dark));
    let mut out = Vec::new();
    let (mut in_settings, mut seen_section, mut written) = (false, false, false);
    for raw in existing.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') {
            if in_settings && !written {
                out.push(line.clone());
                written = true;
            }
            in_settings = trimmed.eq_ignore_ascii_case("[settings]");
            seen_section |= in_settings;
        } else if in_settings && trimmed.split('=').next().is_some_and(|key| key.trim() == KEY) {
            if !written {
                out.push(line.clone());
                written = true;
            }
            continue;
        }
        out.push(raw.to_string());
    }
    if !written {
        if !seen_section {
            if out.last().is_some_and(|last| !last.trim().is_empty()) { out.push(String::new()); }
            out.push("[Settings]".to_string());
        }
        out.push(line);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

fn gtk3_settings() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("gtk-3.0").join("settings.ini"))
}

fn gsettings(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("gsettings").args(args).stderr(std::process::Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Tells applications, in the background, to draw themselves light or dark.
pub fn apply(theme: AppTheme) {
    if cfg!(test) { return; }
    std::thread::spawn(move || {
        const SCHEMA: &str = "org.gnome.desktop.interface";
        gsettings(&["set", SCHEMA, "color-scheme", color_scheme(theme)]);
        if gsettings(&["get", SCHEMA, "gtk-theme"]).is_some_and(|current| swaps_gtk_theme(&current)) {
            gsettings(&["set", SCHEMA, "gtk-theme", gtk_theme(theme)]);
        }
        if let Some(path) = gtk3_settings() {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let wanted = settings_ini(&existing, theme);
            if wanted != existing {
                if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
                let temporary = path.with_extension("ini.des-ui");
                if std::fs::write(&temporary, wanted).is_ok() {
                    let _ = std::fs::rename(&temporary, &path);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_gets_the_section_and_the_key() {
        assert_eq!(settings_ini("", AppTheme::Dark), "[Settings]\ngtk-application-prefer-dark-theme=1\n");
        assert_eq!(settings_ini("", AppTheme::Light), "[Settings]\ngtk-application-prefer-dark-theme=0\n");
    }

    #[test]
    fn other_settings_are_kept_and_the_key_is_replaced_in_place() {
        let existing = "[Settings]\ngtk-font-name=Inter 11\ngtk-application-prefer-dark-theme = 0\ngtk-icon-theme-name=Papirus\n";
        assert_eq!(
            settings_ini(existing, AppTheme::Dark),
            "[Settings]\ngtk-font-name=Inter 11\ngtk-application-prefer-dark-theme=1\ngtk-icon-theme-name=Papirus\n"
        );
    }

    #[test]
    fn the_key_goes_into_the_settings_section_not_another_one() {
        let existing = "[Settings]\ngtk-font-name=Inter 11\n\n[Other]\nthing=1\n";
        assert_eq!(
            settings_ini(existing, AppTheme::Dark),
            "[Settings]\ngtk-font-name=Inter 11\n\ngtk-application-prefer-dark-theme=1\n[Other]\nthing=1\n"
        );
        assert_eq!(settings_ini("[Other]\nthing=1\n", AppTheme::Light), "[Other]\nthing=1\n\n[Settings]\ngtk-application-prefer-dark-theme=0\n");
    }

    #[test]
    fn applying_twice_changes_nothing() {
        let once = settings_ini("[Settings]\ngtk-font-name=Inter 11\n", AppTheme::Dark);
        assert_eq!(settings_ini(&once, AppTheme::Dark), once);
    }

    #[test]
    fn only_stock_gtk_themes_are_swapped() {
        assert!(swaps_gtk_theme("'Adwaita'"));
        assert!(swaps_gtk_theme("'Adwaita-dark'"));
        assert!(!swaps_gtk_theme("'Yaru-purple'"));
        assert_eq!(gtk_theme(AppTheme::Dark), "Adwaita-dark");
        assert_eq!(color_scheme(AppTheme::Light), "prefer-light");
    }
}
