//! One-use permission to cover the existing lock screen, and filtering for the input
//! that wakes it. Neither state can change whether the desktop is locked.

use std::collections::HashSet;
use smithay::reexports::wayland_server::backend::ObjectId;

#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Unavailable,
    Pending,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Device(String),
    Hands,
    Virtual(ObjectId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    Key(u32),
    Button(u32),
    Touch(i32),
    Swipe,
    Pinch,
    Hold,
    ScrollHorizontal,
    ScrollVertical,
}

impl Control {
    /// Additional parts of the same wake gesture or chord start out hidden too.
    fn follows(self, held: Self) -> bool {
        use Control::*;
        matches!((self, held),
            (Key(_), Key(_)) | (Touch(_), Touch(_)) |
            (ScrollHorizontal | ScrollVertical, ScrollHorizontal | ScrollVertical))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Begin,
    Update,
    End,
}

#[derive(Default)]
pub struct Saver {
    phase: Phase,
    swallowed: HashSet<(Source, Control)>,
}

impl Saver {
    pub fn request(&mut self) {
        self.phase = Phase::Pending;
    }

    pub fn active(&self) -> bool {
        self.phase == Phase::Active
    }

    pub fn needs_source(&self) -> bool {
        self.active() || !self.swallowed.is_empty()
    }

    /// A premature request cannot approve anything. The real idle request remains
    /// valid for the later confirmation, unless any of the idle guards fail.
    pub fn approve(&mut self, locked: bool, eligible: bool) -> Option<bool> {
        if !eligible {
            self.cancel();
            return Some(false);
        }
        if self.active() {
            return None;
        }
        if locked && self.phase == Phase::Pending {
            self.phase = Phase::Active;
            return Some(true);
        }
        Some(false)
    }

    /// Returns whether the shell needs to uncover the ordinary lock screen.
    pub fn cancel(&mut self) -> bool {
        std::mem::take(&mut self.phase) == Phase::Active
    }

    /// Return (dismissed, swallowed). Keep filtering the wake sequence even
    /// after the animation disappears, until each press/contact gets its end.
    pub fn input(
        &mut self,
        source: Option<&Source>,
        changes: impl IntoIterator<Item = (Control, Change)>,
    ) -> (bool, bool) {
        let dismissed = self.cancel();
        let mut swallowed = dismissed;
        if let Some(source) = source {
            // Use the state at the start of a frame for both axes of a scroll.
            let mut changes = changes.into_iter().peekable();
            let joined = changes.peek().is_some_and(|(control, _)| {
                self.swallowed.iter().any(|(owner, held)| owner == source && control.follows(*held))
            });
            for (control, change) in changes {
                let key = (source.clone(), control);
                match change {
                    Change::End => swallowed |= self.swallowed.remove(&key),
                    Change::Begin | Change::Update
                        if dismissed || joined || self.swallowed.contains(&key) =>
                    {
                        self.swallowed.insert(key);
                        swallowed = true;
                    }
                    _ => {}
                }
            }
        }
        (dismissed, swallowed)
    }

    pub fn has_wake_keys(&self, source: &Source) -> bool {
        self.swallowed.iter().any(|(owner, control)| owner == source && matches!(control, Control::Key(_)))
    }

    pub fn cancel_touches(&mut self, source: &Source) -> bool {
        let before = self.swallowed.len();
        self.swallowed.retain(|(owner, control)| owner != source || !matches!(control, Control::Touch(_)));
        before != self.swallowed.len()
    }

    pub fn forget_source(&mut self, source: &Source) {
        self.swallowed.retain(|(owner, _)| owner != source);
    }
}

/// Authenticated shell toplevels wait for their ID before they can be assigned
/// any role. The saver prefix is reserved even when the index is malformed.
#[derive(Debug, PartialEq, Eq)]
pub enum ShellRole {
    Unassigned,
    Desktop,
    SecondarySaver(usize),
}

pub fn shell_role(app_id: &str) -> ShellRole {
    let Some(index) = app_id.strip_prefix("des-ui-saver-") else {
        return if app_id.is_empty() { ShellRole::Unassigned } else { ShellRole::Desktop };
    };
    match index.parse::<usize>() {
        Ok(screen) if screen != 0 && index == screen.to_string() => ShellRole::SecondarySaver(screen),
        _ => ShellRole::Unassigned,
    }
}

pub fn secondary_visible(locked: bool, approved_cycle: Option<u64>, window_cycle: Option<u64>) -> bool {
    locked && approved_cycle.is_some() && approved_cycle == window_cycle
}

/// A saver window is render-only. It never joins the Space, so neither a stale
/// pointer focus nor an oversized buffer can give it access to another output.
pub struct SecondarySaver {
    pub screen: usize,
    pub window: smithay::desktop::Window,
    pub id: smithay::backend::renderer::element::Id,
    pub cycle: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_secondary_indices_get_the_saver_role() {
        assert_eq!(shell_role(""), ShellRole::Unassigned);
        assert_eq!(shell_role("des-ui"), ShellRole::Desktop);
        assert_eq!(shell_role("des-ui-greeter"), ShellRole::Desktop);
        assert_eq!(shell_role("des-ui-saver-1"), ShellRole::SecondarySaver(1));
        assert_eq!(shell_role("des-ui-saver-10"), ShellRole::SecondarySaver(10));
        for id in ["des-ui-saver-", "des-ui-saver-0", "des-ui-saver-01", "des-ui-saver-+1", "des-ui-saver-1-extra"] {
            assert_eq!(shell_role(id), ShellRole::Unassigned);
        }
    }

