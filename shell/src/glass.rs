//! Coloured liquid glass, drawn by the GPU over the wallpaper.
//!
//! Each glass shape (the clock's numerals, the rounded tags and fields) becomes an
//! exact distance-from-the-edge field on the CPU, shaped into a squircle bevel: steep
//! at the rim, flat inside. A fragment shader then follows light through it, per
//! colour channel: refraction into the curved top by Snell's law with a Cauchy index
//! n(l) = A + B / l^2, absorption by the glass's own colour along the path actually
//! taken (Beer-Lambert, so thin edges stay light and thick strokes go deep), a second
//! refraction out of the flat back (or total internal reflection), on through a gap to
//! the wallpaper. The exact unpolarised Fresnel equations share each surface between
//! reflected and transmitted light; the reflected part sees a blurred, mirrored
//! wallpaper, and a thin interference film on the surface adds iridescence that
//! changes with the angle around the curves. A highlight and a rim lit from one side
//! stand for the light source.
//!
//! It draws only where OpenGL 3 or GLES 3 is available; anywhere else it draws
//! nothing and what was painted under it shows instead.
use eframe::egui::{self, Color32, Pos2, Rect, pos2};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

/// A piece of glass.
#[derive(Clone)]
pub enum Shape {
    /// A rounded rectangle, ends fully rounded.
    Pill(Rect),
    /// Text, laid out, with the top-left corner of its galley.
    Text(Arc<egui::Galley>, Pos2),
}

/// The wallpaper the glass stands over: the sharp and the blurred texture, where it
/// is drawn on the screen, and the part of each texture that is shown.
#[derive(Clone, Copy)]
pub struct Backdrop {
    pub sharp: egui::TextureId,
    pub soft: egui::TextureId,
    pub dest: Rect,
    pub uv: Rect,
}

/// The look, tuned in the glass lab. Lengths are for a screen 900 points tall and
/// scale with the real height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub bevel: f32,
    pub depth: f32,
    pub curve: f32,
    pub index: f32,
    pub dispersion: f32,
    pub thickness: f32,
    pub gap: f32,
    pub absorption: f32,
    pub colour_depth: f32,
    pub iridescence: f32,
    pub film_nm: f32,
    pub frost: f32,
    pub tint: f32,
    pub luminosity: f32,
    pub edge: f32,
    pub edge_width: f32,
    pub reflection: f32,
    pub highlight: f32,
    pub highlight_size: f32,
    pub light_degrees: f32,
    pub shadow: f32,
    pub scrim: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            bevel: 9.0, depth: 1.6, curve: 3.0,
            index: 1.52, dispersion: 0.012, thickness: 26.0, gap: 40.0, absorption: 0.004,
            colour_depth: 0.035, iridescence: 0.3, film_nm: 420.0,
            frost: 0.0, tint: 0.04, luminosity: 0.65,
            edge: 1.0, edge_width: 1.6, reflection: 1.4, highlight: 0.7, highlight_size: 48.0, light_degrees: 225.0,
            shadow: 0.25, scrim: 0.35,
        }
    }
}

/// What the CPU worked out for one set of shapes at one size: per pixel the slope,
/// height and distance from the edge, and the coverage and its blur for the shadow.
struct Fields {
    size: [usize; 2],
    surface: Vec<f32>,
    cover: Vec<f32>,
}

#[derive(Default)]
struct Gpu {
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    surface: Option<glow::Texture>,
    cover: Option<glow::Texture>,
    uploaded: u64,
    /// The size of the fields on the GPU; the CPU copy is dropped once uploaded.
    size: [usize; 2],
    failed: bool,
    srgb: bool,
}

struct Shared {
    fields: Option<Arc<Fields>>,
    generation: u64,
    gpu: Gpu,
}

/// The glass, kept between frames so the fields are worked out only when the
/// shapes or the screen change.
pub struct Glass {
    shared: Arc<Mutex<Shared>>,
    key: Option<String>,
    /// Fields being worked out away from the UI thread (a few hundred milliseconds
    /// on a large screen); the glass already on the GPU shows meanwhile.
    pending: Option<std::sync::mpsc::Receiver<Fields>>,
}

