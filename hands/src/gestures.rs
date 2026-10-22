//! From a hand's joints to what it means for the pointer.
//!
//! | The hand                     | Does                                          |
//! |------------------------------|-----------------------------------------------|
//! | open                         | moves the pointer, as a finger on a touchpad  |
//! | thumb and index pinched      | clicks; moved while pinched, drags; held      |
//! |                              | still a moment, clicks the right button       |
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
const PINCH: f32 = 0.25;
const UNPINCH: f32 = 0.40;
/// How far a fingertip is from the wrist, against the base of its finger, when the
/// finger is straight and when it is curled.
const STRAIGHT: f32 = 1.5;
const CURLED: f32 = 1.25;
/// With two fingers up, how much shorter the other two have to reach than they do:
/// folded fingers are not always folded as far as `CURLED`, but are always shorter.
const SHORTER: f32 = 0.3;
/// How long a pinch held still, in seconds, is a click of the right button.
const LONG_PINCH: f64 = 0.5;
/// How far the pointer goes, in screens, for the hand going across the camera's
/// whole view slowly; and how much more for each view a second faster, up to a most.
const GAIN: f32 = 1.8;
const SPEEDING: f32 = 1.5;
const MOST_GAIN: f32 = 6.0;
/// How near the edge of the picture, as a fraction of it, the palm is too near to be
/// followed well.
const EDGE: f32 = 0.06;
/// After a button is pressed, the pointer holds still until the hand has plainly
/// moved, as a fraction of the screen, so that a click lands where it was aimed: a
/// hand held still still drifts by a few pixels a second.
const CLICK_SLOP: f32 = 0.035;
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
    Two,
    Fist,
    Other,
}

/// The fingers, each as the joints at its base, its middle and its tip.
const FINGERS: [(usize, usize); 4] = [(5, 8), (9, 12), (13, 16), (17, 20)];

