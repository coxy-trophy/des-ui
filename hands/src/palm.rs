//! Finding palms: MediaPipe's palm detector, run over the whole camera picture when no
//! hand is being followed. It answers for 2016 anchors, fixed points spread over the
//! picture, each with a score, a box and seven points of the palm.

use crate::geometry::{Picture, Point, Square, sample, upright};

/// The detector's input, square.
pub const SIZE: usize = 192;
/// How sure it has to be.
const SCORE: f32 = 0.5;
/// How much two boxes may overlap and both be palms.
const OVERLAP: f32 = 0.3;

/// A palm found in the camera picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Palm {
    pub score: f32,
    /// Its box: top left and bottom right.
    pub min: Point,
    pub max: Point,
    /// The wrist, the bases of the fingers and of the thumb.
    pub points: [Point; 7],
}

impl Palm {
    /// The square in which to look for the whole hand, turned to stand it upright:
    /// moved half the palm's height towards the fingers and made two and a half
    /// times as large, as MediaPipe does.
    pub fn hand_square(&self) -> Square {
        let rotation = upright(self.points[0], self.points[2]);
        let (width, height) = (self.max.x - self.min.x, self.max.y - self.min.y);
        let center = Point::new((self.min.x + self.max.x) / 2.0, (self.min.y + self.max.y) / 2.0);
        let square = Square { center, side: width.max(height), rotation };
        let center = square.to_picture(0.0, -0.5 * height / square.side);
        Square { center, side: square.side * 2.6, rotation }
    }
}

/// The anchors, in the order the detector answers for them: on a 24 by 24 grid, two
/// to a cell, then on a 12 by 12 grid, six to a cell; row by row, from 0 to 1.
pub fn anchors() -> Vec<Point> {
    let mut anchors = Vec::with_capacity(2016);
    for (grid, each) in [(24, 2), (12, 6)] {
        for row in 0..grid {
            for column in 0..grid {
                let at = Point::new((column as f32 + 0.5) / grid as f32, (row as f32 + 0.5) / grid as f32);
                anchors.extend(std::iter::repeat_n(at, each));
            }
        }
    }
    anchors
}

/// The square that holds the whole picture, its longer side across, letterboxed.
pub fn whole(picture: &Picture) -> Square {
    let side = picture.width.max(picture.height) as f32;
    Square { center: Point::new(picture.width as f32 / 2.0, picture.height as f32 / 2.0), side, rotation: 0.0 }
}

/// The detector's input for a picture.
pub fn input(picture: &Picture, into: &mut Vec<f32>) {
    sample(picture, &whole(picture), SIZE, into);
}

/// Reads the detector's answer: for each anchor, 18 numbers (the box's centre and size,
/// then the seven points, in input pixels from the anchor) and one score before the
/// sigmoid. Returns the palms found, surest first, in camera picture pixels.
pub fn decode(picture: &Picture, anchors: &[Point], boxes: &[f32], scores: &[f32]) -> Vec<Palm> {
    let view = whole(picture);
    // From the detector's input to the picture.
    let place = |x: f32, y: f32| view.to_picture(x / SIZE as f32 - 0.5, y / SIZE as f32 - 0.5);
    let mut found: Vec<Palm> = Vec::new();
    for (index, anchor) in anchors.iter().enumerate() {
        let (Some(row), Some(logit)) = (boxes.get(index * 18..index * 18 + 18), scores.get(index)) else {
            break;
        };
        let score = 1.0 / (1.0 + (-logit.clamp(-80.0, 80.0)).exp());
        if score < SCORE {
            continue;
        }
        let (ax, ay) = (anchor.x * SIZE as f32, anchor.y * SIZE as f32);
        let (cx, cy, w, h) = (ax + row[0], ay + row[1], row[2], row[3]);
        let points = std::array::from_fn(|point| place(ax + row[4 + point * 2], ay + row[5 + point * 2]));
        found.push(Palm { score, min: place(cx - w / 2.0, cy - h / 2.0), max: place(cx + w / 2.0, cy + h / 2.0), points });
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    // Of boxes that overlap, the surest stands for them all.
    let mut kept: Vec<Palm> = Vec::new();
    for palm in found {
        if kept.iter().all(|other| overlap(&palm, other) <= OVERLAP) {
            kept.push(palm);
        }
    }
    kept
}

/// How much two boxes overlap: their intersection over their union.
fn overlap(a: &Palm, b: &Palm) -> f32 {
    let width = (a.max.x.min(b.max.x) - a.min.x.max(b.min.x)).max(0.0);
    let height = (a.max.y.min(b.max.y) - a.min.y.max(b.min.y)).max(0.0);
    let area = |palm: &Palm| (palm.max.x - palm.min.x) * (palm.max.y - palm.min.y);
    let shared = width * height;
    let union = area(a) + area(b) - shared;
    if union <= 0.0 { 0.0 } else { shared / union }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_laid_out_as_the_detector_expects() {
        let anchors = anchors();
        assert_eq!(anchors.len(), 2016);
        // The first two share the first cell; the next is one cell to the right.
        assert_eq!(anchors[0], Point::new(0.5 / 24.0, 0.5 / 24.0));
        assert_eq!(anchors[1], anchors[0]);
        assert_eq!(anchors[2], Point::new(1.5 / 24.0, 0.5 / 24.0));
        assert_eq!(anchors[1152], Point::new(0.5 / 12.0, 0.5 / 12.0));
        assert_eq!(anchors[2015], Point::new(11.5 / 12.0, 11.5 / 12.0));
    }

    #[test]
    fn decodes_a_palm_into_the_picture_and_drops_its_echoes() {
        // 640 by 480: the detector sees it 0.3 times as large, 24 pixels down.
        let picture = Picture { width: 640, height: 480, rgb: Vec::new() };
        let anchors = vec![Point::new(0.5, 0.5), Point::new(0.5, 0.5), Point::new(0.1, 0.1)];
        let mut boxes = vec![0.0; 18 * 3];
        boxes[2] = 48.0;
        boxes[3] = 48.0;
        // The second, a shade less sure and in the same place, is the same palm.
        boxes[20] = 48.0;
        boxes[21] = 50.0;
        let scores = [3.0, 2.0, -3.0];
        let palms = decode(&picture, &anchors, &boxes, &scores);
        assert_eq!(palms.len(), 1);
        let palm = &palms[0];
        assert!((palm.score - 0.9526).abs() < 1e-3);
        // Centred in the picture, 160 pixels a side.
        assert!(palm.min.distance(Point::new(240.0, 160.0)) < 1e-3, "{palm:?}");
        assert!(palm.max.distance(Point::new(400.0, 320.0)) < 1e-3);
        assert!(palm.points[0].distance(Point::new(320.0, 240.0)) < 1e-3);
    }

    #[test]
    fn looks_for_the_hand_beyond_the_palm_towards_the_fingers() {
        let palm = Palm {
            score: 1.0,
            min: Point::new(100.0, 100.0),
            max: Point::new(200.0, 200.0),
            // The wrist below, the middle finger above: upright.
            points: [Point::new(150.0, 200.0), Point::new(0.0, 0.0), Point::new(150.0, 100.0), Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, 0.0)],
        };
        let square = palm.hand_square();
        assert!(square.rotation.abs() < 1e-6);
        assert!(square.center.distance(Point::new(150.0, 100.0)) < 1e-3);
        assert!((square.side - 260.0).abs() < 1e-3);
    }
}
