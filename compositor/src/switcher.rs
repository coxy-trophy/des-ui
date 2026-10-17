//! Alt+Tab: the windows of a display in the order they were last used.
//!
//! The compositor remembers which windows had the keyboard, most recent first. Holding
//! Alt and pressing Tab opens a strip of pictures of them on the display in use, with
//! the one used before this one chosen; Tab and Shift+Tab move along it, and letting go
//! of Alt brings the chosen one forward. Alt+` does the same among the windows of the
//! application in use.

use smithay::utils::{Logical, Rectangle, Size};

/// Windows, most recently used first.
pub struct Recency<K> {
    order: Vec<K>,
}

impl<K: PartialEq + Clone> Default for Recency<K> {
    fn default() -> Self {
        Recency { order: Vec::new() }
    }
}

impl<K: PartialEq + Clone> Recency<K> {
    /// This one was just given the keyboard.
    pub fn touch(&mut self, key: &K) {
        self.order.retain(|other| other != key);
        self.order.insert(0, key.clone());
    }

    pub fn forget(&mut self, key: &K) {
        self.order.retain(|other| other != key);
    }

    /// `candidates` in the order they were last used: those used before, most recent
    /// first, then those never used, in the order given.
    pub fn sort(&self, candidates: &[K]) -> Vec<K> {
        let mut sorted: Vec<K> = self.order.iter().filter(|key| candidates.contains(key)).cloned().collect();
        sorted.extend(candidates.iter().filter(|key| !self.order.contains(key)).cloned());
        sorted
    }
}

/// The next place along a ring of `len`, forwards or back.
pub fn step(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let current = current.min(len - 1);
    if forward { (current + 1) % len } else { (current + len - 1) % len }
}

/// Which picture is chosen when the switcher opens: the window used before this one,
/// or this one when it is alone.
pub fn first_choice(len: usize, forward: bool) -> usize {
    match len {
        0 | 1 => 0,
        _ if forward => 1,
        _ => len - 1,
    }
}

/// How far the strip keeps from the edges of the display, and around its pictures.
const EDGE: i32 = 48;
const PAD: i32 = 20;
const GAP: i32 = 16;
/// Room under each picture for its name, which the shell writes there.
pub const LABEL: i32 = 28;

/// Where the switcher's strip is on a display of `size`, and each window's picture in
/// it, in the display's own coordinates. `aspects` are width over height. Pictures are
/// as tall as a fifth of the display at most, as wide as their shape asks within
/// reason, and all shrink together until the strip fits across the display.
pub fn strip(size: Size<i32, Logical>, aspects: &[f64]) -> (Rectangle<i32, Logical>, Vec<Rectangle<i32, Logical>>) {
    let height = ((size.h / 5).clamp(60, 180)) as f64;
    let widths: Vec<f64> = aspects
        .iter()
        .map(|aspect| {
            let aspect = if aspect.is_finite() && *aspect > 0.0 { *aspect } else { 16.0 / 9.0 };
            (height * aspect).clamp(height * 0.6, height * 2.0)
        })
        .collect();
    let gaps = (GAP * (widths.len().saturating_sub(1)) as i32) as f64;
    let room = (size.w - 2 * EDGE - 2 * PAD) as f64 - gaps;
    let natural: f64 = widths.iter().sum();
    let shrink = if natural > room && natural > 0.0 { (room / natural).max(0.05) } else { 1.0 };
    let height = (height * shrink).max(1.0);
    let widths: Vec<i32> = widths.iter().map(|width| ((width * shrink).round() as i32).max(1)).collect();
    let inner: i32 = widths.iter().sum::<i32>() + GAP * widths.len().saturating_sub(1) as i32;
    let strip_w = (inner + 2 * PAD).min(size.w).max(1);
    let strip_h = (height.round() as i32 + LABEL + 2 * PAD).min(size.h).max(1);
    let strip = Rectangle::new(((size.w - strip_w) / 2, (size.h - strip_h) / 2).into(), (strip_w, strip_h).into());
    let mut x = strip.loc.x + PAD;
    let thumbs = widths
        .iter()
        .map(|width| {
            let thumb = Rectangle::new((x, strip.loc.y + PAD).into(), (*width, height.round() as i32).into());
            x += width + GAP;
            thumb
        })
        .collect();
    (strip, thumbs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_used_comes_first_and_new_ones_after() {
        let mut recency = Recency::default();
        for key in ["a", "b", "c"] {
            recency.touch(&key);
        }
        recency.touch(&"a");
        assert_eq!(recency.sort(&["a", "b", "c", "d"]), vec!["a", "c", "b", "d"]);
        recency.forget(&"c");
        assert_eq!(recency.sort(&["c", "b", "a"]), vec!["a", "b", "c"]);
        // Only the candidates, whatever else was used.
        assert_eq!(recency.sort(&["b"]), vec!["b"]);
    }

    #[test]
    fn the_choice_goes_round_both_ways() {
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(1, 3, true), 2);
        assert_eq!(step(9, 3, true), 0, "a stale place is taken as the last");
        assert_eq!(step(0, 0, true), 0);
        assert_eq!(first_choice(1, true), 0);
        assert_eq!(first_choice(4, true), 1);
        assert_eq!(first_choice(4, false), 3);
    }

    #[test]
    fn the_strip_is_centred_and_fits_however_many_windows() {
        let size: Size<i32, Logical> = (1920, 1080).into();
        for n in 1..=24 {
            let aspects = vec![16.0 / 9.0; n];
            let (strip, thumbs) = strip(size, &aspects);
            assert_eq!(thumbs.len(), n);
            assert!(strip.loc.x >= 0 && strip.loc.x + strip.size.w <= 1920, "{n}: {strip:?}");
            assert!((strip.loc.x - (1920 - strip.loc.x - strip.size.w)).abs() <= 1, "centred");
            for pair in thumbs.windows(2) {
                assert!(pair[0].loc.x + pair[0].size.w <= pair[1].loc.x, "no overlap");
            }
            for thumb in &thumbs {
                assert!(strip.contains_rect(*thumb), "{n}: {thumb:?} in {strip:?}");
            }
        }
        // A narrow window is not drawn as a sliver, nor a wide one as a ribbon.
        let (_, thumbs) = strip(size, &[0.1, 10.0, f64::NAN]);
        let h = thumbs[0].size.h as f64;
        assert!(thumbs[0].size.w as f64 >= h * 0.6 - 1.0);
        assert!(thumbs[1].size.w as f64 <= h * 2.0 + 1.0);
        assert!(thumbs[2].size.w > 0);
    }
}
