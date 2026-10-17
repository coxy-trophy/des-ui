//! How the applications in view share a display's workspace: where each slot is for
//! each layout, where the dividers between columns are, and how far they may move.
//!
//! Pure geometry, so that it can be tested without windows. Slot 0 is the master:
//! the whole workspace alone, or its first column. On the main display slot 0 may be
//! the terminal, when an application shares the workspace with it.

use des_common::ipc::{LayoutKind, Place};
use smithay::utils::{Logical, Point, Rectangle};

/// The space between two windows that share a workspace, as between split halves.
pub const GAP: i32 = des_common::SPLIT_GAP as i32;

/// The narrowest a column may be. A layout whose columns would be narrower shows
/// fewer of them, side by side or stacked.
pub const MIN_COLUMN: i32 = 320;

/// Dividers stay between a fifth and four fifths of the width.
pub const MIN_SHARE: f32 = 0.2;
pub const MAX_SHARE: f32 = 0.8;

/// How wide the strip either side of a divider is that a press can grab, in all.
pub const GRAB_BAND: i32 = 8;

/// How much one press of Super+Alt+Left or Right moves a divider.
pub const NUDGE: f32 = 0.05;

/// Where the dividers of one display's workspace are, per layout. Each layout keeps
/// its own, so going from one to another and back finds them as they were.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ratios {
    /// Split halves, and the terminal beside an application.
    pub split: f32,
    /// The master's share.
    pub master: f32,
    /// The two dividers of three columns.
    pub columns: [f32; 2],
}

impl Default for Ratios {
    fn default() -> Self {
        Ratios { split: 0.5, master: 0.6, columns: [1.0 / 3.0, 2.0 / 3.0] }
    }
}

impl Ratios {
    /// The first divider of a layout, as the shell is told it.
    pub fn primary(&self, kind: LayoutKind) -> f32 {
        match kind {
            LayoutKind::Single | LayoutKind::Split => self.split,
            LayoutKind::Master => self.master,
            LayoutKind::Columns => self.columns[0],
        }
    }

    /// The second divider, which only three columns have.
    pub fn secondary(&self, kind: LayoutKind) -> f32 {
        match kind {
            LayoutKind::Columns => self.columns[1],
            _ => 0.5,
        }
    }

    /// Moves one divider of a layout, kept in bounds. Divider 1 exists only for
    /// three columns, and the two of them keep a fifth of the width between them.
    pub fn set(&mut self, kind: LayoutKind, divider: usize, share: f32) {
        if !share.is_finite() {
            return;
        }
        match (kind, divider) {
            (LayoutKind::Single | LayoutKind::Split, _) => self.split = clamp_share(share),
            (LayoutKind::Master, _) => self.master = clamp_share(share),
            (LayoutKind::Columns, 0) => {
                self.columns[0] = share.clamp(MIN_SHARE, MAX_SHARE - MIN_SHARE).min(self.columns[1] - MIN_SHARE);
            }
            (LayoutKind::Columns, _) => {
                self.columns[1] = share.clamp(2.0 * MIN_SHARE, MAX_SHARE).max(self.columns[0] + MIN_SHARE);
            }
        }
    }

    /// Restored from a saved session or the shell: both dividers at once.
    pub fn restore(&mut self, kind: LayoutKind, primary: f32, secondary: f32) {
        if kind != LayoutKind::Columns {
            self.set(kind, 0, primary);
            return;
        }
        // Both at once, so that neither is held back by where the other was.
        let first = if primary.is_finite() { primary.clamp(MIN_SHARE, MAX_SHARE - MIN_SHARE) } else { 1.0 / 3.0 };
        let second = if secondary.is_finite() { secondary.clamp(2.0 * MIN_SHARE, MAX_SHARE) } else { 2.0 / 3.0 };
        self.columns = [first, second.max(first + MIN_SHARE)];
    }
}

pub fn clamp_share(share: f32) -> f32 {
    if share.is_finite() { share.clamp(MIN_SHARE, MAX_SHARE) } else { 0.5 }
}

/// A share of `width` kept so that both sides are at least a column wide, when the
/// width allows two columns at all.
fn usable_share(share: f32, width: i32) -> f32 {
    let share = clamp_share(share);
    let width = width as f32;
    if width < 2.0 * MIN_COLUMN as f32 {
        return share;
    }
    let least = MIN_COLUMN as f32 / width;
    share.clamp(least, 1.0 - least)
}

