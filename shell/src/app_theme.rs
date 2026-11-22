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
//!
//! Qt apps get it through the session's `QT_QPA_PLATFORMTHEME=gtk3`: Qt 6 takes its
//! colours from the GTK theme and follows a change at once. Qt 5's GTK plugin has no
//! colours, so Qt 5 apps are given the Adwaita style in the matching mode when the
//! desktop starts them ([`qt_style`]); they change the next time they open.
//! Browsers and Electron apps read the portal only while their own appearance
//! setting is left on "system" or "device".

use crate::config::AppTheme;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// The choice last applied: 0 none yet, 1 light, 2 dark.
static CURRENT: AtomicU8 = AtomicU8::new(0);

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

/// The Qt style for a newly started app in this mode. Adwaita (from `adwaita-qt`)
/// is a Qt 5 style; Qt 6 does not find it and keeps its default, which takes its
/// colours from the GTK theme instead.
fn qt_style_for(theme: AppTheme) -> &'static str {
    match theme {
        AppTheme::Light => "adwaita",
        AppTheme::Dark => "adwaita-dark",
    }
}

/// The style a Qt app started now should be given, if any: none before a choice was
/// applied, and none when the person set `QT_STYLE_OVERRIDE` for themselves.
pub fn qt_style() -> Option<&'static str> {
    if std::env::var_os("QT_STYLE_OVERRIDE").is_some() { return None; }
    match CURRENT.load(Ordering::Relaxed) {
        1 => Some(qt_style_for(AppTheme::Light)),
        2 => Some(qt_style_for(AppTheme::Dark)),
        _ => None,
    }
}

/// Tells applications, in the background, to draw themselves light or dark.
pub fn apply(theme: AppTheme) {
    CURRENT.store(if theme == AppTheme::Dark { 2 } else { 1 }, Ordering::Relaxed);
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
    fn qt_apps_get_the_adwaita_style_in_the_chosen_mode() {
        assert_eq!(qt_style_for(AppTheme::Dark), "adwaita-dark");
        assert_eq!(qt_style_for(AppTheme::Light), "adwaita");
    }
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
