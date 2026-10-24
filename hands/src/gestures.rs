//! From a hand's joints to what it means for the pointer.
//!
//! | The hand                     | Does                                          |
//! |------------------------------|-----------------------------------------------|
//! | open                         | moves the pointer, as a finger on a touchpad  |
//! | thumb and index pinched      | clicks; a second pinch and movement locks a drag |
//! | thumb and middle pinched     | clicks the right button                        |
//! | index and middle up          | scrolls with the hand up and down; a quick    |
//! |                              | sweep sideways goes to the next workspace     |
//! | a fist                       | holds still, to rest the hand                 |
//! | anything else                | holds still                                   |
//!
//! The pointer moves as the middle of the palm does, which stays put while fingers
//! pinch: by as much again and more the faster the hand goes, mirrored so that it goes
//! the way the hand does. Like a finger on a touchpad, the hand can be anywhere the
//! camera sees it, and is lifted, to start again elsewhere, by making a fist. Its
//! movement is smoothed by a One Euro filter: heavily while the hand is nearly still,
//! so that the pointer does not shake, and hardly at all when it moves fast, so that
//! it does not lag.

use crate::geometry::Point;
use crate::hand::Hand;

/// How near the thumb has to come to a fingertip to pinch, and how far it has to go
/// again to let go, in palm lengths. The gap between them keeps a pinch from flickering.
const PINCH: f32 = 0.33;
const UNPINCH: f32 = 0.50;
/// How far a fingertip is from the wrist, against the base of its finger, when the
/// finger is straight and when it is curled.
const STRAIGHT: f32 = 1.5;
const CURLED: f32 = 1.25;
/// With two fingers up, how much shorter the other two have to reach than they do:
/// folded fingers are not always folded as far as `CURLED`, but are always shorter.
const SHORTER: f32 = 0.3;
/// The second tap in this interval can become a latched drag.
const DOUBLE_TAP: f64 = 0.45;
/// Intentional movement during the second pinch, in camera-view fractions.
const DRAG_SLOP: f32 = 0.012;
/// How far the pointer goes, in screens, for the hand going across the camera's
/// whole view slowly; and how much more for each view a second faster, up to a most.
const GAIN: f32 = 1.8;
const SPEEDING: f32 = 1.5;
const MOST_GAIN: f32 = 6.0;
/// How near the edge of the picture, as a fraction of it, the palm is too near to be
/// followed well.
const EDGE: f32 = 0.06;
/// After a button is let go, the pointer holds still this long, in seconds, while
/// the fingers part.
const RELEASE_HOLD: f64 = 0.12;
/// A sweep sideways with two fingers up: this far across the view, within this long.
const SWEEP: f32 = 0.20;
const SWEEP_WITHIN: f64 = 0.4;
/// How far up or down the hand must go, as a fraction of the view, before two fingers
/// scroll: less is the hand shaking.
const SCROLL_STILL: f32 = 0.012;
/// How long a new pose must last, in frames, before it counts.
const STEADY: u32 = 2;

/// One Euro filter, for one coordinate: Casiez, Roussel and Vogel, 2012.
#[derive(Debug, Clone)]
pub struct OneEuro {
    /// The least smoothing, in hertz: lower is smoother when still.
    min_cutoff: f64,
    /// How much speed lessens smoothing.
    beta: f64,
    last: Option<(f64, f64, f64)>,
}

impl OneEuro {
    pub fn new(min_cutoff: f64, beta: f64) -> OneEuro {
        OneEuro { min_cutoff, beta, last: None }
    }

    fn alpha(cutoff: f64, dt: f64) -> f64 {
        let tau = 1.0 / (std::f64::consts::TAU * cutoff);
        1.0 / (1.0 + tau / dt)
    }