impl Default for Glass {
    fn default() -> Self {
        Glass { shared: Arc::new(Mutex::new(Shared { fields: None, generation: 0, gpu: Gpu::default() })), key: None, pending: None }
    }
}

impl Glass {
    /// False once the GPU turned out unable to draw glass; then the caller paints
    /// its plain version instead.
    pub fn available(&self) -> bool {
        !self.shared.lock().map(|s| s.gpu.failed).unwrap_or(true)
    }

    /// True once there is glass to draw: the first fields are worked out.
    pub fn ready(&self) -> bool {
        self.shared.lock().map(|s| s.generation > 0).unwrap_or(false)
    }

    /// The glass over all of `screen`, as a shape to paint: the wallpaper, darkened
    /// a little at the top and bottom, with `shapes` as glass of colour `tone` on it.
    /// It covers the whole screen, so what should show on the glass is painted after.
    #[allow(clippy::too_many_arguments)]
    pub fn shape(&mut self, ctx: &egui::Context, screen: Rect, backdrop: Backdrop, shapes: &[Shape], tone: Color32, look: Look) -> Option<egui::Shape> {
        let ppp = ctx.pixels_per_point();
        let size = [(screen.width() * ppp).round().max(1.0) as usize, (screen.height() * ppp).round().max(1.0) as usize];
        let key = key(size, shapes, look);
        if let Some(done) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            if let Ok(mut shared) = self.shared.lock() {
                shared.fields = Some(Arc::new(done));
                shared.generation += 1;
            }
        }
        if self.key.as_deref() != Some(key.as_str()) && self.pending.is_none() {
            let scale = size[1] as f32 / 900.0;
            let (bevel, curve, blur) = (look.bevel * scale, look.curve, (size[1] as f32 * 0.012).max(1.0));
            let atlas = shapes.iter().any(|s| matches!(s, Shape::Text(..))).then(|| ctx.fonts(|fonts| fonts.image()));
            let (shapes, origin, repaint) = (shapes.to_vec(), screen.min, ctx.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("des-ui-glass".into()).spawn(move || {
                let _ = tx.send(fields(atlas.as_ref(), size, origin, ppp, &shapes, bevel, curve, blur));
                repaint.request_repaint();
            });
            if spawned.is_ok() { self.pending = Some(rx); self.key = Some(key); }
        }
        if !self.available() || !self.ready() { return None; }
        let shared = self.shared.clone();
        let to_uv = |r: Rect| [(r.min.x - screen.min.x) / screen.width(), (r.min.y - screen.min.y) / screen.height(), r.width() / screen.width(), r.height() / screen.height()];
        let dest = to_uv(backdrop.dest);
        let uv = [backdrop.uv.min.x, backdrop.uv.min.y, backdrop.uv.width(), backdrop.uv.height()];
        let tone = [tone.r() as f32 / 255.0, tone.g() as f32 / 255.0, tone.b() as f32 / 255.0];
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            let Ok(mut shared) = shared.lock() else { return };
            let viewport = info.viewport_in_pixels();
            // SAFETY: called by egui_glow on its own GL context during painting; every
            // object used here was made on that context, and egui_glow restores its
            // own state after a callback.
            unsafe { draw(&mut shared, painter, [viewport.left_px, viewport.from_bottom_px, viewport.width_px, viewport.height_px], backdrop, dest, uv, tone, look) };
        });
        Some(egui::Shape::Callback(egui::PaintCallback { rect: screen, callback: Arc::new(callback) }))
    }
}

/// What the fields depend on, as text, so a change in any of it recomputes them.
fn key(size: [usize; 2], shapes: &[Shape], look: Look) -> String {
    let mut key = format!("{}x{} {} {}", size[0], size[1], look.bevel, look.curve);
    for shape in shapes {
        match shape {
            Shape::Pill(r) => key.push_str(&format!(" p{:.1},{:.1},{:.1},{:.1}", r.min.x, r.min.y, r.max.x, r.max.y)),
            Shape::Text(galley, at) => key.push_str(&format!(" t{:?}@{:.1},{:.1}/{:.1}", galley.text(), at.x, at.y, galley.size().y)),
        }
    }
    key
}

