//! Touchpad swipes with three or four fingers move between workspaces and open the
//! overview. Four fingers do the same as three, for those who keep three for dragging.
//! Other gestures, and these while the screen is locked, go to the application under
//! the pointer as they are.

/// What a finished swipe asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// The workspace to the left: the fingers moved right.
    Previous,
    /// The workspace to the right: the fingers moved left.
    Next,
    OpenOverview,
    CloseOverview,
}

/// How far the fingers must travel, in the touchpad's logical units, and by how much
/// more along one axis than the other, before a swipe counts.
const HORIZONTAL: f64 = 120.0;
const VERTICAL: f64 = 100.0;
const DOMINANCE: f64 = 1.5;

/// Whether a swipe with this many fingers is the desktop's rather than the
/// application's.
pub fn ours(fingers: u32) -> bool {
    matches!(fingers, 3 | 4)
}

/// A swipe in progress that the desktop took.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Swipe {
    pub dx: f64,
    pub dy: f64,
}

impl Swipe {
    pub fn update(&mut self, dx: f64, dy: f64) {
        self.dx += dx;
        self.dy += dy;
    }

    /// What the swipe asks for once the fingers lift. `overview_open` decides whether
    /// down means anything.
    pub fn outcome(&self, cancelled: bool, overview_open: bool) -> Option<Outcome> {
        if cancelled {
            return None;
        }
        let (ax, ay) = (self.dx.abs(), self.dy.abs());
        if ax >= HORIZONTAL && ax >= ay * DOMINANCE {
            return Some(if self.dx < 0.0 { Outcome::Next } else { Outcome::Previous });
        }
        if ay >= VERTICAL && ay >= ax * DOMINANCE {
            return match (self.dy < 0.0, overview_open) {
                (true, false) => Some(Outcome::OpenOverview),
                (false, true) => Some(Outcome::CloseOverview),
                _ => None,
            };
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swipe(dx: f64, dy: f64) -> Swipe {
        let mut swipe = Swipe::default();
        // Arrives in pieces, as a touchpad reports it.
        for _ in 0..10 {
            swipe.update(dx / 10.0, dy / 10.0);
        }
        swipe
    }

    #[test]
    fn three_and_four_fingers_are_the_desktop_s() {
        assert!(ours(3) && ours(4));
        assert!(!ours(2) && !ours(5) && !ours(1));
    }

    #[test]
    fn sideways_moves_between_workspaces_as_the_fingers_lead() {
        assert_eq!(swipe(-200.0, 10.0).outcome(false, false), Some(Outcome::Next));
        assert_eq!(swipe(200.0, -20.0).outcome(false, false), Some(Outcome::Previous));
        assert_eq!(swipe(-200.0, 10.0).outcome(true, false), None, "a cancelled swipe does nothing");
        assert_eq!(swipe(-80.0, 0.0).outcome(false, false), None, "too short");
        assert_eq!(swipe(-150.0, 140.0).outcome(false, false), None, "too diagonal to tell");
    }

    #[test]
    fn up_opens_the_overview_and_down_closes_it() {
        assert_eq!(swipe(0.0, -150.0).outcome(false, false), Some(Outcome::OpenOverview));
        assert_eq!(swipe(0.0, -150.0).outcome(false, true), None, "already open");
        assert_eq!(swipe(5.0, 150.0).outcome(false, true), Some(Outcome::CloseOverview));
        assert_eq!(swipe(5.0, 150.0).outcome(false, false), None, "nothing to close");
    }
}