    /// The smoothed value of `value`, measured at `time` seconds.
    pub fn filter(&mut self, value: f64, time: f64) -> f64 {
        let Some((last_value, last_speed, last_time)) = self.last else {
            self.last = Some((value, 0.0, time));
            return value;
        };
        let dt = (time - last_time).max(1e-3);
        let speed = (value - last_value) / dt;
        let speed = last_speed + Self::alpha(1.0, dt) * (speed - last_speed);
        let cutoff = self.min_cutoff + self.beta * speed.abs();
        let smoothed = last_value + Self::alpha(cutoff, dt) * (value - last_value);
        self.last = Some((smoothed, speed, time));
        smoothed
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// The pointer moves by this much, as fractions of the screen across and down.
    Motion { dx: f32, dy: f32 },
    Press(Button),
    Release(Button),
    /// Scroll by this much of the screen's height: up is less than nothing.
    Scroll(f32),
    /// Two fingers swept to this side.
    Sweep(Side),
    /// A fist: the pointer rests.
    Rest,
    /// The fist opened again.
    Wake,
    /// The hand left the camera's view.
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    Open,
    Pinch,
    RightPinch,
    Two,
    Fist,
    Other,
}

/// The fingers, each as the joints at its base, its middle and its tip.
const FINGERS: [(usize, usize); 4] = [(5, 8), (9, 12), (13, 16), (17, 20)];

/// What the hand is doing. `before` is the pose it was in, for a pinch to let go
/// only once the thumb is well clear.
pub fn pose(hand: &Hand, before: Pose) -> Pose {
    pose_with_preferences(hand, before, Preferences::default())
}

/// Hand comfort settings. Positive sensitivity accepts a wider pinch gap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preferences {
    pub pinch_sensitivity: f32,
    pub drag_lock: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { pinch_sensitivity: 0.0, drag_lock: true }
    }
}

impl Preferences {
    fn normalized(self) -> Self {
        Self {
            pinch_sensitivity: if self.pinch_sensitivity.is_finite() { self.pinch_sensitivity.clamp(-1.0, 1.0) } else { 0.0 },
            ..self
        }
    }
}

fn pose_with_preferences(hand: &Hand, before: Pose, preferences: Preferences) -> Pose {
    let joints = &hand.joints;
    let palm = joints[0].distance(joints[9]).max(1.0);
    let reach: Vec<f32> = FINGERS
        .iter()
        .map(|(base, tip)| joints[0].distance(joints[*tip]) / joints[0].distance(joints[*base]).max(1.0))
        .collect();
    let curled = |finger: usize| reach[finger] < CURLED;
    let straight = |finger: usize| reach[finger] > STRAIGHT;
    // A fist first: its thumb lies across the fingers, close by the index fingertip,
    // where it would seem to pinch. The other three fingers tell it apart, being
    // folded in a fist and not in a pinch.
    if curled(1) && curled(2) && curled(3) && !straight(0) {
        return Pose::Fist;
    }
    let sensitivity = preferences.normalized().pinch_sensitivity * 0.10;
    let index = joints[4].distance(joints[8]) / palm;
    let middle = joints[4].distance(joints[12]) / palm;
    let index_limit = if before == Pose::Pinch { UNPINCH } else { PINCH } + sensitivity;
    let middle_limit = if before == Pose::RightPinch { UNPINCH } else { PINCH } + sensitivity;
    // The nearest fingertip distinguishes the two pinches even when adjacent
    // fingertips are closer together than the relaxed activation threshold.
    if index <= middle && index < index_limit {
        return Pose::Pinch;
    }
    if middle < index && middle < middle_limit {
        return Pose::RightPinch;
    }
    if straight(0) && straight(1) && reach[2].max(reach[3]) < reach[0].min(reach[1]) - SHORTER {
        Pose::Two
    } else if (0..4).filter(|finger| !curled(*finger)).count() >= 2 {
        Pose::Open
    } else {
        Pose::Other
    }
}

/// The middle of the palm: the wrist and the bases of the four fingers.
fn palm_middle(hand: &Hand) -> Point {
    let joints = [0, 5, 9, 13, 17].map(|joint| hand.joints[joint]);
    Point::new(joints.iter().map(|point| point.x).sum::<f32>() / 5.0, joints.iter().map(|point| point.y).sum::<f32>() / 5.0)
}

/// A point of the camera's picture as fractions of it across and down, mirrored, so
/// that what goes towards the hand's right goes towards the screen's right.
pub fn in_view(point: Point, width: f32, height: f32) -> (f32, f32) {
    (1.0 - point.x / width.max(1.0), point.y / height.max(1.0))
}

/// Whether the hand is at the edge of the camera's view, where it is soon lost: its
/// palm near the edge, or its wrist beyond it.
pub fn at_edge(hand: &Hand, width: f32, height: f32) -> bool {
    let (across, down) = in_view(palm_middle(hand), width, height);
    let (wrist_across, wrist_down) = in_view(hand.joints[0], width, height);
    let near = |value: f32| !(EDGE..=1.0 - EDGE).contains(&value);
    let beyond = |value: f32| !(0.0..=1.0).contains(&value);
    near(across) || near(down) || beyond(wrist_across) || beyond(wrist_down)
}