/// Coverage of every shape at `size` pixels, its exact distance field shaped into a
/// bevel, and a blur of the coverage for the drop shadow.
#[allow(clippy::too_many_arguments)]
fn fields(atlas: Option<&egui::epaint::FontImage>, size: [usize; 2], origin: Pos2, ppp: f32, shapes: &[Shape], bevel: f32, curve: f32, blur: f32) -> Fields {
    let [w, h] = size;
    let mut cover = vec![0.0_f32; w * h];
    let mut regions = Vec::new();
    for shape in shapes {
        let bounds = match shape {
            Shape::Pill(rect) => { pill(&mut cover, size, origin, ppp, *rect); *rect }
            Shape::Text(galley, at) => {
                if let Some(atlas) = atlas { text(&mut cover, size, origin, ppp, galley, *at, atlas); }
                Rect::from_min_size(*at, galley.size())
            }
        };
        let r = bounds.translate(-origin.to_vec2()).expand(bevel / ppp + 4.0);
        let x0 = ((r.min.x * ppp).floor().max(0.0) as usize).min(w);
        let y0 = ((r.min.y * ppp).floor().max(0.0) as usize).min(h);
        let x1 = ((r.max.x * ppp).ceil().max(0.0) as usize).min(w);
        let y1 = ((r.max.y * ppp).ceil().max(0.0) as usize).min(h);
        if x1 > x0 && y1 > y0 { regions.push((x0, y0, x1, y1)); }
    }
    let mut surface = vec![0.0_f32; w * h * 4];
    for (x0, y0, x1, y1) in regions {
        bevel_region(&cover, &mut surface, size, (x0, y0, x1, y1), bevel.max(1.0), curve);
    }
    // The coverage in r, its blur (for the shadow) in g.
    let blurred = box_blur(&cover, size, blur);
    let mut both = vec![0.0_f32; w * h * 4];
    for i in 0..w * h { both[i * 4] = cover[i]; both[i * 4 + 1] = blurred[i]; }
    Fields { size, surface, cover: both }
}

/// A fully rounded rectangle, antialiased.
fn pill(cover: &mut [f32], [w, h]: [usize; 2], origin: Pos2, ppp: f32, rect: Rect) {
    let r = Rect::from_min_max(((rect.min - origin) * ppp).to_pos2(), ((rect.max - origin) * ppp).to_pos2());
    let radius = r.height().min(r.width()) / 2.0;
    let inner = r.shrink(radius);
    let (x0, y0) = (r.min.x.floor().max(0.0) as usize, r.min.y.floor().max(0.0) as usize);
    let (x1, y1) = ((r.max.x.ceil() as usize).min(w), (r.max.y.ceil() as usize).min(h));
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let dx = (inner.min.x - p.x).max(p.x - inner.max.x).max(0.0);
            let dy = (inner.min.y - p.y).max(p.y - inner.max.y).max(0.0);
            let d = (dx * dx + dy * dy).sqrt() - radius;
            let c = (0.5 - d).clamp(0.0, 1.0);
            let i = y * w + x;
            cover[i] = cover[i].max(c);
        }
    }
}