/// Two side by side, the first `share` of the width, the gap between them.
fn columns2(area: Rectangle<i32, Logical>, share: f32) -> [Rectangle<i32, Logical>; 2] {
    let usable = (area.size.w - GAP).max(2);
    let left = ((usable as f32 * usable_share(share, usable)).floor() as i32).clamp(1, usable - 1);
    let right_x = area.loc.x + left + GAP;
    [
        Rectangle::new(area.loc, (left, area.size.h).into()),
        Rectangle::new((right_x, area.loc.y).into(), ((area.loc.x + area.size.w - right_x).max(1), area.size.h).into()),
    ]
}

/// `count` one above the other, the gap between each.
fn rows(area: Rectangle<i32, Logical>, count: usize) -> Vec<Rectangle<i32, Logical>> {
    let count = count.max(1) as i32;
    let usable = (area.size.h - GAP * (count - 1)).max(count);
    let each = usable / count;
    (0..count)
        .map(|i| {
            let y = area.loc.y + i * (each + GAP);
            // The last takes what rounding left over.
            let h = if i == count - 1 { (area.loc.y + area.size.h - y).max(1) } else { each.max(1) };
            Rectangle::new((area.loc.x, y).into(), (area.size.w, h).into())
        })
        .collect()
}

/// Whether `columns` columns fit side by side.
fn fits(area: Rectangle<i32, Logical>, columns: i32) -> bool {
    area.size.w >= columns * MIN_COLUMN + (columns - 1) * GAP
}

/// The master on the left, the others stacked on the right; all stacked when even
/// two columns do not fit.
fn master(area: Rectangle<i32, Logical>, share: f32, count: usize) -> Vec<Rectangle<i32, Logical>> {
    if !fits(area, 2) {
        return rows(area, count);
    }
    let [left, right] = columns2(area, share);
    let mut slots = vec![left];
    slots.extend(rows(right, count - 1));
    slots
}

/// The layout that is actually drawn for `count` slots in use: two in use under a
/// layout of one or of three columns are the split halves, with the split's divider.
pub fn effective(kind: LayoutKind, count: usize) -> LayoutKind {
    match kind {
        LayoutKind::Single | LayoutKind::Columns if count == 2 => LayoutKind::Split,
        other => other,
    }
}

/// Where each of `count` slots is, in slot order, for a layout. `count` is how many
/// slots are in use: one past the last occupied, the master counted even when empty
/// because the terminal is there. A layout never shows more than its slots; asked for
/// fewer, it shows those in the same manner (three columns with two in use are two
/// columns). One slot is always the whole workspace.
pub fn slot_rects(kind: LayoutKind, ratios: &Ratios, area: Rectangle<i32, Logical>, count: usize) -> Vec<Rectangle<i32, Logical>> {
    // A layout of one with two in use is the split the user asked for some other way.
    let count = count.clamp(1, kind.slots().max(2));
    if count == 1 {
        return vec![area];
    }
    match effective(kind, count) {
        LayoutKind::Single | LayoutKind::Split if fits(area, 2) => columns2(area, ratios.split).to_vec(),
        LayoutKind::Master => master(area, ratios.master, count),
        LayoutKind::Columns if count == 2 && fits(area, 2) => columns2(area, ratios.split).to_vec(),
        LayoutKind::Columns if count == 3 && fits(area, 3) => {
            let usable = (area.size.w - 2 * GAP).max(3);
            let first = ((usable as f32 * ratios.columns[0]).floor() as i32).max(MIN_COLUMN.min(usable / 3));
            let second = ((usable as f32 * ratios.columns[1]).floor() as i32)
                .clamp(first + MIN_COLUMN.min(usable / 3), usable - MIN_COLUMN.min(usable / 3));
            let x1 = area.loc.x + first + GAP;
            let x2 = area.loc.x + second + 2 * GAP;
            vec![
                Rectangle::new(area.loc, (first, area.size.h).into()),
                Rectangle::new((x1, area.loc.y).into(), (x2 - GAP - x1, area.size.h).into()),
                Rectangle::new((x2, area.loc.y).into(), ((area.loc.x + area.size.w - x2).max(1), area.size.h).into()),
            ]
        }
        // Three columns that do not fit become the master and two stacked beside it.
        LayoutKind::Columns if count == 3 => master(area, 0.5, count),
        // Two that do not fit side by side go one above the other.
        _ => rows(area, count),
    }
}