/// How far the pointer goes for the hand moving `by`, as fractions of the view, in
/// `seconds`: further the faster it goes.
pub fn pointer_motion(by: (f32, f32), seconds: f64) -> (f32, f32) {
    let speed = by.0.hypot(by.1) / seconds.max(1e-3) as f32;
    let gain = (GAIN * (1.0 + SPEEDING * speed)).min(MOST_GAIN);
    (by.0 * gain, by.1 * gain)
}

/// How an index pinch currently controls the left button.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pinching {
    Clicked,
    Second { origin: (f32, f32) },
    Grip,
    Unlocking,
    Ignored,
}

/// Follows the hand from frame to frame and says what it does.
pub struct Gestures {
    preferences: Preferences,
    pose: Pose,
    coming: (Pose, u32),
    seen: bool,
    /// A neutral pose must be seen before a newly visible hand can click.
    armed: bool,
    x: OneEuro,
    y: OneEuro,
    last: Option<((f32, f32), f64)>,
    pinch: Option<Pinching>,
    held_left: bool,
    drag_locked: bool,
    last_click: Option<f64>,
    hold_until: f64,
    scroll_from: Option<f32>,
    sweep: Vec<(f64, f32)>,
    swept: bool,
}

impl Default for Gestures {
    fn default() -> Gestures {
        Self::with_preferences(Preferences::default())
    }
}

impl Gestures {
    pub fn with_preferences(preferences: Preferences) -> Self {
        Self {
            preferences: preferences.normalized(),
            pose: Pose::Other,
            coming: (Pose::Other, 0),
            seen: false,
            armed: false,
            x: OneEuro::new(1.0, 4.0),
            y: OneEuro::new(1.0, 4.0),
            last: None,
            pinch: None,
            held_left: false,
            drag_locked: false,
            last_click: None,
            hold_until: 0.0,
            scroll_from: None,
            sweep: Vec::new(),
            swept: false,
        }
    }

    pub fn pose(&self) -> Pose {
        self.pose
    }

    fn release_left(&mut self, events: &mut Vec<Event>) {
        if std::mem::take(&mut self.held_left) {
            events.push(Event::Release(Button::Left));
        }
        self.drag_locked = false;
    }

    fn cancel(&mut self, events: &mut Vec<Event>) {
        self.release_left(events);
        self.pinch = None;
        self.last_click = None;
        self.armed = false;
        self.last = None;
        self.x.reset();
        self.y.reset();
    }

    /// Takes one frame. Unsafe tracking (including an edge-clipped hand) always
    /// lets go; returning pinched cannot click until a neutral pose arms it again.
    pub fn update(&mut self, hand: Option<&Hand>, width: f32, height: f32, time: f64) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(hand) = hand.filter(|hand| !at_edge(hand, width, height)) else {
            self.cancel(&mut events);
            if std::mem::take(&mut self.seen) {
                events.push(Event::Gone);
            }
            self.pose = Pose::Other;
            self.coming = (Pose::Other, 0);
            self.scroll_from = None;
            self.sweep.clear();
            self.swept = false;
            return events;
        };
        self.seen = true;
        let seen = pose_with_preferences(hand, self.pose, self.preferences);
        self.coming = if seen == self.coming.0 { (seen, self.coming.1.saturating_add(1)) } else { (seen, 1) };
        let before = self.pose;
        if seen != self.pose && self.coming.1 >= STEADY {
            self.pose = seen;
        }
        let pose = self.pose;
        let raw = in_view(palm_middle(hand), width, height);