/// Text copied out of egui's font atlas, glyph by glyph, at its place on the screen.
fn text(cover: &mut [f32], [w, h]: [usize; 2], origin: Pos2, ppp: f32, galley: &egui::Galley, at: Pos2, atlas: &egui::epaint::FontImage) {
    let [aw, ah] = atlas.size;
    for row in &galley.rows {
        for glyph in &row.glyphs {
            let uv = glyph.uv_rect;
            if uv.max[0] <= uv.min[0] || uv.max[1] <= uv.min[1] { continue; }
            let top_left = (at + glyph.pos.to_vec2() + uv.offset - origin) * ppp;
            let texels = [(uv.max[0] - uv.min[0]) as usize, (uv.max[1] - uv.min[1]) as usize];
            let size = uv.size * ppp;
            let (x0, y0) = (top_left.x.floor().max(0.0) as usize, top_left.y.floor().max(0.0) as usize);
            let (x1, y1) = (((top_left.x + size.x).ceil() as usize).min(w), ((top_left.y + size.y).ceil() as usize).min(h));
            for y in y0..y1 {
                for x in x0..x1 {
                    // Bilinear sample of the glyph's texels.
                    let u = ((x as f32 + 0.5 - top_left.x) / size.x * texels[0] as f32 - 0.5).clamp(0.0, texels[0] as f32 - 1.0);
                    let v = ((y as f32 + 0.5 - top_left.y) / size.y * texels[1] as f32 - 0.5).clamp(0.0, texels[1] as f32 - 1.0);
                    let (u0, v0) = (u.floor() as usize, v.floor() as usize);
                    let (u1, v1) = ((u0 + 1).min(texels[0] - 1), (v0 + 1).min(texels[1] - 1));
                    let (fu, fv) = (u - u0 as f32, v - v0 as f32);
                    let at = |uu: usize, vv: usize| {
                        let (ax, ay) = (uv.min[0] as usize + uu, uv.min[1] as usize + vv);
                        if ax < aw && ay < ah { atlas.pixels[ay * aw + ax] } else { 0.0 }
                    };
                    let c = (at(u0, v0) * (1.0 - fu) + at(u1, v0) * fu) * (1.0 - fv) + (at(u0, v1) * (1.0 - fu) + at(u1, v1) * fu) * fv;
                    let i = y * w + x;
                    cover[i] = cover[i].max(c);
                }
            }
        }
    }
}

/// The squircle bevel over the exact distance from the edge, inside one region:
/// slope x and y (scaled by the bevel width), height, and distance in pixels.
fn bevel_region(cover: &[f32], surface: &mut [f32], [w, _h]: [usize; 2], (x0, y0, x1, y1): (usize, usize, usize, usize), bevel: f32, curve: f32) {
    let (rw, rh) = (x1 - x0, y1 - y0);
    let inside: Vec<bool> = (0..rw * rh).map(|i| cover[(y0 + i / rw) * w + x0 + i % rw] > 0.5).collect();
    let distance = distance_field(&inside, rw, rh);
    let height: Vec<f32> = distance.iter().zip(&inside).map(|(&d, &inside)| {
        if !inside { return 0.0; }
        let t = (d / bevel).min(1.0);
        (1.0 - (1.0 - t).powf(curve)).powf(1.0 / curve)
    }).collect();
    for y in 1..rh.saturating_sub(1) {
        for x in 1..rw.saturating_sub(1) {
            let i = y * rw + x;
            if !inside[i] { continue; }
            let o = ((y0 + y) * w + x0 + x) * 4;
            surface[o] = (height[i + 1] - height[i - 1]) * 0.5 * bevel;
            surface[o + 1] = (height[i + rw] - height[i - rw]) * 0.5 * bevel;
            surface[o + 2] = height[i];
            surface[o + 3] = distance[i];
        }
    }
}

/// Exact Euclidean distance, in pixels, from every inside pixel to the nearest
/// outside one (Felzenszwalb and Huttenlocher), zero outside.
pub fn distance_field(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f64 = 1e20;
    let mut grid: Vec<f64> = inside.iter().map(|&i| if i { FAR } else { 0.0 }).collect();
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h { f[y] = grid[y * w + x]; }
        transform(&f[..h], &mut d, &mut v, &mut z);
        for y in 0..h { grid[y * w + x] = d[y]; }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..y * w + w]);
        transform(&f[..w], &mut d, &mut v, &mut z);
        for x in 0..w { grid[y * w + x] = d[x]; }
    }
    grid.into_iter().map(|d| d.sqrt() as f32).collect()
}

