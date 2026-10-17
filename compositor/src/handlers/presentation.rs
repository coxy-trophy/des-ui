//! presentation-time: an application learns when each of its frames reached the
//! screen, to the vertical blank, and how often the display refreshes. Video players
//! and games pace themselves by it instead of guessing.
//!
//! On the hardware, the feedback for a frame is gathered when it is queued and
//! answered at its page flip, with the kernel's time stamp. In a window, the host
//! gives no such moment; frames are answered as shown when they are handed over.

use std::time::Duration;

use smithay::{
    delegate_presentation,
    desktop::{Space, Window, utils::OutputPresentationFeedback},
    output::Output,
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    utils::{Clock, Monotonic, Time},
    wayland::presentation::Refresh,
};

use crate::DesComp;

delegate_presentation!(DesComp);

/// The feedback of every window drawn on `output` this frame.
pub fn feedback_for(space: &Space<Window>, output: &Output) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    for window in space.elements() {
        // A window counts as shown on the first display it is on, the one it is told
        // to draw for; a window across two answers once, for that one.
        let Some(home) = space.outputs_for_element(window).into_iter().next() else {
            continue;
        };
        if &home != output {
            continue;
        }
        window.take_presentation_feedback(&mut feedback, |_, _| Some(home.clone()), |_, _| wp_presentation_feedback::Kind::empty());
    }
    feedback
}

/// When a page flip happened, as the kernel says, or else now.
pub fn flip_time(metadata: Option<&smithay::backend::drm::DrmEventMetadata>) -> Time<Monotonic> {
    match metadata.map(|metadata| metadata.time) {
        Some(smithay::backend::drm::DrmEventTime::Monotonic(time)) => Time::from(time),
        // A realtime stamp cannot be compared with the clock clients were told of.
        _ => Clock::<Monotonic>::new().now(),
    }
}

/// Answers a frame's feedback at its page flip.
pub fn presented(feedback: &mut OutputPresentationFeedback, metadata: Option<&smithay::backend::drm::DrmEventMetadata>, refresh: Duration) {
    let sequence = metadata.map_or(0, |metadata| metadata.sequence as u64);
    let flags = wp_presentation_feedback::Kind::Vsync | wp_presentation_feedback::Kind::HwClock | wp_presentation_feedback::Kind::HwCompletion;
    feedback.presented(flip_time(metadata), refresh_of(refresh), sequence, flags);
}

/// A refresh period clients can use; none is better than a wrong one.
pub fn refresh_of(period: Duration) -> Refresh {
    if period.is_zero() { Refresh::Unknown } else { Refresh::Fixed(period) }
}

impl DesComp {
    /// In a window: every window's frame counts as shown once handed to the host.
    pub fn presented_in_window(&self) {
        let now = Clock::<Monotonic>::new().now();
        for output in self.space.outputs() {
            let mut feedback = feedback_for(&self.space, output);
            feedback.presented(now, Refresh::Unknown, 0, wp_presentation_feedback::Kind::empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_refresh_is_said_to_be_unknown() {
        assert!(matches!(refresh_of(Duration::ZERO), Refresh::Unknown));
        assert!(matches!(refresh_of(Duration::from_micros(16_667)), Refresh::Fixed(period) if period == Duration::from_micros(16_667)));
    }

    #[test]
    fn a_flip_without_a_kernel_stamp_is_now() {
        let before = Clock::<Monotonic>::new().now();
        let at = flip_time(None);
        assert!(Duration::from(at) >= Duration::from(before));
    }
}