        // A fist or an incompatible pose cancels even during pose debounce.
        if matches!(seen, Pose::Fist | Pose::Other | Pose::Two) && self.held_left {
            self.cancel(&mut events);
        }
        if seen == Pose::Other {
            self.armed = false;
            self.last_click = None;
        }
        if pose != before {
            // Preserve the held state across generic pinch exit: releasing a
            // grip or pending second tap must not also trigger a right click.
            let was_holding_left = self.held_left;
            if before == Pose::Pinch {
                if !self.drag_locked {
                    self.release_left(&mut events);
                }
                self.pinch = None;
                self.hold_until = time + RELEASE_HOLD;
            }
            if before == Pose::RightPinch {
                self.hold_until = time + RELEASE_HOLD;
            }
            if before == Pose::Fist {
                events.push(Event::Wake);
            }
            if before == Pose::Two {
                self.scroll_from = None;
                self.sweep.clear();
                self.swept = false;
            }
            // Fresh anchors on pose changes avoid smoothing a finger transition
            // into movement of a click target or into the start of a drag.
            self.last = None;
            self.x.reset();
            self.y.reset();
            match pose {
                Pose::Pinch if self.drag_locked => {
                    self.release_left(&mut events);
                    self.last_click = None;
                    self.pinch = Some(Pinching::Unlocking);
                }
                Pose::Pinch if !self.armed => self.pinch = Some(Pinching::Ignored),
                Pose::Pinch if !self.preferences.drag_lock => {
                    events.push(Event::Press(Button::Left));
                    self.held_left = true;
                    self.pinch = Some(Pinching::Grip);
                }
                Pose::Pinch => {
                    if self.last_click.take().is_some_and(|last| (0.0..=DOUBLE_TAP).contains(&(time - last))) {
                        events.push(Event::Press(Button::Left));
                        self.held_left = true;
                        self.pinch = Some(Pinching::Second { origin: raw });
                    } else {
                        events.extend([Event::Press(Button::Left), Event::Release(Button::Left)]);
                        self.last_click = Some(time);
                        self.pinch = Some(Pinching::Clicked);
                    }
                }
                Pose::RightPinch => {
                    self.release_left(&mut events);
                    self.last_click = None;
                    if self.armed && !was_holding_left {
                        events.extend([Event::Press(Button::Right), Event::Release(Button::Right)]);
                    }
                }
                Pose::Fist => {
                    self.cancel(&mut events);
                    events.push(Event::Rest);
                }
                _ => {}
            }
        }
        if seen == pose && matches!(pose, Pose::Open | Pose::Fist | Pose::Two) {
            self.armed = true;
        }