/// One dimension of the squared distance transform: the lower envelope of parabolas.
fn transform(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 { return; }
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let mut s;
        loop {
            let p = v[k] as f64;
            s = ((f[q] + qf * qf) - (f[v[k]] + p * p)) / (2.0 * qf - 2.0 * p);
            if s <= z[k] && k > 0 { k -= 1; } else { break; }
        }
        if s <= z[k] { // k == 0: replace the first parabola
            v[0] = q;
            z[0] = f64::NEG_INFINITY;
            z[1] = f64::INFINITY;
            continue;
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 { k += 1; }
        let p = q as f64 - v[k] as f64;
        *out = p * p + f[v[k]];
    }
}

/// Three box blurs, close to a Gaussian of the given radius.
fn box_blur(source: &[f32], [w, h]: [usize; 2], radius: f32) -> Vec<f32> {
    let r = (radius / 1.7).round().max(1.0) as usize;
    let mut a = source.to_vec();
    let mut b = vec![0.0_f32; w * h];
    for _ in 0..3 {
        for y in 0..h {
            let row = &a[y * w..y * w + w];
            let mut sum: f32 = row[..r.min(w)].iter().sum();
            for x in 0..w {
                if x + r < w { sum += row[x + r]; }
                if x > r { sum -= row[x - r - 1]; }
                let count = (x + r).min(w - 1) + 1 - x.saturating_sub(r);
                b[y * w + x] = sum / count as f32;
            }
        }
        for x in 0..w {
            let mut sum: f32 = (0..r.min(h)).map(|y| b[y * w + x]).sum();
            for y in 0..h {
                if y + r < h { sum += b[(y + r) * w + x]; }
                if y > r { sum -= b[(y - r - 1) * w + x]; }
                let count = (y + r).min(h - 1) + 1 - y.saturating_sub(r);
                a[y * w + x] = sum / count as f32;
            }
        }
    }
    a
}

