//! Colours as the eye sees them: OKLCH, where a step in lightness, chroma or hue
//! looks like the same step whatever the colour. Used to derive a whole palette from
//! one accent so that every colour in it agrees.
//!
//! The maths is Björn Ottosson's OKLab (2020); sRGB in and out, with the usual
//! transfer curve. Nothing here allocates.

use eframe::egui::Color32;

/// A colour by perceived lightness (0 black to 1 white), chroma (0 grey, about 0.3 at
/// the most vivid sRGB can show) and hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

fn to_linear(channel: u8) -> f32 {
    let c = channel as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Linear sRGB to OKLab.
fn lab_of_linear(r: f32, g: f32, b: f32) -> [f32; 3] {
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab to linear sRGB, which may fall outside 0..1 for colours sRGB cannot show.
fn linear_of_lab(lab: [f32; 3]) -> [f32; 3] {
    let [big_l, a, b] = lab;
    let l = (big_l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (big_l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (big_l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

impl Oklch {
    pub fn new(l: f32, c: f32, h: f32) -> Oklch {
        Oklch { l, c, h: h.rem_euclid(360.0) }
    }

    pub fn from_srgb(color: Color32) -> Oklch {
        let [big_l, a, b] = lab_of_linear(to_linear(color.r()), to_linear(color.g()), to_linear(color.b()));
        let c = (a * a + b * b).sqrt();
        // A grey has no hue; 0 keeps derived colours predictable.
        let h = if c < 1e-4 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
        Oklch { l: big_l, c, h }
    }

    fn lab(self) -> [f32; 3] {
        let (sin, cos) = self.h.to_radians().sin_cos();
        [self.l, self.c * cos, self.c * sin]
    }

    /// Whether sRGB can show this colour as it is.
    pub fn in_gamut(self) -> bool {
        linear_of_lab(self.lab()).iter().all(|c| (-1e-4..=1.0 + 1e-4).contains(c))
    }

    /// The nearest sRGB colour: when this one is too vivid to show, its chroma is
    /// lowered, keeping its lightness and hue, until it fits.
    pub fn to_srgb(self) -> Color32 {
        let mut color = Oklch { l: self.l.clamp(0.0, 1.0), ..self };
        if !color.in_gamut() {
            let (mut low, mut high) = (0.0, color.c);
            for _ in 0..24 {
                let mid = (low + high) / 2.0;
                if (Oklch { c: mid, ..color }).in_gamut() { low = mid } else { high = mid }
            }
            color.c = low;
        }
        let [r, g, b] = linear_of_lab(color.lab());
        let byte = |c: f32| (from_linear(c.clamp(0.0, 1.0)) * 255.0).round() as u8;
        Color32::from_rgb(byte(r), byte(g), byte(b))
    }
}

/// The signed shortest turn from hue `from` to hue `to`, in -180..=180 degrees.
pub fn hue_turn(from: f32, to: f32) -> f32 {
    let turn = (to - from).rem_euclid(360.0);
    if turn > 180.0 { turn - 360.0 } else { turn }
}

/// Relative luminance, as WCAG defines it.
fn luminance(color: Color32) -> f32 {
    0.2126 * to_linear(color.r()) + 0.7152 * to_linear(color.g()) + 0.0722 * to_linear(color.b())
}

/// WCAG contrast ratio between two colours, 1 (the same) to 21 (black on white).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// `color` made light enough to reach `ratio` against `ground`, by raising its
/// lightness alone. A colour already there is returned as it is.
pub fn legible(color: Oklch, ground: Color32, ratio: f32) -> Oklch {
    let mut color = color;
    for _ in 0..40 {
        if contrast(color.to_srgb(), ground) >= ratio || color.l >= 1.0 {
            break;
        }
        color.l = (color.l + 0.02).min(1.0);
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_white_black_and_red() {
        let white = Oklch::from_srgb(Color32::WHITE);
        assert!((white.l - 1.0).abs() < 0.001 && white.c < 0.001, "{white:?}");
        let black = Oklch::from_srgb(Color32::BLACK);
        assert!(black.l.abs() < 0.001 && black.c < 0.001, "{black:?}");
        let red = Oklch::from_srgb(Color32::from_rgb(255, 0, 0));
        assert!((red.l - 0.628).abs() < 0.005, "{red:?}");
        assert!((red.c - 0.258).abs() < 0.005, "{red:?}");
        assert!((red.h - 29.2).abs() < 0.5, "{red:?}");
    }

    #[test]
    fn round_trips_every_corner_and_a_grid() {
        let mut worst = 0;
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    let color = Color32::from_rgb(r as u8, g as u8, b as u8);
                    let back = Oklch::from_srgb(color).to_srgb();
                    for (a, b) in [(color.r(), back.r()), (color.g(), back.g()), (color.b(), back.b())] {
                        worst = worst.max((a as i32 - b as i32).unsigned_abs());
                    }
                }
            }
        }
        assert!(worst <= 1, "a channel drifted by {worst}");
    }

    #[test]
    fn too_vivid_colours_lose_chroma_not_hue() {
        let wild = Oklch::new(0.7, 0.4, 142.0);
        assert!(!wild.in_gamut());
        let shown = Oklch::from_srgb(wild.to_srgb());
        assert!(shown.c < 0.4 && shown.c > 0.1, "{shown:?}");
        assert!((shown.l - 0.7).abs() < 0.02, "{shown:?}");
        assert!(hue_turn(shown.h, 142.0).abs() < 3.0, "{shown:?}");
    }

    #[test]
    fn turns_hues_the_short_way() {
        assert_eq!(hue_turn(350.0, 10.0), 20.0);
        assert_eq!(hue_turn(10.0, 350.0), -20.0);
        assert_eq!(Oklch::new(0.5, 0.1, 370.0).h, 10.0);
    }

    #[test]
    fn measures_contrast_and_lifts_dim_colours() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.001);
        let ground = crate::ui::GROUND;
        let dim = Oklch::new(0.3, 0.12, 29.0);
        assert!(contrast(dim.to_srgb(), ground) < 4.5);
        let lifted = legible(dim, ground, 4.5);
        assert!(contrast(lifted.to_srgb(), ground) >= 4.5);
        assert!(lifted.l > dim.l && lifted.h == dim.h);
        let fine = Oklch::new(0.8, 0.12, 29.0);
        assert_eq!(legible(fine, ground, 4.5), fine);
    }
}
