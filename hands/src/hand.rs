//! A hand's 21 joints: MediaPipe's hand landmark model, run on a square of the camera
//! picture turned to stand the hand upright. Once a hand is found, the next square is
//! worked out from its joints, so the palm detector need not run again while the hand
//! stays in view.

use crate::geometry::{Picture, Point, Square, sample, upright};

/// The model's input, square.
pub const SIZE: usize = 224;
/// How sure the model has to be that a hand is there, to go on following it.
pub const PRESENT: f32 = 0.5;

/// The joints, in the model's order.
pub const WRIST: usize = 0;
pub const THUMB_TIP: usize = 4;
pub const INDEX_BASE: usize = 5;
pub const INDEX_TIP: usize = 8;

/// A hand found in the camera picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Hand {
    /// The 21 joints in camera picture pixels.
    pub joints: [Point; 21],
    /// How far each joint is towards the camera from the wrist, in the same pixels.
    pub depth: [f32; 21],
    /// How sure the model is that this is a hand.
    pub presence: f32,
    /// From 0, a left hand, to 1, a right one, as seen in a mirror.
    pub right: f32,
}

/// The model's input for one square of the picture.
pub fn input(picture: &Picture, square: &Square, into: &mut Vec<f32>) {
    sample(picture, square, SIZE, into);
}

/// Reads the model's answer: 63 numbers, each joint's x, y and depth in input pixels.
pub fn decode(square: &Square, joints: &[f32], presence: f32, right: f32) -> Option<Hand> {
    if joints.len() < 63 {
        return None;
    }
    let scale = square.side / SIZE as f32;
    Some(Hand {
        joints: std::array::from_fn(|joint| {
            let (x, y) = (joints[joint * 3], joints[joint * 3 + 1]);
            square.to_picture(x / SIZE as f32 - 0.5, y / SIZE as f32 - 0.5)
        }),
        depth: std::array::from_fn(|joint| joints[joint * 3 + 2] * scale),
        presence,
        right,
    })
}

impl Hand {
    /// The square to look in on the next frame, as MediaPipe works it out: turned
    /// from the wrist towards the middle of the fingers, around the palm and the
    /// lower fingers, moved a tenth towards the fingers and made twice as large.
    pub fn next_square(&self) -> Square {
        let joints = &self.joints;
        let mean = |a: Point, b: Point| Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let rotation = upright(joints[WRIST], mean(mean(joints[6], joints[14]), joints[10]));
        let frame = Square { center: joints[WRIST], side: 1.0, rotation };
        let (mut low, mut high) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for joint in [0, 1, 2, 3, 5, 6, 9, 10, 13, 14, 17, 18] {
            let (across, down) = frame.from_picture(joints[joint]);
            low = (low.0.min(across), low.1.min(down));
            high = (high.0.max(across), high.1.max(down));
        }
        let (width, height) = (high.0 - low.0, high.1 - low.1);
        let center = frame.to_picture((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0 - 0.1 * height);
        Square { center, side: width.max(height) * 2.0, rotation }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joints_come_back_to_the_picture_through_the_square() {
        // A square turned a quarter: the top of the model's input lies to the right.
        let square = Square { center: Point::new(300.0, 200.0), side: 224.0, rotation: std::f32::consts::FRAC_PI_2 };
        let mut joints = vec![112.0; 63];
        joints[1] = 0.0; // The wrist at the top middle of the input.
        joints[2] = 5.0;
        let hand = decode(&square, &joints, 0.9, 1.0).unwrap();
        assert!(hand.joints[WRIST].distance(Point::new(412.0, 200.0)) < 1e-3, "{:?}", hand.joints[WRIST]);
        assert!(hand.joints[1].distance(Point::new(300.0, 200.0)) < 1e-3);
        assert_eq!(hand.depth[0], 5.0);
        assert!(decode(&square, &joints[..10], 0.9, 1.0).is_none());
    }

    #[test]
    fn follows_an_upright_hand_from_its_joints() {
        // A hand 100 pixels wide and 100 from the wrist to the base of the fingers.
        let mut joints = [Point::new(0.0, 0.0); 21];
        joints[WRIST] = Point::new(200.0, 300.0);
        for (joint, x) in [(1, 150.0), (2, 150.0), (3, 150.0), (5, 160.0), (9, 200.0), (13, 240.0), (17, 250.0)] {
            joints[joint] = Point::new(x, 200.0);
        }
        for (joint, x) in [(6, 160.0), (10, 200.0), (14, 240.0), (18, 250.0)] {
            joints[joint] = Point::new(x, 180.0);
        }
        let hand = Hand { joints, depth: [0.0; 21], presence: 1.0, right: 1.0 };
        let square = hand.next_square();
        assert!(square.rotation.abs() < 1e-6);
        // Around x 150 to 250 and y 180 to 300, moved up a tenth of its 120 height.
        assert!(square.center.distance(Point::new(200.0, 228.0)) < 1e-3, "{square:?}");
        assert!((square.side - 240.0).abs() < 1e-3);
    }
}