const VERTEX: &str = r#"
out vec2 uv;
void main() {
    // One triangle that covers the viewport.
    vec2 corner = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    uv = corner;
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"
in vec2 uv;
out vec4 color;
uniform sampler2D sharp, soft, surface, cover;
uniform vec2 res;
uniform vec4 dest, area;
uniform vec3 tone;
uniform float srgb, depth, index, cauchy, thick, gap, absorb, density, coat, film, frost, tint, lift, edge, edgew, mirror, spec, shine, light, shadow, scrim;
vec3 gamma(vec3 c) { return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c)); }
vec2 wall(vec2 at) { return area.xy + (at - dest.xy) / dest.zw * area.zw; }
// Everything is worked in gamma space, as in the lab where the look was tuned; an
// sRGB texture reads back linear, so it is brought back to gamma.
vec3 sample_(sampler2D t, vec2 at) { vec3 c = texture(t, wall(at)).rgb; return srgb > 0.5 ? gamma(c) : c; }
vec3 back(vec2 at, float f) { return mix(sample_(sharp, at), sample_(soft, at), f); }
float fresnel(float cosI, float n1, float n2) {
    float sinT2 = (n1 / n2) * (n1 / n2) * (1.0 - cosI * cosI);
    if (sinT2 >= 1.0) return 1.0;
    float cosT = sqrt(1.0 - sinT2);
    float rs = (n1 * cosI - n2 * cosT) / (n1 * cosI + n2 * cosT);
    float rp = (n2 * cosI - n1 * cosT) / (n2 * cosI + n1 * cosT);
    return 0.5 * (rs * rs + rp * rp);
}
vec3 trace(vec3 n, float ior, float slab, out float through, out float path) {
    vec3 into = vec3(0.0, 0.0, -1.0);
    float r1 = fresnel(max(n.z, 0.0), 1.0, ior);
    vec3 t1 = refract(into, n, 1.0 / ior);
    float travel = slab / max(-t1.z, 0.05);
    path = travel;
    vec2 shift = t1.xy * travel;
    vec3 t2 = refract(t1, vec3(0.0, 0.0, 1.0), ior);
    float r2 = fresnel(max(-t1.z, 0.0), ior, 1.0);
    if (dot(t2, t2) < 1e-6) { through = 0.0; return vec3(shift, r1); }
    shift += t2.xy * gap / max(-t2.z, 0.05);
    through = (1.0 - r1) * (1.0 - r2) * exp(-absorb * travel);
    return vec3(shift, r1);
}
void main() {
    vec2 px = 1.0 / res;
    // The texture rows run top down; screen uv here runs bottom up.
    vec2 at = vec2(uv.x, 1.0 - uv.y);
    vec3 base = sample_(sharp, at);
    float top = smoothstep(0.38, 0.0, at.y), bottom = smoothstep(0.55, 1.0, at.y);
    base *= 1.0 - scrim * max(top * 0.75, bottom);
    vec4 c = texture(cover, at);
    float m = c.r;
    float drop = texture(cover, at - vec2(0.0, 6.0) * px).g;
    base *= 1.0 - shadow * drop * (1.0 - m);
    if (m < 0.002) { color = vec4(base, 1.0); return; }
    vec4 f = texture(surface, at);
    // Screen y runs down in the fields; the normal's y follows.
    vec3 n = normalize(vec3(-f.xy * depth, 1.0));
    float slab = thick * f.z;
    float a = index - cauchy / (0.55 * 0.55);
    vec3 iors = vec3(a + cauchy / (0.65 * 0.65), index, a + cauchy / (0.45 * 0.45));
    float tr, tg, tb, pr, pg, pb;
    vec3 sr = trace(n, iors.r, slab, tr, pr);
    vec3 sg = trace(n, iors.g, slab, tg, pg);
    vec3 sb = trace(n, iors.b, slab, tb, pb);
    vec3 sigma = -log(clamp(tone, 0.02, 1.0)) * density;
    vec3 body = exp(-sigma * vec3(pr, pg, pb));
    float cosI = clamp(n.z, 0.0, 1.0);
    float filmIndex = 1.38;
    float cosF = sqrt(max(1.0 - (1.0 - cosI * cosI) / (filmIndex * filmIndex), 0.0));
    vec3 phase = 4.0 * 3.14159265 * filmIndex * film * cosF / vec3(650.0, 550.0, 450.0);
    vec3 film_ = coat * 0.5 * (1.0 - cos(phase));
    vec3 seen = vec3(back(at + sr.xy * px, frost).r * tr, back(at + sg.xy * px, frost).g * tg, back(at + sb.xy * px, frost).b * tb);
    seen *= body * (1.0 - 0.6 * film_);
    vec3 bounce = reflect(vec3(0.0, 0.0, -1.0), n);
    vec2 mirrored = clamp(vec2(at.x, 1.0 - at.y) + bounce.xy * 0.25, 0.0, 1.0);
    vec3 room = sample_(soft, mirrored) * (1.0 + lift) + 0.08;
    vec3 shine_ = vec3(sr.z, sg.z, sb.z) + film_ * (0.25 + 0.75 * (1.0 - cosI));
    seen += room * shine_ * mirror;
    seen = seen * (1.0 + lift) + lift * 0.04;
    seen = mix(seen, vec3(1.0), tint);
    float ang = radians(light);
    vec2 toward = vec2(cos(ang), sin(ang));
    vec3 l = normalize(vec3(toward, 0.6));
    vec3 h = normalize(l + vec3(0.0, 0.0, 1.0));
    float highlight = pow(max(dot(n, h), 0.0), shine) * (shine + 2.0) / 6.28 * 0.05 * spec;
    vec2 outward = length(f.xy) > 1e-4 ? -normalize(f.xy) : vec2(0.0);
    float facing = dot(outward, toward);
    float band = 1.0 - smoothstep(0.0, edgew, f.w);
    seen += vec3(highlight);
    seen += vec3(band * edge * 0.55 * (0.35 + 0.65 * max(facing, 0.0)));
    seen *= 1.0 - band * edge * 0.45 * max(-facing, 0.0);
    color = vec4(mix(base, clamp(seen, 0.0, 1.0), m), 1.0);
}
"#;

