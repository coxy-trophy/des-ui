//! The DesOS bird, drawn from the one master drawing in `branding/bird.svg`.
//!
//! The drawing is a single path of moves, lines and cubic curves, filled even-odd.
//! It is flattened into polygons once and filled here at whatever size is asked for,
//! as coverage: for the glass on the lock and login screens, and as a texture for the
//! status strip.
use eframe::egui::{self, pos2, Pos2, Rect};
use std::sync::OnceLock;

const SVG: &str = include_str!("../../branding/bird.svg");
/// The part of the box the bird takes, in box units, so it can be placed by its ink.
pub const INK: Rect = Rect { min: Pos2 { x: 68.0, y: 0.0 }, max: Pos2 { x: 932.0, y: 1000.0 } };
/// Points each curve is cut into; the drawing's curves are short.
const STEPS: usize = 6;

/// Width over height of the bird's ink.
pub fn aspect() -> f32 { INK.width() / INK.height() }

/// The bird as closed polygons, in box units.
fn polygons() -> &'static [Vec<Pos2>] {
    static POLYGONS: OnceLock<Vec<Vec<Pos2>>> = OnceLock::new();
    POLYGONS.get_or_init(|| parse(SVG))
}

fn parse(svg: &str) -> Vec<Vec<Pos2>> {
    let d = svg.split(" d=\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or_default();
    let mut polygons = Vec::new();
    let mut current: Vec<Pos2> = Vec::new();
    let mut op = ' ';
    let mut numbers: Vec<f32> = Vec::new();
    let flush = |op: char, numbers: &mut Vec<f32>, current: &mut Vec<Pos2>, polygons: &mut Vec<Vec<Pos2>>| {
        let p: Vec<Pos2> = numbers.chunks_exact(2).map(|v| pos2(v[0], v[1])).collect();
        match op {
            'M' => {
                if current.len() > 2 { polygons.push(std::mem::take(current)); }
                current.clear();
                current.extend(p);
            }
            'L' => current.extend(p),
            'C' => {
                for c in p.chunks_exact(3) {
                    let a = current.last().copied().unwrap_or(c[0]);
                    for step in 1..=STEPS {
                        let t = step as f32 / STEPS as f32;
                        let u = 1.0 - t;
                        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                        current.push(pos2(
                            w[0] * a.x + w[1] * c[0].x + w[2] * c[1].x + w[3] * c[2].x,
                            w[0] * a.y + w[1] * c[0].y + w[2] * c[1].y + w[3] * c[2].y,
                        ));
                    }
                }
            }
            'Z' => if current.len() > 2 { polygons.push(std::mem::take(current)); },
            _ => {}
        }
        numbers.clear();
    };
    let mut number = String::new();
    for ch in d.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || ch == '.' || (ch == '-' && number.is_empty()) {
            number.push(ch);
            continue;
        }
        if !number.is_empty() {
            if let Ok(value) = number.parse() { numbers.push(value); }
            number.clear();
        }
        if ch == '-' { number.push(ch); continue; }
        if matches!(ch, 'M' | 'L' | 'C' | 'Z') {
            flush(op, &mut numbers, &mut current, &mut polygons);
            op = ch;
        }
    }
    flush(op, &mut numbers, &mut current, &mut polygons);
    if current.len() > 2 { polygons.push(current); }
    polygons
}

/// Where the bird's box goes so its ink fills `ink` (in pixels), keeping its shape.
fn place(ink: Rect) -> (f32, egui::Vec2) {
    let k = (ink.width() / INK.width()).min(ink.height() / INK.height());
    let offset = ink.center().to_vec2() - INK.center().to_vec2() * k;
    (k, offset)
}