        let smooth = (self.x.filter(raw.0 as f64, time) as f32, self.y.filter(raw.1 as f64, time) as f32);
        match pose {
            Pose::Two if seen == pose => {
                let down = smooth.1;
                match self.scroll_from {
                    Some(from) if (down - from).abs() > SCROLL_STILL => {
                        events.push(Event::Scroll((down - from) * 2.0));
                        self.scroll_from = Some(down);
                    }
                    Some(_) => {}
                    None => self.scroll_from = Some(down),
                }
                self.sweep.push((time, raw.0));
                self.sweep.retain(|(at, _)| time - at <= SWEEP_WITHIN);
                if let (false, Some((_, first))) = (self.swept, self.sweep.first()) {
                    let moved = raw.0 - first;
                    if moved.abs() > SWEEP {
                        events.push(Event::Sweep(if moved < 0.0 { Side::Left } else { Side::Right }));
                        self.swept = true;
                    }
                }
            }
            Pose::Open | Pose::Pinch => {
                let mut motion = self.last.map_or((0.0, 0.0), |(from, then)| {
                    pointer_motion((smooth.0 - from.0, smooth.1 - from.1), time - then)
                });
                if seen != pose || time < self.hold_until {
                    motion = (0.0, 0.0);
                }
                if pose == Pose::Pinch && !self.drag_locked {
                    match self.pinch {
                        Some(Pinching::Second { origin }) if (raw.0 - origin.0).hypot(raw.1 - origin.1) >= DRAG_SLOP && seen == pose => {
                            self.drag_locked = true;
                        }
                        Some(Pinching::Grip) => {}
                        _ => motion = (0.0, 0.0),
                    }
                }
                if motion != (0.0, 0.0) {
                    events.push(Event::Motion { dx: motion.0, dy: motion.1 });
                }
                self.last = Some((smooth, time));
            }
            _ => {}
        }
        if !matches!(pose, Pose::Open | Pose::Pinch) {
            self.last = None;
            if pose != Pose::Two {
                self.x.reset();
                self.y.reset();
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand standing upright with its wrist at `wrist` and its palm 100 pixels long:
    /// each finger straight or curled, the thumb out to the side, or touching the tip
    /// of the index or middle finger.
    fn hand_at(wrist: Point, straight: [bool; 4], touching: Option<usize>) -> Hand {
        let mut joints = [Point::new(0.0, 0.0); 21];
        let mut set = |joint: usize, x: f32, y: f32| joints[joint] = Point::new(wrist.x + x, wrist.y + y);
        set(0, 0.0, 0.0);
        for (finger, straight) in straight.into_iter().enumerate() {
            let x = -30.0 + finger as f32 * 20.0;
            let base = 5 + finger * 4;
            set(base, x, -100.0);
            let (middle, upper, tip) = if straight { (-140.0, -165.0, -190.0) } else { (-130.0, -105.0, -95.0) };
            set(base + 1, x, middle);
            set(base + 2, x, upper);
            set(base + 3, x, tip);
        }
        set(1, -40.0, -20.0);
        set(2, -60.0, -45.0);
        set(3, -75.0, -65.0);
        set(4, -85.0, -85.0);
        if let Some(tip) = touching {
            joints[4] = joints[tip];
        }
        Hand { joints, depth: [0.0; 21], presence: 1.0, right: 1.0 }
    }

    fn hand(straight: [bool; 4], touching: Option<usize>) -> Hand {
        hand_at(Point::new(320.0, 400.0), straight, touching)
    }

    const OPEN: [bool; 4] = [true; 4];

    #[test]
    fn tells_poses_apart() {
        assert_eq!(pose(&hand(OPEN, None), Pose::Other), Pose::Open);
        assert_eq!(pose(&hand([false; 4], None), Pose::Other), Pose::Fist);
        assert_eq!(pose(&hand([true, true, false, false], None), Pose::Other), Pose::Two);
        assert_eq!(pose(&hand(OPEN, Some(8)), Pose::Other), Pose::Pinch);
        // A fist's thumb lies by the index fingertip, but it is still a fist.
        assert_eq!(pose(&hand([false; 4], Some(8)), Pose::Other), Pose::Fist);
        // Two fingers up with the others only half folded, as hands do.
        let mut half = hand([true, true, false, false], None);
        for (joint, y) in [(16, -150.0), (20, -150.0)] {
            half.joints[joint].y = 400.0 + y;
        }
        assert_eq!(pose(&half, Pose::Other), Pose::Two);
        assert_eq!(pose(&hand([true, false, false, false], None), Pose::Other), Pose::Other);
    }

    #[test]
    fn a_pinch_lets_go_only_once_the_thumb_is_clear() {
        let mut nearly = hand(OPEN, Some(8));
        // Between activation and release distances: only an existing pinch stays on.
        nearly.joints[4].x -= 42.0;
        assert_eq!(pose(&nearly, Pose::Open), Pose::Open);
        assert_eq!(pose(&nearly, Pose::Pinch), Pose::Pinch);
    }

    #[test]
    fn the_pointer_goes_the_way_the_hand_does_and_further_when_fast() {
        // The camera sees the hand move to its right, which is the hand's left.
        let (left, _) = in_view(Point::new(500.0, 240.0), 640.0, 480.0);
        let (right, _) = in_view(Point::new(140.0, 240.0), 640.0, 480.0);
        assert!(left < right);
        let slow = pointer_motion((0.01, 0.0), 0.1);
        let fast = pointer_motion((0.1, 0.0), 0.1);
        assert!(slow.0 > 0.01 && fast.0 / 0.1 > slow.0 / 0.01);
        assert!(pointer_motion((1.0, 0.0), 0.01).0 <= MOST_GAIN);
    }

    #[test]
    fn knows_a_hand_at_the_edge_of_the_view() {
        assert!(!at_edge(&hand_at(Point::new(320.0, 400.0), OPEN, None), 640.0, 480.0));
        // The wrist below the picture, as with a hand held low in front of a laptop.
        assert!(at_edge(&hand_at(Point::new(320.0, 520.0), OPEN, None), 640.0, 480.0));
        assert!(at_edge(&hand_at(Point::new(10.0, 300.0), OPEN, None), 640.0, 480.0));
    }

    #[test]
    fn the_hand_lifted_by_a_fist_comes_back_without_a_jump() {
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 4], 0.0);
        run(&mut gestures, &vec![Some(hand_at(Point::new(320.0, 400.0), [false; 4], None)); 4], 1.0);
        // Far across the view while a fist, then open again where it is: no motion.
        run(&mut gestures, &vec![Some(hand_at(Point::new(100.0, 400.0), [false; 4], None)); 4], 2.0);
        let events = run(&mut gestures, &vec![Some(hand_at(Point::new(100.0, 400.0), OPEN, None)); 4], 3.0);
        assert!(!events.iter().any(|event| matches!(event, Event::Motion { .. })), "{events:?}");
    }

    /// Runs frames 1/30 s apart and gathers what they did.
    fn run(gestures: &mut Gestures, frames: &[Option<Hand>], from: f64) -> Vec<Event> {
        frames
            .iter()
            .enumerate()
            .flat_map(|(index, hand)| gestures.update(hand.as_ref(), 640.0, 480.0, from + index as f64 / 30.0))
            .collect()
    }

    fn arm(gestures: &mut Gestures) {
        run(gestures, &vec![Some(hand(OPEN, None)); 3], 0.0);
    }

