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
    /// What an extended display shows while the screen is locked and no saver runs:
    /// the time and where to unlock. Like a saver, it never draws the session.
    SecondaryLock(usize),
}

pub fn shell_role(app_id: &str) -> ShellRole {
    let (index, lock) = match (app_id.strip_prefix("des-ui-saver-"), app_id.strip_prefix("des-ui-lock-")) {
        (Some(index), _) => (index, false),
        (_, Some(index)) => (index, true),
        _ => return if app_id.is_empty() { ShellRole::Unassigned } else { ShellRole::Desktop },
    };
    match index.parse::<usize>() {
        Ok(screen) if screen != 0 && index == screen.to_string() => {
            if lock { ShellRole::SecondaryLock(screen) } else { ShellRole::SecondarySaver(screen) }
        }
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
    /// A lock display rather than a saver: its cycle counts locks, not savers.
    pub lock: bool,
}

/// What the lock-cycle rules need to know about a secondary surface. Kept apart
/// from the window so the rules can be tested without a Wayland client.
pub trait Stamped {
    fn screen(&self) -> usize;
    fn lock(&self) -> bool;
    fn cycle(&self) -> Option<u64>;
    fn set_cycle(&mut self, cycle: Option<u64>);
}

impl Stamped for SecondarySaver {
    fn screen(&self) -> usize { self.screen }
    fn lock(&self) -> bool { self.lock }
    fn cycle(&self) -> Option<u64> { self.cycle }
    fn set_cycle(&mut self, cycle: Option<u64>) { self.cycle = cycle; }
}

/// The cycle a newly registered lock display gets: this lock's, or none yet if
/// the shell locked before the compositor did.
pub fn lock_surface_cycle(locked: bool, lock_cycle: u64) -> Option<u64> {
    locked.then_some(lock_cycle)
}

/// The compositor locks: a new lock cycle begins. Lock displays the shell made
/// as it locked, before this, belong to this lock; one from an earlier lock keeps
/// its old cycle and stays hidden.
pub fn begin_lock<T: Stamped>(entries: &mut [T], lock_cycle: &mut u64) {
    *lock_cycle = lock_cycle.wrapping_add(1);
    for entry in entries.iter_mut().filter(|entry| entry.lock() && entry.cycle().is_none()) {
        entry.set_cycle(Some(*lock_cycle));
    }
}

/// The surface to draw on secondary `screen`: the newest saver of this saver
/// cycle while a saver runs, else the newest lock display of this lock, else none.
/// Nothing is shown while unlocked.
pub fn pick_secondary<T: Stamped>(entries: &[T], screen: usize, locked: bool, saver: Option<u64>, lock_cycle: u64) -> Option<&T> {
    if !locked {
        return None;
    }
    let newest = |lock: bool, cycle: u64| entries.iter().rev()
        .find(|entry| entry.lock() == lock && entry.screen() == screen && secondary_visible(locked, Some(cycle), entry.cycle()));
    saver.and_then(|cycle| newest(false, cycle)).or_else(|| newest(true, lock_cycle))
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
        assert_eq!(shell_role("des-ui-lock-1"), ShellRole::SecondaryLock(1));
        assert_eq!(shell_role("des-ui-lock-12"), ShellRole::SecondaryLock(12));
        for id in ["des-ui-lock-", "des-ui-lock-0", "des-ui-lock-01", "des-ui-lock-1x"] {
            assert_eq!(shell_role(id), ShellRole::Unassigned, "{id}");
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

    /// A secondary surface without a window: (name, screen, lock, cycle).
    #[derive(Debug, PartialEq)]
    struct Fake(&'static str, usize, bool, Option<u64>);

    impl Stamped for Fake {
        fn screen(&self) -> usize { self.1 }
        fn lock(&self) -> bool { self.2 }
        fn cycle(&self) -> Option<u64> { self.3 }
        fn set_cycle(&mut self, cycle: Option<u64>) { self.3 = cycle; }
    }

    fn shown(entries: &[Fake], screen: usize, locked: bool, saver: Option<u64>, lock_cycle: u64) -> Option<&'static str> {
        pick_secondary(entries, screen, locked, saver, lock_cycle).map(|entry| entry.0)
    }

    #[test]
    fn a_lock_display_from_one_lock_is_hidden_on_the_next() {
        let mut lock_cycle = 0;
        let mut entries = Vec::new();
        begin_lock(&mut entries, &mut lock_cycle);
        entries.push(Fake("first", 1, true, lock_surface_cycle(true, lock_cycle)));
        assert_eq!(shown(&entries, 1, true, None, lock_cycle), Some("first"));
        // Unlocked: nothing shows, whatever is still registered.
        assert_eq!(shown(&entries, 1, false, None, lock_cycle), None);
        // The next lock: the old surface stays registered but belongs to the last lock.
        begin_lock(&mut entries, &mut lock_cycle);
        assert_eq!(entries[0].3, Some(1));
        assert_eq!(shown(&entries, 1, true, None, lock_cycle), None);
        entries.push(Fake("second", 1, true, lock_surface_cycle(true, lock_cycle)));
        assert_eq!(shown(&entries, 1, true, None, lock_cycle), Some("second"));
    }

    #[test]
    fn a_lock_display_made_before_the_compositor_locked_joins_that_lock() {
        let mut lock_cycle = 4;
        // The shell locked first; the compositor is still unlocked.
        let mut entries = vec![Fake("early", 2, true, lock_surface_cycle(false, lock_cycle))];
        assert_eq!(entries[0].3, None);
        assert_eq!(shown(&entries, 2, false, None, lock_cycle), None);
        begin_lock(&mut entries, &mut lock_cycle);
        assert_eq!(lock_cycle, 5);
        assert_eq!(entries[0].3, Some(5));
        assert_eq!(shown(&entries, 2, true, None, lock_cycle), Some("early"));
        // It does not show on another display.
        assert_eq!(shown(&entries, 1, true, None, lock_cycle), None);
    }

    #[test]
    fn adoption_leaves_savers_and_older_lock_displays_alone() {
        let mut lock_cycle = 1;
        let mut entries = vec![
            Fake("old lock", 1, true, Some(1)),
            Fake("saver", 1, false, None),
            Fake("pending lock", 1, true, None),
        ];
        begin_lock(&mut entries, &mut lock_cycle);
        assert_eq!(entries.iter().map(|entry| entry.3).collect::<Vec<_>>(), [Some(1), None, Some(2)]);
    }

    #[test]
    fn a_running_saver_of_this_cycle_takes_precedence_over_the_lock_display() {
        let entries = vec![
            Fake("lock", 1, true, Some(3)),
            Fake("saver", 1, false, Some(7)),
        ];
        assert_eq!(shown(&entries, 1, true, Some(7), 3), Some("saver"));
        // No saver running: the lock display.
        assert_eq!(shown(&entries, 1, true, None, 3), Some("lock"));
        // A saver from another saver cycle never shows; the lock display does.
        assert_eq!(shown(&entries, 1, true, Some(8), 3), Some("lock"));
        // A lock display never stands in for a saver, nor a saver for a lock display.
        let swapped = vec![Fake("lock", 1, true, Some(7)), Fake("saver", 1, false, Some(3))];
        assert_eq!(shown(&swapped, 1, true, Some(7), 3), None);
    }

    #[test]
    fn the_newest_surface_of_the_right_cycle_wins() {
        let entries = vec![
            Fake("older", 1, true, Some(2)),
            Fake("newer", 1, true, Some(2)),
            Fake("stale", 1, true, Some(1)),
        ];
        assert_eq!(shown(&entries, 1, true, None, 2), Some("newer"));
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