/// Adds the bird's coverage, its ink filling `ink` (pixels), to a `w` x `h` field,
/// keeping the larger where it overlaps. Four rows of samples a pixel; the spans
/// between crossings are counted exactly across each row.
pub fn cover(field: &mut [f32], [w, h]: [usize; 2], ink: Rect) {
    const ROWS: usize = 4;
    let (k, offset) = place(ink);
    let edges: Vec<(Pos2, Pos2)> = polygons()
        .iter()
        .flat_map(|polygon| (0..polygon.len()).map(move |i| (polygon[i], polygon[(i + 1) % polygon.len()])))
        .map(|(a, b)| ((a.to_vec2() * k + offset).to_pos2(), (b.to_vec2() * k + offset).to_pos2()))
        .collect();
    let y0 = (ink.min.y.floor().max(0.0) as usize).min(h);
    let y1 = (ink.max.y.ceil().max(0.0) as usize).min(h);
    let x0 = (ink.min.x.floor().max(0.0) as usize).min(w);
    let x1 = (ink.max.x.ceil().max(0.0) as usize).min(w);
    if x1 <= x0 { return; }
    let mut row = vec![0.0_f32; x1 - x0];
    let mut crossings = Vec::new();
    for y in y0..y1 {
        row.iter_mut().for_each(|v| *v = 0.0);
        for sample in 0..ROWS {
            let sy = y as f32 + (sample as f32 + 0.5) / ROWS as f32;
            crossings.clear();
            for (a, b) in &edges {
                if (a.y <= sy) != (b.y <= sy) {
                    crossings.push(a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            crossings.sort_by(|a, b| a.total_cmp(b));
            for span in crossings.chunks_exact(2) {
                let (left, right) = (span[0].max(x0 as f32), span[1].min(x1 as f32));
                if right <= left { continue; }
                let (first, last) = (left.floor() as usize, (right.ceil() as usize).min(x1));
                for x in first..last {
                    let inside = (right.min(x as f32 + 1.0) - left.max(x as f32)).max(0.0);
                    row[x - x0] += inside / ROWS as f32;
                }
            }
        }
        for (i, value) in row.iter().enumerate() {
            let at = y * w + x0 + i;
            field[at] = field[at].max(value.min(1.0));
        }
    }
}

/// The bird in white, `height` pixels tall, as a picture whose alpha is its coverage.
pub fn image(height: usize) -> egui::ColorImage {
    let width = ((height as f32 * aspect()).ceil() as usize).max(1);
    let mut field = vec![0.0; width * height];
    cover(&mut field, [width, height], Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32)));
    egui::ColorImage { size: [width, height], pixels: field.iter().map(|c| egui::Color32::from_white_alpha((c * 255.0).round() as u8)).collect() }
}

/// The bird as a texture of the given pixel height, made once per height and kept.
pub fn texture(ctx: &egui::Context, height: usize) -> egui::TextureHandle {
    let id = egui::Id::new(("des-bird", height));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) { return texture; }
    let texture = ctx.load_texture(format!("des-bird-{height}"), image(height), egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_master_drawing_is_five_pieces_with_the_eye_cut_out() {
        // The upper wing, the body and head, the lower wing, the eye and the tail.
        assert_eq!(polygons().len(), 5);
        let picture = image(200);
        assert_eq!(picture.size, [173, 200]);
        let alpha = |x: usize, y: usize| picture.pixels[y * picture.size[0] + x].a();
        // Empty where the tail curves round, solid in the body, clear in the eye.
        assert_eq!(alpha(130, 160), 0);
        let solid = picture.pixels.iter().filter(|p| p.a() == 255).count();
        assert!(solid > 2000 && solid < 15000, "{solid}");
        let eye = polygons().iter().min_by(|a, b| a.len().cmp(&b.len())).unwrap();
        let centre = eye.iter().fold(egui::Vec2::ZERO, |sum, p| sum + p.to_vec2()) / eye.len() as f32;
        let (k, offset) = place(Rect::from_min_size(Pos2::ZERO, egui::vec2(173.0, 200.0)));
        let at = centre * k + offset;
        assert!(alpha(at.x as usize, at.y as usize) < 40, "the eye is open");
    }
}
