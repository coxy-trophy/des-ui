//! Fractional scaling: each window is told the exact scale of the display it is on,
//! 1.25 or 1.5 as well as whole numbers, so that it draws its buffer at that size
//! and, with a viewport, is shown sharp instead of a whole-number buffer scaled down.
//! A spanning window follows its largest independent-output overlap. Mirrors never
//! take ownership of client scale, and equal overlaps prefer the explicit primary.

use smithay::{
    delegate_fractional_scale,
    desktop::Window,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        compositor::{send_surface_state, with_states},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
    },
};

use crate::DesComp;

impl FractionalScaleHandler for DesComp {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // Said at once, for a window that asks after it was placed.
        let output = self
            .window_for(&surface)
            .and_then(|window| self.preferred_output_for_window(&window))
            .or_else(|| self.primary_output());
        if let Some(output) = output {
            with_states(&surface, |states| {
                with_fractional_scale(states, |fractional| fractional.set_preferred_scale(output.current_scale().fractional_scale()));
            });
        }
    }
}

delegate_fractional_scale!(DesComp);

/// The whole-number scale a client without fractional scaling should draw at: the
/// display's scale rounded up, so that it is scaled down, never blurred up.
pub fn integer_scale(fractional: f64) -> i32 {
    (fractional.ceil() as i32).max(1)
}

/// Tells every surface of `window` the scale and turn of `output`.
fn prefer(window: &Window, output: &Output) {
    let scale = output.current_scale().fractional_scale();
    let transform = output.current_transform();
    window.with_surfaces(|surface, states| {
        with_fractional_scale(states, |fractional| fractional.set_preferred_scale(scale));
        send_surface_state(surface, states, integer_scale(scale), transform);
    });
}

#[derive(Clone)]
struct ScaleCandidate {
    name: String,
    geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    primary: bool,
    mirror: bool,
}

fn preferred_output_index(window: smithay::utils::Rectangle<i32, smithay::utils::Logical>, candidates: &[ScaleCandidate]) -> Option<usize> {
    candidates.iter().enumerate().filter_map(|(index, candidate)| {
        if candidate.mirror { return None; }
        let overlap = window.intersection(candidate.geometry)?;
        let area = i64::from(overlap.size.w) * i64::from(overlap.size.h);
        (area > 0).then_some((index, area, candidate))
    }).max_by(|(_, area_a, a), (_, area_b, b)| {
        area_a.cmp(area_b).then(a.primary.cmp(&b.primary)).then_with(|| b.name.cmp(&a.name))
    }).map(|(index,_,_)|index)
}

impl DesComp {
    fn scale_candidates(&self) -> (Vec<Output>, Vec<ScaleCandidate>) {
        let primary = self.primary_output();
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        let candidates: Vec<_> = outputs.iter().map(|output| ScaleCandidate {
            name: output.name(), geometry: self.space.output_geometry(output).unwrap_or_default(),
            primary: primary.as_ref() == Some(output), mirror: self.output_is_mirror(output),
        }).collect();
        (outputs, candidates)
    }

    fn preferred_output_for_window(&self, window: &Window) -> Option<Output> {
        let geometry = self.space.element_geometry(window)?;
        let (outputs, candidates) = self.scale_candidates();
        preferred_output_index(geometry, &candidates).map(|index| outputs[index].clone())
    }

    /// Called once a frame, before drawing: windows that moved to another display,
    /// or whose display changed scale, hear of it. Smithay sends nothing it already sent.
    pub fn prefer_scales(&self) {
        let (outputs, candidates) = self.scale_candidates();
        let windows = self.shell.iter().chain(self.backdrops.iter()).chain(self.apps.iter().map(|app| &app.window));
        for window in windows {
            if let Some(index) = self.space.element_geometry(window).and_then(|geometry| preferred_output_index(geometry, &candidates)) {
                prefer(window, &outputs[index]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, x: i32, width: i32, primary: bool, mirror: bool) -> ScaleCandidate {
        ScaleCandidate { name: name.into(), geometry: smithay::utils::Rectangle::new((x, 0).into(), (width, 1080).into()), primary, mirror }
    }

    #[test]
    fn mirrored_output_never_changes_the_clients_preferred_scale() {
        let window = smithay::utils::Rectangle::new((80, 30).into(), (1600, 900).into());
        let primary = candidate("panel", 0, 1920, true, false);
        let mirror = candidate("external", 0, 1920, false, true);
        for candidates in [vec![mirror.clone(), primary.clone()], vec![primary.clone(), mirror.clone()]] {
            let index = preferred_output_index(window, &candidates).unwrap();
            assert_eq!(candidates[index].name, "panel");
        }
    }

    #[test]
    fn spanning_windows_follow_largest_real_overlap_and_stable_ties() {
        let candidates = vec![candidate("right",1920,1920,false,false),candidate("left",0,1920,true,false)];
        for (x,width,name) in [(1800,1000,"right"),(1000,1200,"left"),(1820,200,"left")] {
            let window = smithay::utils::Rectangle::new((x,0).into(),(width,800).into());
            assert_eq!(candidates[preferred_output_index(window,&candidates).unwrap()].name,name);
            let reversed: Vec<_> = candidates.iter().cloned().rev().collect();
            assert_eq!(reversed[preferred_output_index(window,&reversed).unwrap()].name,name);
        }
        assert!(preferred_output_index(smithay::utils::Rectangle::new((5000,0).into(),(100,100).into()),&candidates).is_none());
    }

    #[test]
    fn whole_number_scales_round_up() {
        assert_eq!(integer_scale(1.0), 1);
        assert_eq!(integer_scale(1.25), 2);
        assert_eq!(integer_scale(1.5), 2);
        assert_eq!(integer_scale(2.0), 2);
        assert_eq!(integer_scale(0.0), 1);
    }
}
