//! Shared desktop appearance settings and validation.

use std::path::PathBuf;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceStyle { Solid, Frosted, Liquid }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperFit { Fill, Fit }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality { Auto, Low, High }

#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceSettings {
    pub wallpaper: Option<PathBuf>,
    pub style: SurfaceStyle,
    pub fit: WallpaperFit,
    pub blur: f32,
    pub dim: f32,
    pub opacity: f32,
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    pub quality: Quality,
}
impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { wallpaper: None, style: SurfaceStyle::Solid, fit: WallpaperFit::Fill,
            blur: 18.0, dim: 35.0, opacity: 62.0, reduce_transparency: false,
            reduce_motion: false, quality: Quality::Auto }
    }
}
impl AppearanceSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, min, max) in [(self.blur, 0.0, 40.0), (self.dim, 0.0, 80.0), (self.opacity, 35.0, 100.0)] {
            if !value.is_finite() || !(min..=max).contains(&value) { return Err("Appearance value is outside its supported range"); }
        }
        if self.wallpaper.as_ref().is_some_and(|p| !p.is_absolute() || p.as_os_str().as_bytes().len() > 4096 || p.as_os_str().as_bytes().contains(&0)) {
            return Err("Wallpaper must be an absolute local path of at most 4096 bytes");
        }
        Ok(())
    }
    pub fn effective_style(&self, live_glass: bool) -> SurfaceStyle {
        if self.reduce_transparency { SurfaceStyle::Solid }
        else if self.style == SurfaceStyle::Liquid && !live_glass { SurfaceStyle::Frosted }
        else { self.style }
    }
    /// A single atomic config value. Hex preserves arbitrary Unix paths and cannot
    /// introduce comments, newlines or additional configuration keys.
    pub fn encode(&self) -> String {
        let path = self.wallpaper.as_ref().map(|p| p.as_os_str().as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()).unwrap_or_else(|| "-".into());
        format!("2 {} {} {} {} {} {} {} {} {}",
            match self.style { SurfaceStyle::Solid => "solid", SurfaceStyle::Frosted => "frosted", SurfaceStyle::Liquid => "liquid" },
            match self.fit { WallpaperFit::Fill => "fill", WallpaperFit::Fit => "fit" },
            self.blur, self.dim, self.opacity, u8::from(self.reduce_transparency), u8::from(self.reduce_motion),
            match self.quality { Quality::Auto => "auto", Quality::Low => "low", Quality::High => "high" }, path)
    }
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > 8300 { return None; }
        let fields: Vec<_> = value.split_whitespace().collect();
        if fields.len() != 10 || !matches!(fields[0], "1" | "2") { return None; }
        let path = fields[9];
        let wallpaper = if path == "-" { None } else {
            if path.len() % 2 != 0 || !path.is_ascii() { return None; }
            let bytes = (0..path.len()).step_by(2).map(|i| u8::from_str_radix(&path[i..i+2], 16)).collect::<Result<Vec<_>, _>>().ok()?;
            Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        };
        let mut settings = Self {
            wallpaper,
            style: match fields[1] { "solid" => SurfaceStyle::Solid, "frosted" => SurfaceStyle::Frosted, "liquid" => SurfaceStyle::Liquid, _ => return None },
            fit: match fields[2] { "fill" => WallpaperFit::Fill, "fit" => WallpaperFit::Fit, _ => return None },
            blur: fields[3].parse().ok()?, dim: fields[4].parse().ok()?, opacity: fields[5].parse().ok()?,
            reduce_transparency: match fields[6] { "0" => false, "1" => true, _ => return None },
            reduce_motion: match fields[7] { "0" => false, "1" => true, _ => return None },
            quality: match fields[8] { "auto" => Quality::Auto, "low" => Quality::Low, "high" => Quality::High, _ => return None },
        };
        // Version 1 selected wallpaper without enabling its surface treatment.
        // Migrate that legacy default once; v2 preserves an explicit Opaque choice.
        if fields[0] == "1" && settings.wallpaper.is_some() && settings.style == SurfaceStyle::Solid {
            settings.style = SurfaceStyle::Frosted;
            settings.opacity = Self::default().opacity;
            settings.dim = settings.dim.min(35.0);
            settings.blur = settings.blur.max(18.0);
        }
        settings.validate().ok()?;
        Some(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_wallpaper_solid_defaults_migrate_but_explicit_new_opaque_choice_survives() {
        let legacy = AppearanceSettings::parse("1 solid fill 7 50 91 0 0 auto 2f746d702f612e706e67").unwrap();
        assert_eq!(legacy.style, SurfaceStyle::Frosted);
        assert!(legacy.opacity <= 65.0);
        let explicit = AppearanceSettings { style: SurfaceStyle::Solid, wallpaper: Some("/tmp/a.png".into()), ..Default::default() };
        assert_eq!(AppearanceSettings::parse(&explicit.encode()).unwrap(), explicit);
        let accessible = AppearanceSettings::parse("1 solid fill 7 50 91 1 0 auto 2f746d702f612e706e67").unwrap();
        assert_eq!(accessible.effective_style(false), SurfaceStyle::Solid);
    }
    #[test]
    fn appearance_defaults_and_round_trip_preserve_unix_paths() {
        let mut settings = AppearanceSettings::default();
        assert_eq!(settings.style, SurfaceStyle::Solid);
        assert!(settings.wallpaper.is_none());
        settings.wallpaper = Some("/home/user/Pictures/a # = 雨.png".into());
        settings.style = SurfaceStyle::Frosted;
        assert_eq!(AppearanceSettings::parse(&settings.encode()).unwrap(), settings);
        assert!(!settings.encode().contains('#'));
    }
    #[test]
    fn appearance_rejects_invalid_values_and_unknown_versions() {
        let mut settings = AppearanceSettings::default();
        settings.blur = f32::NAN;
        assert!(settings.validate().is_err());
        settings.blur = 41.0;
        assert!(settings.validate().is_err());
        assert!(AppearanceSettings::parse("3 solid fill 18 35 88 0 0 auto -").is_none());
        assert!(AppearanceSettings::parse("1 mystery fill 18 35 88 0 0 auto -").is_none());
        assert!(AppearanceSettings::parse("1 solid fill 18 35 88 0 0 auto ff0").is_none());
    }
    #[test]
    fn appearance_effective_style_respects_accessibility_and_capability() {
        let mut settings = AppearanceSettings::default();
        settings.style = SurfaceStyle::Liquid;
        assert_eq!(settings.effective_style(false), SurfaceStyle::Frosted);
        settings.reduce_transparency = true;
        assert_eq!(settings.effective_style(true), SurfaceStyle::Solid);
    }
}