    #[test]
    fn saver_window_visibility_ends_with_its_approved_lock_cycle() {
        assert!(secondary_visible(true, Some(2), Some(2)));
        assert!(!secondary_visible(false, Some(2), Some(2)));
        assert!(!secondary_visible(true, None, Some(2)));
        assert!(!secondary_visible(true, Some(3), Some(2)));
        assert!(!secondary_visible(true, None, None));
    }

    #[test]
    fn saver_needs_an_idle_request_and_confirmed_lock() {
        let mut saver = Saver::default();
        assert_eq!(saver.approve(true, true), Some(false));
        saver.request();
        assert_eq!(saver.approve(false, true), Some(false));
        assert_eq!(saver.approve(true, true), Some(true));
        assert_eq!(saver.approve(true, true), None);
        assert!(saver.cancel());
        assert_eq!(saver.approve(true, true), Some(false));
    }

    #[test]
    fn activity_or_display_off_retires_a_pending_request() {
        let mut saver = Saver::default();
        saver.request();
        assert_eq!(saver.input(None, []), (false, false));
        assert_eq!(saver.approve(true, true), Some(false));
        saver.request();
        assert_eq!(saver.approve(true, false), Some(false));
        assert_eq!(saver.approve(true, true), Some(false));
    }

    #[test]
    fn wake_key_repeats_chords_and_releases_cannot_reach_the_lock_screen() {
        let mut saver = Saver::default();
        saver.request();
        assert_eq!(saver.approve(true, true), Some(true));
        let keyboard = Source::Device("keyboard".into());
        let mut input = |control, change| saver.input(Some(&keyboard), [(control, change)]);
        assert_eq!(input(Control::Key(38), Change::Begin), (true, true));
        assert_eq!(input(Control::Key(38), Change::Begin), (false, true));
        assert_eq!(input(Control::Key(24), Change::Begin), (false, true));
        assert_eq!(input(Control::Key(38), Change::End), (false, true));
        assert_eq!(input(Control::Key(24), Change::End), (false, true));
        assert_eq!(input(Control::Key(38), Change::Begin), (false, false));
    }

    #[test]
    fn wake_button_release_is_suppressed_for_the_same_source_only() {
        let mut saver = Saver::default();
        saver.request();
        assert_eq!(saver.approve(true, true), Some(true));
        let button = Control::Button(272);
        assert_eq!(saver.input(Some(&Source::Hands), [(button, Change::Begin)]), (true, true));
        let mouse = Source::Device("mouse".into());
        assert_eq!(saver.input(Some(&mouse), [(button, Change::End)]), (false, false));
        assert_eq!(saver.input(Some(&Source::Hands), [(button, Change::End)]), (false, true));
        assert_eq!(saver.input(Some(&Source::Hands), [(button, Change::Begin)]), (false, false));
    }

    #[test]
    fn wake_touch_swallows_every_contact_until_the_sequence_ends() {
        let mut saver = Saver::default();
        saver.request();
        assert_eq!(saver.approve(true, true), Some(true));
        let screen = Source::Device("touchscreen".into());
        let mut input = |control, change| saver.input(Some(&screen), [(control, change)]);
        assert_eq!(input(Control::Touch(0), Change::Begin), (true, true));
        assert_eq!(input(Control::Touch(1), Change::Begin), (false, true));
        assert_eq!(input(Control::Touch(0), Change::End), (false, true));
        assert_eq!(input(Control::Touch(1), Change::Update), (false, true));
        assert_eq!(input(Control::Touch(1), Change::End), (false, true));
        assert_eq!(input(Control::Touch(0), Change::Begin), (false, false));
    }

    #[test]
    fn two_axis_finger_scroll_cannot_send_a_partial_wake_sequence() {
        let mut saver = Saver::default();
        saver.request();
        assert_eq!(saver.approve(true, true), Some(true));
        let pad = Source::Device("touchpad".into());
        let mut scroll = |changes| saver.input(Some(&pad), changes);
        assert_eq!(scroll([(Control::ScrollVertical, Change::Update), (Control::ScrollHorizontal, Change::Update)]), (true, true));
        assert_eq!(scroll([(Control::ScrollVertical, Change::End), (Control::ScrollHorizontal, Change::Update)]), (false, true));
        assert_eq!(scroll([(Control::ScrollVertical, Change::Update), (Control::ScrollHorizontal, Change::End)]), (false, true));
        assert_eq!(saver.input(Some(&pad), [(Control::ScrollVertical, Change::End)]), (false, true));
        assert_eq!(saver.input(Some(&pad), [(Control::ScrollVertical, Change::Update)]), (false, false));
    }
}