/// Where the applications of one display's workspace go when it is shared another
/// way. `places` and `used` are parallel, one per application of that workspace and
/// display (the pinned one left out); `focused` is the one with the keyboard; on the
/// main display `terminal` is set, for the terminal can share the workspace there.
/// Returns the new places, and the slot to give the keyboard to when that changes.
///
/// Going to fewer slots, the application in use keeps its place and those that no
/// longer fit go out of view; going to more, the ones used most recently fill the
/// empty slots. With nothing to put beside it, a lone application on the main display
/// shares the workspace with the terminal, as a split always did.
pub fn plan(kind: LayoutKind, places: &[Place], used: &[u64], focused: Option<usize>, terminal: bool) -> (Vec<Place>, Option<usize>) {
    let mut places = places.to_vec();
    let room = kind.slots();
    let mut shown: Vec<(usize, usize)> = places.iter().enumerate().filter_map(|(i, place)| place.slot().map(|slot| (slot, i))).collect();
    shown.sort();
    let terminal_beside = terminal && !places.contains(&Place::Main) && !shown.is_empty();
    if kind == LayoutKind::Single {
        // Whole again: the application in use wins the workspace.
        let winner = focused.or_else(|| shown.first().map(|(_, i)| *i));
        for (_, i) in &shown {
            places[*i] = Place::Hidden;
        }
        if let Some(winner) = winner {
            places[winner] = Place::Main;
        }
        return (places, Some(0));
    }
    // The application in use stays in view even if its slot is past the room.
    if let Some(f) = focused {
        if places[f].slot().is_some_and(|slot| slot >= room) {
            if let Some(&(_, last)) = shown.iter().find(|(slot, _)| *slot == room - 1) {
                places[last] = Place::Hidden;
            }
            places[f] = Place::of_slot(room - 1);
        }
    }
    for (slot, i) in &shown {
        if *slot >= room && Some(*i) != focused {
            places[*i] = Place::Hidden;
        }
    }
    // The empty slots fill with those out of view, most recently used first; the
    // terminal keeps the first slot when it was beside an application.
    let mut waiting: Vec<usize> = (0..places.len()).filter(|i| places[*i] == Place::Hidden).collect();
    waiting.sort_by_key(|i| std::cmp::Reverse((used.get(*i).copied().unwrap_or(0), *i)));
    let mut waiting = waiting.into_iter();
    let first = if terminal_beside { 1 } else { 0 };
    for slot in first..room {
        if places.contains(&Place::of_slot(slot)) {
            continue;
        }
        match waiting.next() {
            Some(i) => places[i] = Place::of_slot(slot),
            None => break,
        }
    }
    // A lone application on the main display moves over for the terminal.
    let in_view: Vec<usize> = (0..places.len()).filter(|i| places[*i] != Place::Hidden).collect();
    if let [only] = in_view[..] {
        if terminal && places[only] == Place::Main {
            places[only] = Place::Side;
        }
    }
    let focus = match focused {
        Some(f) => Some(places[f].slot().unwrap_or(0)),
        // The terminal had the keyboard and still has.
        None if !places.contains(&Place::Main) => Some(0),
        None => None,
    };
    (places, focus)
}

/// Super+Enter: the application at `focused` becomes the master, and the master
/// takes its slot. Beside the terminal, it takes the terminal's place. Returns
/// whether anything moved.
pub fn swap_into_master(places: &mut [Place], focused: usize) -> bool {
    let Some(slot) = places.get(focused).and_then(|place| place.slot()).filter(|slot| *slot > 0) else {
        return false;
    };
    if let Some(master) = places.iter().position(|place| *place == Place::Main) {
        places[master] = Place::of_slot(slot);
    }
    places[focused] = Place::Main;
    true
}

/// A divider being dragged: on which display, which one, in which layout, across
/// which workspace, and when the windows were last told their sizes.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub screen: usize,
    pub divider: usize,
    pub kind: LayoutKind,
    pub area: Rectangle<i32, Logical>,
    pub last_configure: std::time::Instant,
}

