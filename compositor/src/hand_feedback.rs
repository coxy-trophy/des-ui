//! Static pointer-side acknowledgement for hand clicks and a held drag.
use std::time::{Duration, Instant};

use des_common::hands::Button;
use smithay::{
    backend::renderer::{element::{Id, Kind, solid::SolidColorRenderElement}, utils::CommitCounter},
    utils::{Logical, Point, Rectangle},
};

const CLICK_FOR: Duration = Duration::from_millis(220);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker { Click, RightClick, Grab }

pub struct Feedback {
    left_held: bool,
    pulse: Option<(Button, Instant)>,
    accent: [f32; 4],
    ids: [Id; 17],
}

impl Default for Feedback {
    fn default() -> Self {
        let [r, g, b] = crate::splash::chosen_accent().map(|channel| channel as f32 / 255.0);
        Self { left_held: false, pulse: None, accent: [r, g, b, 1.0], ids: std::array::from_fn(|_| Id::new()) }
    }
}

impl Feedback {
    pub fn button(&mut self, button: Button, pressed: bool, now: Instant) {
        if button == Button::Left { self.left_held = pressed; }
        if pressed { self.pulse = Some((button, now)); }
    }

    pub fn reset(&mut self) {
        self.left_held = false;
        self.pulse = None;
    }

    fn marker(&self, now: Instant) -> Option<Marker> {
        if self.left_held { return Some(Marker::Grab); }
        self.pulse.filter(|(_, at)| now.saturating_duration_since(*at) < CLICK_FOR)
            .map(|(button, _)| if button == Button::Right { Marker::RightClick } else { Marker::Click })
    }

    pub fn elements(&self, position: Point<f64, Logical>, scale: f64, now: Instant) -> Vec<SolidColorRenderElement> {
        let Some(marker) = self.marker(now) else { return Vec::new(); };
        strokes(marker).into_iter().zip(&self.ids).map(|((x, y, w, h), id)| {
            let rectangle = Rectangle::<f64, Logical>::new(
                (position.x + 18.0 + x, position.y + 12.0 + y).into(), (w, h).into(),
            ).to_physical(scale).to_i32_round();
            SolidColorRenderElement::new(id.clone(), rectangle, CommitCounter::default(), self.accent, Kind::Unspecified)
        }).collect()
    }
}

fn strokes(marker: Marker) -> Vec<(f64, f64, f64, f64)> {
    let mut result = Vec::with_capacity(17);
    let mut brackets = |inset: f64, size: f64| {
        for (x, y) in [(inset, inset), (inset + size - 5.0, inset),
            (inset, inset + size - 2.0), (inset + size - 5.0, inset + size - 2.0)] {
            result.push((x, y, 5.0, 2.0));
        }
        for (x, y) in [(inset, inset), (inset + size - 2.0, inset),
            (inset, inset + size - 5.0), (inset + size - 2.0, inset + size - 5.0)] {
            result.push((x, y, 2.0, 5.0));
        }
    };
    brackets(0.0, 18.0);
    if marker == Marker::RightClick { brackets(4.0, 10.0); }
    if marker == Marker::Grab { result.push((7.0, 7.0, 4.0, 4.0)); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_is_brief_and_right_click_is_distinct() {
        let now = Instant::now();
        let mut feedback = Feedback::default();
        assert_eq!(feedback.marker(now), None);
        feedback.button(Button::Left, true, now);
        assert_eq!(feedback.marker(now), Some(Marker::Grab));
        feedback.button(Button::Left, false, now + Duration::from_millis(20));
        assert_eq!(feedback.marker(now + Duration::from_millis(219)), Some(Marker::Click));
        assert_eq!(feedback.marker(now + CLICK_FOR), None);
        feedback.button(Button::Right, true, now);
        feedback.button(Button::Right, false, now);
        assert_eq!(feedback.marker(now), Some(Marker::RightClick));
        assert_eq!(feedback.marker(now + CLICK_FOR), None);
    }

    #[test]
    fn grab_persists_until_release_and_reset_clears_everything() {
        let now = Instant::now();
        let mut feedback = Feedback::default();
        feedback.button(Button::Left, true, now);
        assert_eq!(feedback.marker(now + Duration::from_secs(10)), Some(Marker::Grab));
        feedback.button(Button::Left, false, now + Duration::from_secs(10));
        assert_eq!(feedback.marker(now + Duration::from_secs(10)), None);
        feedback.button(Button::Right, true, now);
        feedback.reset();
        assert_eq!(feedback.marker(now), None);
        feedback.button(Button::Left, true, now);
        feedback.reset();
        assert_eq!(feedback.marker(now), None);
    }

    #[test]
    fn strokes_stay_in_small_positive_bounds_and_right_click_is_double() {
        for marker in [Marker::Click, Marker::RightClick, Marker::Grab] {
            let rectangles = strokes(marker);
            assert!(!rectangles.is_empty());
            assert!(rectangles.len() <= 17);
            for (x, y, w, h) in rectangles {
                assert!(x >= 0.0 && y >= 0.0 && w > 0.0 && h > 0.0);
                assert!(x + w <= 18.0 && y + h <= 18.0);
            }
        }
        assert_eq!(strokes(Marker::Click).len(), 8);
        assert_eq!(strokes(Marker::RightClick).len(), 16);
        assert_eq!(strokes(Marker::Grab).len(), 9);
    }

    #[test]
    fn rendering_scales_output_local_coordinates_and_keeps_damage_identity() {
        use smithay::backend::renderer::element::Element;
        let now = Instant::now();
        let mut feedback = Feedback::default();
        let position = (100.0, 50.0).into();
        assert!(feedback.elements(position, 1.5, now).is_empty());
        feedback.button(Button::Left, true, now);
        let first = feedback.elements(position, 1.5, now);
        let later = feedback.elements(position, 1.5, now + Duration::from_secs(10));
        let scale = 1.5.into();
        assert_eq!(first[0].geometry(scale).loc, (177, 93).into());
        assert_eq!(first[0].geometry(scale).size, (8, 3).into());
        for (before, after) in first.iter().zip(&later) {
            assert_eq!(before.id(), after.id());
            assert_eq!(before.current_commit(), after.current_commit());
            assert_eq!(before.geometry(scale), after.geometry(scale));
        }
        feedback.reset();
        assert!(feedback.elements(position, 1.5, now).is_empty());
    }
}
