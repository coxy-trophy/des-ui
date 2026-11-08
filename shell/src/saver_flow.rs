//! The shell may paint an idle saver only after the compositor approves it.
//! An old approval cannot start a saver on a later manual lock.

use std::time::Instant;

#[derive(Default)]
pub(crate) enum IdleSaver {
    #[default]
    Inactive,
    Preparing,
    Requested,
    Visible(Instant),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    None,
    Show,
    Dismiss,
}

impl IdleSaver {
    pub(crate) fn begin(&mut self) {
        *self = Self::Preparing;
    }

    pub(crate) fn cancel(&mut self) -> bool {
        !matches!(std::mem::take(self), Self::Inactive)
    }

    pub(crate) fn update(&mut self, locked: bool, approved: bool, now: Instant) -> Action {
        if !locked {
            return Action::None;
        }
        match (&*self, approved) {
            (Self::Preparing, false) => {
                *self = Self::Requested;
                Action::Show
            }
            (Self::Requested, true) => {
                *self = Self::Visible(now);
                Action::None
            }
            (Self::Visible(_), false) => {
                *self = Self::Inactive;
                Action::None
            }
            (Self::Inactive | Self::Preparing, true) => Action::Dismiss,
            _ => Action::None,
        }
    }

    pub(crate) fn started(&self) -> Option<Instant> {
        match self {
            Self::Visible(started) => Some(*started),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saver_waits_for_both_lock_and_compositor_approval() {
        let mut saver = IdleSaver::default();
        let now = Instant::now();
        saver.begin();
        assert_eq!(saver.update(false, false, now), Action::None);
        assert_eq!(saver.started(), None);
        assert_eq!(saver.update(true, false, now), Action::Show);
        assert_eq!(saver.started(), None);
        assert_eq!(saver.update(true, true, now), Action::None);
        assert_eq!(saver.started(), Some(now));
        assert_eq!(saver.update(true, false, now), Action::None);
        assert_eq!(saver.started(), None);
        assert_eq!(saver.update(true, false, now), Action::None);
    }

    #[test]
    fn cancelled_idle_request_cannot_cover_a_manual_lock() {
        let mut saver = IdleSaver::default();
        let now = Instant::now();
        saver.begin();
        assert!(saver.cancel());
        assert_eq!(saver.update(true, false, now), Action::None);

        saver.begin();
        assert_eq!(saver.update(true, false, now), Action::Show);
        assert!(saver.cancel());
        assert_eq!(saver.update(true, true, now), Action::Dismiss);
        assert_eq!(saver.started(), None);
    }
}