/// How often windows are told their new sizes while a divider is dragged; in
/// between they are drawn scaled into place.
pub const RECONFIGURE_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// A divider between two columns that a press can grab: which one, and where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Divider {
    pub index: usize,
    /// The middle of the gap.
    pub x: i32,
    pub top: i32,
    pub bottom: i32,
}

/// The dividers between side-by-side columns, left to right. Stacked rows have none:
/// only widths are shared out by hand.
pub fn dividers(kind: LayoutKind, ratios: &Ratios, area: Rectangle<i32, Logical>, count: usize) -> Vec<Divider> {
    let slots = slot_rects(kind, ratios, area, count);
    let mut columns: Vec<Rectangle<i32, Logical>> = Vec::new();
    for slot in slots {
        // A slot stacked under another shares its column.
        if columns.iter().any(|column| column.loc.x == slot.loc.x) {
            continue;
        }
        columns.push(slot);
    }
    columns.sort_by_key(|column| column.loc.x);
    columns
        .windows(2)
        .enumerate()
        .map(|(index, pair)| Divider {
            index,
            x: pair[0].loc.x + pair[0].size.w + GAP / 2,
            top: area.loc.y,
            bottom: area.loc.y + area.size.h,
        })
        .collect()
}

/// The divider a press at `pos` would grab, if one is that close.
pub fn divider_at(dividers: &[Divider], pos: Point<f64, Logical>) -> Option<Divider> {
    let half = GRAB_BAND as f64 / 2.0;
    dividers
        .iter()
        .copied()
        .filter(|divider| pos.y >= divider.top as f64 && pos.y < divider.bottom as f64)
        .filter(|divider| (pos.x - divider.x as f64).abs() <= half)
        .min_by(|a, b| (pos.x - a.x as f64).abs().total_cmp(&(pos.x - b.x as f64).abs()))
}

/// The share a divider is dragged to with the pointer at `x`, as `Ratios::set` takes
/// it. The second divider of three columns lies past one more gap.
pub fn share_at(kind: LayoutKind, divider: usize, area: Rectangle<i32, Logical>, x: f64) -> f32 {
    // `kind` is the effective layout, as `effective` gives it for the slots in use.
    let gaps = if kind == LayoutKind::Columns { 2 } else { 1 };
    let usable = (area.size.w - gaps * GAP).max(1) as f64;
    let before = (divider as i32 * GAP) as f64 + GAP as f64 / 2.0;
    ((x - area.loc.x as f64 - before) / usable) as f32
}