/// What the hand is doing. `before` is the pose it was in, for a pinch to let go
/// only once the thumb is well clear.
pub fn pose(hand: &Hand, before: Pose) -> Pose {
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
    let limit = if before == Pose::Pinch { UNPINCH } else { PINCH };
    if joints[4].distance(joints[8]) / palm < limit {
        return Pose::Pinch;
    }
    if straight(0) && straight(1) && reach[2].max(reach[3]) < reach[0].min(reach[1]) - SHORTER {
        Pose::Two
    } else if (0..4).filter(|finger| straight(*finger)).count() >= 3 {
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

/// A pinch, from when it began: waiting to see whether it is a click, a drag or a
/// right click; dragging with the left button down; or done, the right button clicked.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pinching {
    Waiting { since: f64, moved: (f32, f32) },
    Dragging,
    Done,
}

/// Follows the hand from frame to frame and says what it does.
pub struct Gestures {
    /// The pose in force, and one that is turning up but has not lasted yet.
    pose: Pose,
    coming: (Pose, u32),
    seen: bool,
    x: OneEuro,
    y: OneEuro,
    /// Where the hand was last, smoothed, as fractions of the view, and when; none
    /// while it is lifted.
    last: Option<((f32, f32), f64)>,
    /// A pinch under way.
    pinch: Option<Pinching>,
    /// Until when the pointer holds still after a button is let go.
    hold_until: f64,
    /// Two fingers up: where the hand was last, unsmoothed, and the recent sideways
    /// positions with their times, for sweeps.
    scroll_from: Option<f32>,
    sweep: Vec<(f64, f32)>,
    swept: bool,
}

impl Default for Gestures {
    fn default() -> Gestures {
        Gestures {
            pose: Pose::Other,
            coming: (Pose::Other, 0),
            seen: false,
            x: OneEuro::new(1.0, 4.0),
            y: OneEuro::new(1.0, 4.0),
            last: None,
            pinch: None,
            hold_until: 0.0,
            scroll_from: None,
            sweep: Vec::new(),
            swept: false,
        }
    }
}

impl Gestures {
    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// Takes in one frame: the hand in a `width` by `height` picture, if one was
    /// found, at `time` seconds.
    pub fn update(&mut self, hand: Option<&Hand>, width: f32, height: f32, time: f64) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(hand) = hand else {
            // A drag ends; a pinch not yet a click is not one.
            if self.pinch.take() == Some(Pinching::Dragging) {
                events.push(Event::Release(Button::Left));
            }
            if std::mem::take(&mut self.seen) {
                events.push(Event::Gone);
            }
            self.pose = Pose::Other;
            self.coming = (Pose::Other, 0);
            self.x.reset();
            self.y.reset();
            self.last = None;
            self.scroll_from = None;
            self.sweep.clear();
            return events;
        };
        self.seen = true;

        // A new pose counts once it has lasted.
        let seen = pose(hand, self.pose);
        self.coming = if seen == self.coming.0 { (seen, self.coming.1 + 1) } else { (seen, 1) };
        let before = self.pose;
        if seen != self.pose && self.coming.1 >= STEADY {
            self.pose = seen;
        }
        let pose = self.pose;

        let middle = palm_middle(hand);
        let raw = in_view(middle, width, height);
        let smooth = (self.x.filter(raw.0 as f64, time) as f32, self.y.filter(raw.1 as f64, time) as f32);

        if pose != before {
            // Leaving a pose.
            match before {
                Pose::Pinch => {
                    match self.pinch.take() {
                        Some(Pinching::Waiting { .. }) => events.extend([Event::Press(Button::Left), Event::Release(Button::Left)]),
                        Some(Pinching::Dragging) => events.push(Event::Release(Button::Left)),
                        Some(Pinching::Done) | None => {}
                    }
                    self.hold_until = time + RELEASE_HOLD;
                }
                Pose::Fist => events.push(Event::Wake),
                Pose::Two => {
                    self.scroll_from = None;
                    self.sweep.clear();
                    self.swept = false;
                }
                _ => {}
            }
            // Coming into one.
            match pose {
                Pose::Pinch => self.pinch = Some(Pinching::Waiting { since: time, moved: (0.0, 0.0) }),
                Pose::Fist => events.push(Event::Rest),
                _ => {}
            }
        }

        match pose {
            Pose::Fist => {}
            Pose::Two => {
                // Up and down scrolls, by the smoothed hand and only once it has gone
                // further than it shakes; a quick sweep sideways, once, goes on.
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
                let mut motion = match self.last {
                    Some((from, then)) => pointer_motion((smooth.0 - from.0, smooth.1 - from.1), time - then),
                    None => (0.0, 0.0),
                };
                // Held still just after a release, and while the hand is changing
                // pose, as it does when fingers come together to pinch.
                if seen != pose || time < self.hold_until {
                    motion = (0.0, 0.0);
                }
                // A pinch holds the pointer still: moved plainly, it drags, from where
                // it began; held long enough, it clicks the right button.
                match self.pinch {
                    Some(Pinching::Waiting { since, moved }) => {
                        let moved = (moved.0 + motion.0, moved.1 + motion.1);
                        motion = (0.0, 0.0);
                        if moved.0.hypot(moved.1) >= CLICK_SLOP {
                            events.push(Event::Press(Button::Left));
                            motion = moved;
                            self.pinch = Some(Pinching::Dragging);
                        } else if time - since >= LONG_PINCH {
                            events.extend([Event::Press(Button::Right), Event::Release(Button::Right)]);
                            self.pinch = Some(Pinching::Done);
                        } else {
                            self.pinch = Some(Pinching::Waiting { since, moved });
                        }
                    }
                    Some(Pinching::Done) => motion = (0.0, 0.0),
                    Some(Pinching::Dragging) | None => {}
                }
                if motion != (0.0, 0.0) {
                    events.push(Event::Motion { dx: motion.0, dy: motion.1 });
                }
                self.last = Some((smooth, time));
            }
            // Something in between: the pointer stays.
            Pose::Other => {}
        }
        // Lifted, or the hand off doing something else: it starts again from where
        // it is next time.
        if !matches!(pose, Pose::Open | Pose::Pinch) {
            self.last = None;
            self.x.reset();
            self.y.reset();
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
        // A third of a palm out to the side: not yet a pinch, but not yet let go of one.
        nearly.joints[4].x -= 33.0;
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

    #[test]
    fn a_quick_pinch_clicks_where_the_pointer_was() {
        let mut gestures = Gestures::default();
        let moving: Vec<Option<Hand>> = (0..5).map(|step| Some(hand_at(Point::new(320.0 + step as f32 * 3.0, 400.0), OPEN, None))).collect();
        let events = run(&mut gestures, &moving, 0.0);
        assert!(matches!(events.first(), Some(Event::Motion { .. })), "{events:?}");
        let open = vec![Some(hand_at(Point::new(332.0, 400.0), OPEN, None)); 5];
        run(&mut gestures, &open, 0.5);
        // Pinching, the hand shifts a little: nothing yet.
        let pinched = vec![Some(hand_at(Point::new(335.0, 402.0), OPEN, Some(8))); 4];
        assert_eq!(run(&mut gestures, &pinched, 1.0), []);
        // Let go soon: a click, and the pointer still where it was.
        let events = run(&mut gestures, &open, 1.2);
        assert_eq!(&events[..2], [Event::Press(Button::Left), Event::Release(Button::Left)]);
        assert!(!events[2..].iter().any(|event| matches!(event, Event::Press(_))));
    }

    #[test]
    fn a_pinch_moved_far_drags() {
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 5], 0.0);
        let mut frames = vec![Some(hand(OPEN, Some(8))); 3];
        frames.extend((1..=10).map(|step| Some(hand_at(Point::new(320.0 - step as f32 * 15.0, 400.0), OPEN, Some(8)))));
        let events = run(&mut gestures, &frames, 1.0);
        assert_eq!(events[0], Event::Press(Button::Left));
        assert!(matches!(events[1], Event::Motion { .. }), "the drag starts from where the pinch did: {events:?}");
        assert!(!events.contains(&Event::Release(Button::Left)));
        let events = run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 2.0);
        assert_eq!(events.first(), Some(&Event::Release(Button::Left)));
    }

    #[test]
    fn a_pinch_held_still_is_the_right_button() {
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 0.0);
        let events = run(&mut gestures, &vec![Some(hand(OPEN, Some(8))); 30], 1.0);
        assert_eq!(events, [Event::Press(Button::Right), Event::Release(Button::Right)]);
        // Letting go afterwards clicks nothing more.
        let events = run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 2.0);
        assert!(!events.iter().any(|event| matches!(event, Event::Press(_) | Event::Release(_))), "{events:?}");
    }

    #[test]
    fn a_lost_hand_lets_go_of_a_drag() {
        let mut gestures = Gestures::default();
        run(&mut gestures, &vec![Some(hand(OPEN, None)); 3], 0.0);
        let mut frames = vec![Some(hand(OPEN, Some(8))); 3];
        frames.extend((1..=5).map(|step| Some(hand_at(Point::new(320.0 - step as f32 * 15.0, 400.0), OPEN, Some(8)))));
        run(&mut gestures, &frames, 1.0);
        assert_eq!(gestures.update(None, 640.0, 480.0, 2.0), [Event::Release(Button::Left), Event::Gone]);
        assert!(gestures.update(None, 640.0, 480.0, 2.1).is_empty());
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