/// Builds the program and the textures if needed, uploads new fields, and draws.
#[allow(clippy::too_many_arguments)]
unsafe fn draw(shared: &mut Shared, painter: &egui_glow::Painter, viewport: [i32; 4], backdrop: Backdrop, dest: [f32; 4], area: [f32; 4], tone: [f32; 3], look: Look) {
    let gl = painter.gl().clone();
    let gl = gl.as_ref();
    let version = egui_glow::ShaderVersion::get(gl);
    if !version.is_new_shader_interface() || shared.gpu.failed {
        shared.gpu.failed = true;
        return;
    }
    let (Some(sharp), Some(soft)) = (painter.texture(backdrop.sharp), painter.texture(backdrop.soft)) else { return; };
    unsafe {
        if shared.gpu.program.is_none() {
            shared.gpu.srgb = version == egui_glow::ShaderVersion::Es300 || gl.supported_extensions().iter().any(|e| e.contains("sRGB"));
            match program(gl, version.version_declaration()) {
                Ok(p) => shared.gpu.program = Some(p),
                Err(why) => { eprintln!("des-ui: glass unavailable: {why}"); shared.gpu.failed = true; return; }
            }
            shared.gpu.vao = gl.create_vertex_array().ok();
            shared.gpu.surface = gl.create_texture().ok();
            shared.gpu.cover = gl.create_texture().ok();
        }
        let (Some(program), Some(vao), Some(surface), Some(cover)) = (shared.gpu.program, shared.gpu.vao, shared.gpu.surface, shared.gpu.cover) else { return; };
        if let Some(fields) = shared.fields.take().filter(|_| shared.gpu.uploaded != shared.generation) {
            for (texture, data) in [(surface, &fields.surface), (cover, &fields.cover)] {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for (name, value) in [(glow::TEXTURE_MIN_FILTER, glow::LINEAR), (glow::TEXTURE_MAG_FILTER, glow::LINEAR), (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE), (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE)] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
                }
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                let bytes: &[u8] = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4);
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA16F as i32, fields.size[0] as i32, fields.size[1] as i32, 0, glow::RGBA, glow::FLOAT, glow::PixelUnpackData::Slice(Some(bytes)));
            }
            shared.gpu.uploaded = shared.generation;
            shared.gpu.size = fields.size;
        }
        if shared.gpu.uploaded == 0 { return; }
        let size = shared.gpu.size;
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.bind_vertex_array(Some(vao));
        for (unit, (name, texture)) in [("sharp", sharp), ("soft", soft), ("surface", surface), ("cover", cover)].into_iter().enumerate() {
            gl.active_texture(glow::TEXTURE0 + unit as u32);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
        }
        let scale = size[1] as f32 / 900.0;
        let set = |name: &str, value: f32| gl.uniform_1_f32(gl.get_uniform_location(program, name).as_ref(), value);
        gl.uniform_2_f32(gl.get_uniform_location(program, "res").as_ref(), size[0] as f32, size[1] as f32);
        gl.uniform_4_f32(gl.get_uniform_location(program, "dest").as_ref(), dest[0], dest[1], dest[2], dest[3]);
        gl.uniform_4_f32(gl.get_uniform_location(program, "area").as_ref(), area[0], area[1], area[2], area[3]);
        gl.uniform_3_f32(gl.get_uniform_location(program, "tone").as_ref(), tone[0], tone[1], tone[2]);
        set("srgb", if shared.gpu.srgb { 1.0 } else { 0.0 });
        set("depth", look.depth); set("index", look.index); set("cauchy", look.dispersion);
        set("thick", look.thickness * scale); set("gap", look.gap * scale); set("absorb", look.absorption);
        set("density", look.colour_depth); set("coat", look.iridescence); set("film", look.film_nm);
        set("frost", look.frost); set("tint", look.tint); set("lift", look.luminosity);
        set("edge", look.edge); set("edgew", look.edge_width * scale); set("mirror", look.reflection);
        set("spec", look.highlight); set("shine", look.highlight_size); set("light", look.light_degrees);
        set("shadow", look.shadow); set("scrim", look.scrim);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.active_texture(glow::TEXTURE0);
    }
}