    fn buttons(events: Vec<Event>) -> Vec<Event> {
        events.into_iter().filter(|event| matches!(event, Event::Press(_) | Event::Release(_))).collect()
    }

    #[test]
    fn relaxed_fingers_point_and_a_gentle_index_pinch_clicks_once() {
        let mut relaxed = hand(OPEN, None);
        for tip in [8, 12, 16, 20] { relaxed.joints[tip].y = 260.0; }
        assert_eq!(pose(&relaxed, Pose::Other), Pose::Open);
        // Two relaxed fingers point; only the deliberate straight two-finger
        // shape (covered separately) enters scrolling.
        for tip in [16, 20] { relaxed.joints[tip].y = 305.0; }
        assert_eq!(pose(&relaxed, Pose::Other), Pose::Open);
        let mut gentle = hand(OPEN, Some(8));
        gentle.joints[4].x -= 30.0;
        assert_eq!(pose(&gentle, Pose::Open), Pose::Pinch);
        let mut gestures = Gestures::default();
        arm(&mut gestures);
        assert_eq!(run(&mut gestures, &vec![Some(gentle); 4], 1.0), [Event::Press(Button::Left), Event::Release(Button::Left)]);
        let events = run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.2);
        assert!(buttons(events).is_empty());
    }

    #[test]
    fn a_long_index_pinch_never_right_clicks_or_moves_the_aim() {
        let mut gestures = Gestures::default();
        arm(&mut gestures);
        let frames: Vec<_> = (0..60).map(|step| Some(hand_at(Point::new(320.0 - step as f32, 400.0), OPEN, Some(8)))).collect();
        assert_eq!(run(&mut gestures, &frames, 1.0), [Event::Press(Button::Left), Event::Release(Button::Left)]);
    }

    #[test]
    fn two_short_index_taps_preserve_an_ordinary_double_click() {
        let mut gestures = Gestures::default();
        arm(&mut gestures);
        let first = run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0);
        assert_eq!(buttons(first), [Event::Press(Button::Left), Event::Release(Button::Left)]);
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.1);
        let second = run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.2);
        assert_eq!(buttons(second), [Event::Press(Button::Left)]);
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.3)), [Event::Release(Button::Left)]);
    }

    fn latch(gestures: &mut Gestures) {
        arm(gestures);
        run(gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0);
        run(gestures, &vec![Some(hand(OPEN, None)); 3], 1.1);
        run(gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.2);
        let moving: Vec<_> = (1..=5).map(|step| Some(hand_at(Point::new(320.0 - step as f32 * 5.0, 400.0), OPEN, Some(8)))).collect();
        let events = run(gestures, &moving, 1.3);
        assert!(events.iter().any(|event| matches!(event, Event::Motion { .. })), "{events:?}");
        assert!(buttons(events).is_empty());
    }

    #[test]
    fn second_pinch_motion_latches_and_next_index_pinch_releases_once() {
        let mut gestures = Gestures::default();
        latch(&mut gestures);
        assert!(buttons(run(&mut gestures, &vec![Some(hand_at(Point::new(295.0, 400.0), OPEN, None)); 3], 1.5)).is_empty());
        let moving: Vec<_> = (1..=5).map(|step| Some(hand_at(Point::new(295.0 - step as f32 * 5.0, 400.0), OPEN, None))).collect();
        assert!(run(&mut gestures, &moving, 1.7).iter().any(|event| matches!(event, Event::Motion { .. })));
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand_at(Point::new(270.0, 400.0), OPEN, Some(8))); 3], 2.0)), [Event::Release(Button::Left)]);
        assert!(buttons(run(&mut gestures, &vec![Some(hand_at(Point::new(270.0, 400.0), OPEN, None)); 3], 2.2)).is_empty());
    }

    #[test]
    fn lost_hand_fist_edge_and_other_cancel_a_latched_drag() {
        for interruption in [None, Some(hand([false; 4], None)), Some(hand_at(Point::new(10.0, 400.0), OPEN, None)), Some(hand([true, false, false, false], None))] {
            let mut gestures = Gestures::default();
            latch(&mut gestures);
            let events = run(&mut gestures, &vec![interruption.clone(); 3], 1.5);
            assert_eq!(buttons(events), [Event::Release(Button::Left)], "{interruption:?}");
            assert!(buttons(run(&mut gestures, &vec![interruption; 3], 1.7)).is_empty());
        }
    }

    #[test]
    fn entering_view_pinched_requires_a_neutral_pose_before_clicking() {
        let mut gestures = Gestures::default();
        assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 5], 0.0)).is_empty());
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 0.3);
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 0.5)), [Event::Press(Button::Left), Event::Release(Button::Left)]);
        gestures.update(None, 640.0, 480.0, 1.0);
        assert!(run(&mut gestures, &vec![Some(hand_at(Point::new(100.0, 300.0), OPEN, Some(8))); 5], 1.1).iter().all(|event| !matches!(event, Event::Press(_) | Event::Release(_) | Event::Motion { .. })));
    }

    #[test]
    fn a_middle_pinch_right_clicks_once_and_never_repeats_while_held() {
        assert_eq!(pose(&hand(OPEN, Some(12)), Pose::Open), Pose::RightPinch);
        let mut gestures = Gestures::default();
        arm(&mut gestures);
        let events = run(&mut gestures, &vec![Some(hand(OPEN, Some(12))); 60], 1.0);
        assert_eq!(events, [Event::Press(Button::Right), Event::Release(Button::Right)]);
        assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 3.1)).is_empty());
    }

    #[test]
    fn a_middle_pinch_safely_ends_a_latched_drag_without_right_clicking() {
        let mut gestures = Gestures::default();
        latch(&mut gestures);
        run(&mut gestures, &vec![Some(hand_at(Point::new(295.0, 400.0), OPEN, None)); 3], 1.5);
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand_at(Point::new(295.0, 400.0), OPEN, Some(12))); 3], 1.7)), [Event::Release(Button::Left)]);
        assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 2.0)).is_empty());
    }

    #[test]
    fn switching_a_held_left_pinch_to_middle_pinch_only_releases_left() {
        for drag_lock in [false, true] {
            let mut gestures = Gestures::with_preferences(Preferences { drag_lock, ..Preferences::default() });
            arm(&mut gestures);
            if drag_lock {
                run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0);
                run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.1);
            }
            assert_eq!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.2)), [Event::Press(Button::Left)]);
            assert_eq!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(12))); 3], 1.3)), [Event::Release(Button::Left)]);
            assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(12))); 3], 1.5)).is_empty());
            assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.7)).is_empty());
        }
    }

    #[test]
    fn grip_mode_holds_until_unpinched_and_moves_while_held() {
        let mut gestures = Gestures::with_preferences(Preferences { drag_lock: false, ..Preferences::default() });
        arm(&mut gestures);
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0)), [Event::Press(Button::Left)]);
        let moving: Vec<_> = (1..=5).map(|step| Some(hand_at(Point::new(320.0 - step as f32 * 5.0, 400.0), OPEN, Some(8)))).collect();
        let events = run(&mut gestures, &moving, 1.2);
        assert!(events.iter().any(|event| matches!(event, Event::Motion { .. })));
        assert!(buttons(events).is_empty());
        assert_eq!(buttons(run(&mut gestures, &vec![Some(hand_at(Point::new(295.0, 400.0), OPEN, None)); 3], 1.5)), [Event::Release(Button::Left)]);
        assert!(buttons(gestures.update(None, 640.0, 480.0, 2.0)).is_empty());
    }

    #[test]
    fn grip_mode_also_releases_on_tracking_loss_and_fist() {
        for interruption in [None, Some(hand([false; 4], None))] {
            let mut gestures = Gestures::with_preferences(Preferences { drag_lock: false, ..Preferences::default() });
            arm(&mut gestures);
            run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0);
            assert_eq!(buttons(run(&mut gestures, &vec![interruption; 3], 1.2)), [Event::Release(Button::Left)]);
        }
    }

    #[test]
    fn pinch_sensitivity_is_bounded_and_invalid_values_use_the_default() {
        let mut gentle = hand(OPEN, Some(8));
        gentle.joints[4].x -= 38.0;
        let classify = |sensitivity| pose_with_preferences(&gentle, Pose::Open, Preferences { pinch_sensitivity: sensitivity, ..Preferences::default() });
        assert_eq!(classify(0.0), Pose::Open);
        assert_eq!(classify(1.0), Pose::Pinch);
        assert_eq!(classify(100.0), classify(1.0));
        assert_eq!(classify(-100.0), classify(-1.0));
        assert_eq!(classify(f32::NAN), classify(0.0));
        assert_eq!(classify(f32::INFINITY), classify(0.0));
        let mut sensitive = Gestures::with_preferences(Preferences { pinch_sensitivity: 1.0, ..Preferences::default() });
        arm(&mut sensitive);
        assert_eq!(buttons(run(&mut sensitive, &vec![Some(gentle); 3], 1.0)), [Event::Press(Button::Left), Event::Release(Button::Left)]);
    }

    #[test]
    fn a_cancelled_second_pinch_cannot_click_again_on_exit() {
        let mut gestures = Gestures::default();
        arm(&mut gestures);
        run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.0);
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.1);
        run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 3], 1.2);
        assert_eq!(buttons(gestures.update(None, 640.0, 480.0, 1.3)), [Event::Release(Button::Left)]);
        assert!(buttons(run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 1.5)).is_empty());
    }

    #[test]
    fn a_fist_rests_the_pointer() {
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 0.0);
        let fist: Vec<Option<Hand>> = (0..6).map(|step| Some(hand_at(Point::new(320.0 + step as f32 * 20.0, 400.0), [false; 4], None))).collect();
        let events = run(&mut gestures, &fist, 1.0);
        assert_eq!(events, [Event::Rest]);
        let events = run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 2.0);
        assert_eq!(events.first(), Some(&Event::Wake));
    }

    #[test]
    fn two_fingers_scroll_and_sweep() {
        let two = [true, true, false, false];
        let mut gestures = Gestures::default();
        // Up the picture: scrolls up.
        let rising: Vec<Option<Hand>> = (0..8).map(|step| Some(hand_at(Point::new(320.0, 400.0 - step as f32 * 10.0), two, None))).collect();
        let events = run(&mut gestures, &rising, 0.0);
        assert!(events.iter().any(|event| matches!(event, Event::Scroll(by) if *by < 0.0)), "{events:?}");
        assert!(!events.iter().any(|event| matches!(event, Event::Motion { .. })));
        // Quickly across the picture to its left, which is the hand's right: once.
        let mut gestures = Gestures::default();
        let across: Vec<Option<Hand>> = (0..10).map(|step| Some(hand_at(Point::new(450.0 - step as f32 * 30.0, 300.0), two, None))).collect();
        let sweeps: Vec<Event> = run(&mut gestures, &across, 0.0).into_iter().filter(|event| matches!(event, Event::Sweep(_))).collect();
        assert_eq!(sweeps, [Event::Sweep(Side::Right)]);
    }

    #[test]
    fn a_reacquired_hand_can_sweep_again() {
        let two = [true, true, false, false];
        let across: Vec<Option<Hand>> = (0..10)
            .map(|step| Some(hand_at(Point::new(450.0 - step as f32 * 30.0, 300.0), two, None)))
            .collect();
        let mut gestures = Gestures::default();
        for time in [0.0, 1.0] {
            let events = run(&mut gestures, &across, time);
            assert_eq!(events.iter().filter(|event| matches!(event, Event::Sweep(_))).count(), 1);
            gestures.update(None, 640.0, 480.0, time + 0.5);
        }
    }

    #[test]
    fn two_finger_scrolling_filters_small_alternating_jitter() {
        let two = [true, true, false, false];
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand_at(Point::new(320.0, 400.0), two, None)); 10], 0.0);
        // Fourteen pixels between frames exceeds the scroll threshold, but this
        // seven-pixel shake around a stationary palm should be smoothed away.
        let shaken: Vec<Option<Hand>> = (0..60)
            .map(|frame| Some(hand_at(Point::new(320.0, 400.0 + if frame % 2 == 0 { 7.0 } else { -7.0 }), two, None)))
            .collect();
        let events = run(&mut gestures, &shaken, 0.5);
        assert!(!events.iter().any(|event| matches!(event, Event::Scroll(_))), "{events:?}");
    }

    #[test]
    fn smoothing_steadies_a_still_hand_and_follows_a_moving_one() {
        let mut filter = OneEuro::new(1.0, 4.0);
        // Shaking by a hundredth of the screen about one place.
        let shaken: Vec<f64> = (0..60).map(|frame| filter.filter(0.5 + if frame % 2 == 0 { 0.01 } else { -0.01 }, frame as f64 / 30.0)).collect();
        assert!(shaken[30..].iter().all(|value| (value - 0.5).abs() < 0.005), "{:?}", &shaken[30..]);
        // Across the screen in a third of a second: close behind it by the end.
        let mut filter = OneEuro::new(1.0, 4.0);
        let mut last = 0.0;
        for frame in 0..=10 {
            last = filter.filter(frame as f64 / 10.0, frame as f64 / 30.0);
        }
        assert!(last > 0.8, "{last}");
    }
}
