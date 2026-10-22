//! Pictures and the squares cut from them. Both models look at a square picture of a
//! fixed size: the whole camera picture, letterboxed, for finding a palm, and a square
//! turned to stand the hand upright for finding its joints. Each is sampled straight
//! from the camera picture through one affine mapping, so nothing is cropped, padded,
//! turned and resized in separate steps.

/// A camera picture, eight-bit RGB, row by row.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    pub fn distance(self, other: Point) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// A square of the camera picture, turned by `rotation` radians clockwise, as the
/// screen's y runs downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Square {
    pub center: Point,
    pub side: f32,
    pub rotation: f32,
}

impl Square {
    /// Where a point given within the square, from -0.5 to 0.5 across and down it,
    /// lies in the camera picture.
    pub fn to_picture(&self, across: f32, down: f32) -> Point {
        let (sin, cos) = self.rotation.sin_cos();
        let (u, v) = (across * self.side, down * self.side);
        Point::new(self.center.x + u * cos - v * sin, self.center.y + u * sin + v * cos)
    }

    /// The other way: where a point of the camera picture lies within the square.
    pub fn from_picture(&self, point: Point) -> (f32, f32) {
        let (sin, cos) = self.rotation.sin_cos();
        let (dx, dy) = (point.x - self.center.x, point.y - self.center.y);
        ((dx * cos + dy * sin) / self.side, (-dx * sin + dy * cos) / self.side)
    }
}

/// The turn that stands a hand upright, from its wrist to the base of its middle
/// finger: none when that points straight up the picture.
pub fn upright(wrist: Point, middle: Point) -> f32 {
    let radians = std::f32::consts::FRAC_PI_2 - (-(middle.y - wrist.y)).atan2(middle.x - wrist.x);
    // Brought within a half turn either way.
    let turn = std::f32::consts::TAU;
    radians - turn * ((radians + std::f32::consts::PI) / turn).floor()
}

/// Samples `square` of `picture` into a `size` by `size` model input: RGB from 0 to 1,
/// row by row. What lies outside the picture is black.
pub fn sample(picture: &Picture, square: &Square, size: usize, into: &mut Vec<f32>) {
    into.clear();
    into.reserve(size * size * 3);
    for row in 0..size {
        for column in 0..size {
            let across = (column as f32 + 0.5) / size as f32 - 0.5;
            let down = (row as f32 + 0.5) / size as f32 - 0.5;
            let at = square.to_picture(across, down);
            into.extend(bilinear(picture, at.x - 0.5, at.y - 0.5));
        }
    }
}

/// The colour at a point between pixel centres, each channel from 0 to 1.
fn bilinear(picture: &Picture, x: f32, y: f32) -> [f32; 3] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let pixel = |px: f32, py: f32| -> [f32; 3] {
        if px < 0.0 || py < 0.0 || px >= picture.width as f32 || py >= picture.height as f32 {
            return [0.0; 3];
        }
        let at = (py as usize * picture.width + px as usize) * 3;
        [picture.rgb[at] as f32, picture.rgb[at + 1] as f32, picture.rgb[at + 2] as f32]
    };
    let (a, b, c, d) = (pixel(x0, y0), pixel(x0 + 1.0, y0), pixel(x0, y0 + 1.0), pixel(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        (top + (bottom - top) * fy) / 255.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn a_hand_pointing_up_needs_no_turn() {
        let wrist = Point::new(100.0, 200.0);
        assert!(upright(wrist, Point::new(100.0, 100.0)).abs() < 1e-6);
        // Pointing right, it is turned a quarter clockwise; pointing down, a half.
        assert!((upright(wrist, Point::new(200.0, 200.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((upright(wrist, Point::new(100.0, 300.0)).abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn the_top_of_a_turned_square_is_where_the_fingers_point() {
        // A hand pointing right: the top middle of its square is to the right of centre.
        let square = Square { center: Point::new(50.0, 50.0), side: 20.0, rotation: upright(Point::new(0.0, 50.0), Point::new(10.0, 50.0)) };
        assert!(close(square.to_picture(0.0, -0.5), Point::new(60.0, 50.0)));
        let point = Point::new(43.0, 61.0);
        let (across, down) = square.from_picture(point);
        assert!(close(square.to_picture(across, down), point));
    }

    #[test]
    fn samples_the_picture_and_black_beyond_it() {
        // Two pixels: red, then white.
        let picture = Picture { width: 2, height: 1, rgb: vec![255, 0, 0, 255, 255, 255] };
        let mut out = Vec::new();
        sample(&picture, &Square { center: Point::new(1.0, 1.0), side: 2.0, rotation: 0.0 }, 2, &mut out);
        // The top row is the picture itself; the second lies below it.
        assert_eq!(&out[0..6], &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        assert!(out[6..].iter().all(|value| *value == 0.0));
        let mut far = Vec::new();
        sample(&picture, &Square { center: Point::new(-50.0, -50.0), side: 2.0, rotation: 0.3 }, 2, &mut far);
        assert!(far.iter().all(|value| *value == 0.0));
    }
}