/// Compiles and links the glass program for this GL's shading language.
unsafe fn program(gl: &glow::Context, version: &str) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let header = format!("{version}precision highp float;\n");
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) { return Err(gl.get_shader_info_log(shader)); }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) { return Err(gl.get_program_info_log(program)); }
        for shader in shaders { gl.detach_shader(program, shader); gl.delete_shader(shader); }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_exact_inside_a_square() {
        let (w, h) = (11, 11);
        let inside: Vec<bool> = (0..w * h).map(|i| { let (x, y) = (i % w, i / w); (2..9).contains(&x) && (2..9).contains(&y) }).collect();
        let d = distance_field(&inside, w, h);
        assert_eq!(d[5 * w + 5], 4.0, "centre is four pixels from the nearest outside pixel");
        assert_eq!(d[5 * w + 2], 1.0);
        assert_eq!(d[0], 0.0);
        assert!((d[2 * w + 2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn distance_matches_brute_force_on_a_ring() {
        let (w, h) = (24, 18);
        let inside: Vec<bool> = (0..w * h).map(|i| { let (x, y) = (i % w, i / w); let r = ((x as f32 - 12.0).powi(2) + (y as f32 - 9.0).powi(2)).sqrt(); (3.0..8.0).contains(&r) }).collect();
        let d = distance_field(&inside, w, h);
        for i in 0..w * h {
            let brute = if !inside[i] { 0.0 } else {
                (0..w * h).filter(|&j| !inside[j]).map(|j| { let (dx, dy) = ((i % w) as f32 - (j % w) as f32, (i / w) as f32 - (j / w) as f32); (dx * dx + dy * dy).sqrt() }).fold(f32::MAX, f32::min)
            };
            assert!((d[i] - brute).abs() < 1e-4, "pixel {i}: {} vs {brute}", d[i]);
        }
    }

    #[test]
    fn a_pill_covers_its_middle_and_not_its_corners() {
        let size = [40, 20];
        let mut cover = vec![0.0; 800];
        pill(&mut cover, size, pos2(0.0, 0.0), 1.0, Rect::from_min_max(pos2(4.0, 4.0), pos2(36.0, 16.0)));
        assert_eq!(cover[10 * 40 + 20], 1.0);
        assert_eq!(cover[4 * 40 + 4], 0.0, "rounded ends leave the corners empty");
        assert_eq!(cover[2 * 40 + 20], 0.0);
    }

    #[test]
    fn the_bevel_is_steep_at_the_rim_and_flat_inside() {
        let size = [60, 30];
        let mut cover = vec![0.0; 1800];
        pill(&mut cover, size, pos2(0.0, 0.0), 1.0, Rect::from_min_max(pos2(5.0, 5.0), pos2(55.0, 25.0)));
        let mut surface = vec![0.0; 1800 * 4];
        bevel_region(&cover, &mut surface, size, (0, 0, 60, 30), 6.0, 3.0);
        let height = |x: usize, y: usize| surface[(y * 60 + x) * 4 + 2];
        assert!((height(30, 15) - 1.0).abs() < 1e-6, "flat in the middle");
        assert!(height(30, 6) < height(30, 9) && height(30, 9) <= height(30, 12));
        let slope = |x: usize, y: usize| surface[(y * 60 + x) * 4 + 1];
        assert!(slope(30, 6) > 0.0, "height rises going inward from the top edge");
        assert_eq!(slope(30, 15), 0.0);
    }

    #[test]
    fn the_shadow_blur_keeps_its_total() {
        let size = [32, 32];
        let mut source = vec![0.0; 1024];
        for y in 12..20 { for x in 12..20 { source[y * 32 + x] = 1.0; } }
        let blurred = box_blur(&source, size, 4.0);
        let (a, b): (f32, f32) = (source.iter().sum(), blurred.iter().sum());
        assert!((a - b).abs() / a < 0.02, "{a} vs {b}");
        assert!(blurred[16 * 32 + 16] < 1.0 && blurred[16 * 32 + 9] > 0.0);
    }
}