/// The divider that Super+Alt+Left or Right moves for the slot with the keyboard: the
/// one on its right, or for the last column the one on its left.
pub fn nudged_divider(kind: LayoutKind, focused_slot: usize) -> usize {
    match kind {
        LayoutKind::Columns => focused_slot.min(1),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((100, 50).into(), (w, h).into())
    }

    /// Slots never overlap and never leave the area.
    fn sound(slots: &[Rectangle<i32, Logical>], area: Rectangle<i32, Logical>) {
        for (i, a) in slots.iter().enumerate() {
            assert!(a.size.w >= 1 && a.size.h >= 1, "{a:?}");
            assert!(area.contains_rect(*a), "{a:?} outside {area:?}");
            for b in &slots[i + 1..] {
                assert!(!a.overlaps(*b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn one_in_use_is_the_whole_workspace_in_every_layout() {
        for kind in LayoutKind::ALL {
            assert_eq!(slot_rects(kind, &Ratios::default(), area(1600, 900), 1), vec![area(1600, 900)]);
        }
    }

    #[test]
    fn a_split_at_one_half_is_the_halves_it_always_was() {
        let whole = area(1280, 1032);
        let halves = des_common::Rect { x: 100.0, y: 50.0, width: 1280.0, height: 1032.0 }.halves();
        let slots = slot_rects(LayoutKind::Split, &Ratios::default(), whole, 2);
        assert_eq!((slots[0].loc.x, slots[0].size.w), (halves.0.x as i32, halves.0.width as i32));
        assert_eq!((slots[1].loc.x, slots[1].size.w), (halves.1.x as i32, halves.1.width as i32));
        // A single layout with a second application in use splits as before.
        assert_eq!(slot_rects(LayoutKind::Single, &Ratios::default(), whole, 2), slots);
    }

    #[test]
    fn every_layout_fills_its_slots_soundly_at_several_sizes() {
        for (w, h) in [(3840, 2100), (1920, 1032), (1280, 700), (900, 600), (500, 400), (40, 30)] {
            let whole = area(w, h);
            for kind in LayoutKind::ALL {
                for count in 1..=kind.slots() {
                    let slots = slot_rects(kind, &Ratios::default(), whole, count);
                    assert_eq!(slots.len(), count.max(1).min(kind.slots().max(2)), "{kind:?} {count} at {w}x{h}");
                    sound(&slots, whole);
                }
            }
        }
    }

    #[test]
    fn three_columns_share_the_width_and_degrade_when_they_cannot_fit() {
        let wide = area(1920, 1000);
        let slots = slot_rects(LayoutKind::Columns, &Ratios::default(), wide, 3);
        assert!(slots.iter().all(|slot| slot.loc.y == 50 && slot.size.h == 1000));
        assert!(slots[0].loc.x < slots[1].loc.x && slots[1].loc.x < slots[2].loc.x);
        assert_eq!(slots[2].loc.x + slots[2].size.w, 100 + 1920);
        // Not room for three columns of 320: the first, and two stacked beside it.
        let narrow = area(900, 800);
        let slots = slot_rects(LayoutKind::Columns, &Ratios::default(), narrow, 3);
        assert_eq!(slots[1].loc.x, slots[2].loc.x);
        assert!(slots[1].loc.y < slots[2].loc.y);
        // Not room even for two: three rows.
        let tiny = area(500, 900);
        let slots = slot_rects(LayoutKind::Columns, &Ratios::default(), tiny, 3);
        assert!(slots.iter().all(|slot| slot.loc.x == 100 && slot.size.w == 500));
    }

    #[test]
    fn master_and_stack_keep_the_master_left_and_stack_the_rest() {
        let ratios = Ratios { master: 0.7, ..Ratios::default() };
        let slots = slot_rects(LayoutKind::Master, &ratios, area(2000, 1000), 5);
        assert_eq!(slots.len(), 5);
        assert!((slots[0].size.w as f32 / 1998.0 - 0.7).abs() < 0.01);
        assert!(slots[1..].iter().all(|slot| slot.loc.x == slots[1].loc.x));
        assert_eq!(slots[4].loc.y + slots[4].size.h, 1050);
    }

    #[test]
    fn dividers_stay_in_bounds_and_keep_columns_usable() {
        let mut ratios = Ratios::default();
        ratios.set(LayoutKind::Split, 0, 0.05);
        assert_eq!(ratios.split, MIN_SHARE);
        ratios.set(LayoutKind::Split, 0, 3.0);
        assert_eq!(ratios.split, MAX_SHARE);
        ratios.set(LayoutKind::Split, 0, f32::NAN);
        assert_eq!(ratios.split, MAX_SHARE);
        ratios.set(LayoutKind::Columns, 0, 0.7);
        assert!(ratios.columns[0] <= ratios.columns[1] - MIN_SHARE + 1e-6);
        ratios.set(LayoutKind::Columns, 1, 0.1);
        assert!(ratios.columns[1] >= ratios.columns[0] + MIN_SHARE - 1e-6);
        // At four fifths of 700 the right column would be 140: kept at 320 instead.
        let slots = slot_rects(LayoutKind::Split, &Ratios { split: 0.8, ..Ratios::default() }, area(700, 500), 2);
        assert!(slots[1].size.w >= MIN_COLUMN, "{slots:?}");
    }

    #[test]
    fn a_press_within_the_band_grabs_the_nearest_divider() {
        let whole = area(1600, 900);
        let ratios = Ratios::default();
        let dividers = dividers(LayoutKind::Columns, &ratios, whole, 3);
        assert_eq!(dividers.len(), 2);
        let at = |x: f64, y: f64| divider_at(&dividers, (x, y).into()).map(|d| d.index);
        assert_eq!(at(dividers[0].x as f64, 300.0), Some(0));
        assert_eq!(at(dividers[1].x as f64 + 4.0, 300.0), Some(1));
        assert_eq!(at(dividers[1].x as f64 + 5.0, 300.0), None);
        assert_eq!(at(dividers[0].x as f64, 20.0), None, "above the workspace");
        // Stacked rows have no divider of their own.
        assert_eq!(super::dividers(LayoutKind::Master, &ratios, whole, 4).len(), 1);
        assert!(super::dividers(LayoutKind::Single, &ratios, whole, 1).is_empty());
    }

    #[test]
    fn dragging_a_divider_lands_it_under_the_pointer() {
        let whole = area(1600, 900);
        let mut ratios = Ratios::default();
        for (kind, divider) in [(LayoutKind::Split, 0), (LayoutKind::Master, 0), (LayoutKind::Columns, 0), (LayoutKind::Columns, 1)] {
            let count = kind.slots().min(3);
            let target = dividers(kind, &ratios, whole, count)[divider].x + 60;
            ratios.set(kind, divider, share_at(kind, divider, whole, target as f64));
            let now = dividers(kind, &ratios, whole, count)[divider].x;
            assert!((now - target).abs() <= 2, "{kind:?} {divider}: {now} vs {target}");
        }
        assert_eq!(nudged_divider(LayoutKind::Columns, 0), 0);
        assert_eq!(nudged_divider(LayoutKind::Columns, 2), 1);
        assert_eq!(nudged_divider(LayoutKind::Master, 3), 0);
    }

    #[test]
    fn going_to_more_slots_fills_them_with_the_most_recently_used() {
        use des_common::ipc::Place::{Extra, Hidden, Main, Side};
        // One in view, three waiting: used at 5, 9 and 1.
        let places = [Main, Hidden, Hidden, Hidden];
        let used = [10, 5, 9, 1];
        let (places_now, focus) = plan(LayoutKind::Columns, &places, &used, Some(0), false);
        assert_eq!(places_now, vec![Main, Extra(2), Side, Hidden]);
        assert_eq!(focus, Some(0));
        let (all, _) = plan(LayoutKind::Master, &places, &used, Some(0), false);
        assert_eq!(all, vec![Main, Extra(2), Side, Extra(3)]);
    }

    #[test]
    fn going_to_fewer_slots_keeps_the_one_in_use() {
        use des_common::ipc::Place::{Extra, Hidden, Main, Side};
        let places = [Main, Side, Extra(2)];
        // The third column has the keyboard; a split keeps it, in the second slot.
        let (split, focus) = plan(LayoutKind::Split, &places, &[3, 2, 9], Some(2), false);
        assert_eq!(split, vec![Main, Hidden, Side]);
        assert_eq!(focus, Some(1));
        let (single, focus) = plan(LayoutKind::Single, &places, &[3, 2, 9], Some(2), false);
        assert_eq!(single, vec![Hidden, Hidden, Main]);
        assert_eq!(focus, Some(0));
    }

    #[test]
    fn the_terminal_shares_the_main_display_with_a_lone_application() {
        use des_common::ipc::Place::{Hidden, Main, Side};
        // One application and nothing waiting: it moves over beside the terminal.
        assert_eq!(plan(LayoutKind::Split, &[Main], &[1], Some(0), true).0, vec![Side]);
        // Not on another display, where there is no terminal.
        assert_eq!(plan(LayoutKind::Split, &[Main], &[1], Some(0), false).0, vec![Main]);
        // Beside the terminal already: more columns keep the terminal first.
        let (places, focus) = plan(LayoutKind::Columns, &[Side, Hidden], &[1, 2], None, true);
        assert_eq!(places, vec![Side, des_common::ipc::Place::Extra(2)]);
        assert_eq!(focus, Some(0), "the terminal keeps the keyboard");
    }

    #[test]
    fn swapping_into_the_master_slot_trades_places() {
        use des_common::ipc::Place::{Extra, Main, Side};
        let mut places = [Main, Side, Extra(2)];
        assert!(swap_into_master(&mut places, 2));
        assert_eq!(places, [Extra(2), Side, Main]);
        // Already the master: nothing to do.
        assert!(!swap_into_master(&mut places, 2));
        // Beside the terminal, it takes the terminal's place.
        let mut beside = [Side];
        assert!(swap_into_master(&mut beside, 0));
        assert_eq!(beside, [Main]);
    }

    #[test]
    fn restoring_both_dividers_does_not_trip_on_the_old_ones() {
        let mut ratios = Ratios { columns: [0.6, 0.8], ..Ratios::default() };
        ratios.restore(LayoutKind::Columns, 0.25, 0.5);
        assert_eq!(ratios.columns, [0.25, 0.5]);
        ratios.restore(LayoutKind::Master, 0.65, 0.5);
        assert_eq!(ratios.master, 0.65);
    }
}
