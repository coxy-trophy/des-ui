//! Native scenes shared by the locked surface and Settings preview.
//! All animation is a function of elapsed time; the saver keeps no particle history.

use crate::config::Saver;
use eframe::egui::{self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;
use std::time::Duration;

const WORDMARK: &[u8; 848 * 198 * 2] = include_bytes!("../../compositor/assets/wordmark.la");
const MARK_W: usize = 848;
const MARK_H: usize = 198;
const MARK_ASPECT: f32 = MARK_H as f32 / MARK_W as f32;
type Rgb = [u8; 3];

pub(crate) fn show(
    ctx: &egui::Context,
    style: Saver,
    elapsed: Duration,
    reduced: bool,
    on_battery: bool,
) {
    let rect = ctx.screen_rect();
    if matches!(style, Saver::Off) || !rect.is_finite() || rect.width() < 1.0 || rect.height() < 1.0
    {
        return;
    }
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("screensaver"),
        ))
        .with_clip_rect(rect);
    let time = if reduced { 0.0 } else { elapsed.as_secs_f32() };
    Scene {
        painter,
        rect,
        w: rect.width(),
        h: rect.height(),
    }
    .draw(style, time, reduced);
    if !reduced {
        // Use ceil(1000 / fps) so neither interval can exceed its cap.
        ctx.request_repaint_after(Duration::from_millis(if on_battery { 84 } else { 42 }));
    }
}

fn mix(a: f32, b: f32, f: f32) -> f32 {
    a + (b - a) * f
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let p = ((v - a) / (b - a)).clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn tint(a: Rgb, b: Rgb, f: f32) -> Rgb {
    std::array::from_fn(|i| mix(a[i] as f32, b[i] as f32, f.clamp(0.0, 1.0)).round() as u8)
}

fn color([r, g, b]: Rgb, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (255.0 * a.clamp(0.0, 1.0)).round() as u8)
}

// Seeded independently by index so a particle has the same appearance each frame.
fn rand(i: usize) -> f32 {
    let mut bits = (i as u32).wrapping_add(1).wrapping_mul(0x9e37_79b9);
    bits ^= bits >> 16;
    bits = bits.wrapping_mul(0x7feb_352d);
    bits ^= bits >> 15;
    (bits >> 8) as f32 / 16_777_216.0
}

fn alpha(x: usize, y: usize) -> u8 {
    let index = (y * MARK_W + x) * 2;
    WORDMARK[index].saturating_add(WORDMARK[index + 1])
}

fn mask(u: f32, v: f32) -> f32 {
    if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
        alpha((u * MARK_W as f32) as usize, (v * MARK_H as f32) as usize) as f32 / 255.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
struct MarkPoint {
    u: f32,
    v: f32,
    seed: f32,
    fuzz: f32,
}

fn mark_points() -> &'static [MarkPoint] {
    static POINTS: OnceLock<Vec<MarkPoint>> = OnceLock::new();
    POINTS.get_or_init(|| {
        let mut points = Vec::new();
        for y in (2..MARK_H).step_by(5) {
            for x in (2..MARK_W).step_by(5) {
                if alpha(x, y) > 115 {
                    let i = y * MARK_W + x;
                    points.push(MarkPoint {
                        u: x as f32 / MARK_W as f32,
                        v: y as f32 / MARK_H as f32,
                        seed: rand(i * 2),
                        fuzz: rand(i * 2 + 1),
                    });
                }
            }
        }
        points
    })
}

fn mark_runs() -> &'static [(f32, f32, f32, f32)] {
    static RUNS: OnceLock<Vec<(f32, f32, f32, f32)>> = OnceLock::new();
    RUNS.get_or_init(|| {
        let mut runs = Vec::new();
        for y in (4..MARK_H).step_by(5) {
            let mut left = None;
            for x in 0..=MARK_W {
                if x < MARK_W && alpha(x, y) > 115 {
                    left.get_or_insert(x);
                } else if let Some(a) = left.take().filter(|a| x - a > 2) {
                    runs.push((
                        a as f32 / MARK_W as f32,
                        x as f32 / MARK_W as f32,
                        y as f32 / MARK_H as f32,
                        rand(y * MARK_W + a),
                    ));
                }
            }
        }
        runs
    })
}

struct Scene {
    painter: Painter,
    rect: Rect,
    w: f32,
    h: f32,
}

impl Scene {
    fn draw(&self, style: Saver, t: f32, reduced: bool) {
        match style {
            Saver::Off => {}
            Saver::SilkInk => self.folds(t, Fold::Ink, reduced),
            Saver::SignalContour => self.folds(t, Fold::Contour, reduced),
            Saver::TerminalWeave => self.folds(t, Fold::Terminal, reduced),
            Saver::NightTransit => self.transit(if reduced { 3.4 } else { t }),
            Saver::RisingLines => self.rising(t, reduced),
            Saver::EventHorizon => self.eclipse(if reduced { 2.6 } else { t }),
            Saver::PolarVeil => self.polar(if reduced { 4.2 } else { t }),
        }
    }

    fn pos(&self, x: f32, y: f32) -> Pos2 {
        self.rect.min + Vec2::new(x, y)
    }

    fn at(&self, u: f32, v: f32) -> Pos2 {
        self.pos(self.w * u, self.h * v)
    }

    fn mesh(&self, mesh: Mesh) {
        if !mesh.is_empty() {
            self.painter.add(Shape::mesh(mesh));
        }
    }

    fn ground(&self, top: Rgb, bottom: Rgb) {
        let mut mesh = Mesh::default();
        quad(
            &mut mesh,
            self.rect,
            [
                color(top, 1.0),
                color(top, 1.0),
                color(bottom, 1.0),
                color(bottom, 1.0),
            ],
        );
        self.mesh(mesh);
    }

    fn brand(&self, center: Pos2, width: f32, c: Rgb, opacity: f32) {
        let ctx = self.painter.ctx();
        let id = egui::Id::new("screensaver-boot-wordmark");
        let cached = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id));
        let texture = cached.unwrap_or_else(|| {
            let pixels = WORDMARK
                .chunks_exact(2)
                .map(|p| Color32::from_white_alpha(p[0].saturating_add(p[1])))
                .collect();
            let texture = ctx.load_texture(
                "screensaver-boot-wordmark",
                egui::ColorImage {
                    size: [MARK_W, MARK_H],
                    pixels,
                },
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
        self.painter.image(
            texture.id(),
            Rect::from_center_size(center, Vec2::new(width, width * MARK_ASPECT)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            color(c, opacity),
        );
    }

    fn stars(&self, t: f32, total: usize, ceiling: f32) {
        let mut mesh = Mesh::default();
        for i in 0..total {
            let j = i * 6;
            let p = self.at(rand(j), rand(j + 1) * ceiling);
            let a = (0.20 + rand(j + 2) * 0.62) * (0.84 + 0.16 * (t * 0.23 + i as f32).sin());
            let c = if i % 8 == 0 {
                [160, 203, 236]
            } else {
                [214, 230, 244]
            };
            dot(&mut mesh, p, 0.3 + rand(j + 3) * 0.85, color(c, a));
        }
        self.mesh(mesh);
    }

    fn glow(&self, center: Pos2, radius: Vec2, c: Rgb, opacity: f32) {
        self.mesh(radial(
            center,
            radius,
            &[
                (0.0, color(c, opacity)),
                (0.38, color(c, opacity * 0.31)),
                (1.0, Color32::TRANSPARENT),
            ],
        ));
    }
}

// Colors are ordered clockwise from top left.
fn quad(mesh: &mut Mesh, r: Rect, colors: [Color32; 4]) {
    let base = mesh.vertices.len() as u32;
    for (p, c) in [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
    .into_iter()
    .zip(colors)
    {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

// The transparent outer edge antialiases each particle without producing thousands of egui shapes.
fn dot(mesh: &mut Mesh, center: Pos2, radius: f32, fill: Color32) {
    const EDGE: [Vec2; 6] = [
        Vec2::new(1.0, 0.0),
        Vec2::new(0.5, 0.866),
        Vec2::new(-0.5, 0.866),
        Vec2::new(-1.0, 0.0),
        Vec2::new(-0.5, -0.866),
        Vec2::new(0.5, -0.866),
    ];
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, fill);
    let core = (radius - 0.45).max(radius * 0.38);
    for direction in EDGE {
        mesh.colored_vertex(center + direction * core, fill);
        mesh.colored_vertex(center + direction * (radius + 0.4), Color32::TRANSPARENT);
    }
    for i in 0..6 {
        let a = base + 1 + 2 * i;
        let b = base + 1 + 2 * ((i + 1) % 6);
        mesh.add_triangle(base, a, b);
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(b, a + 1, b + 1);
    }
}

fn radial(center: Pos2, radii: Vec2, stops: &[(f32, Color32)]) -> Mesh {
    const N: u32 = 64;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, stops[0].1);
    for (j, &(r, fill)) in stops[1..].iter().enumerate() {
        let start = 1 + j as u32 * N;
        for i in 0..N {
            let axis = Vec2::angled(TAU * i as f32 / N as f32);
            mesh.colored_vertex(
                center + Vec2::new(axis.x * radii.x, axis.y * radii.y) * r,
                fill,
            );
            let a = start + i;
            let b = start + (i + 1) % N;
            if j == 0 {
                mesh.add_triangle(0, a, b);
            } else {
                mesh.add_triangle(a - N, a, b);
                mesh.add_triangle(a - N, b, b - N);
            }
        }
    }
    mesh
}

#[derive(PartialEq)]
enum Fold {
    Ink,
    Contour,
    Terminal,
}

fn surface(u: f32, v: f32, t: f32) -> f32 {
    let fold = (u * 13.5 + v * 5.2 + 1.18 * (v * 5.2 - u * 3.5).sin() - t * 0.63).sin();
    let under = (u * 24.0 - v * 3.6 + t * 0.5).sin();
    fold * 0.77 + under * 0.23
}

impl Scene {
    fn folds(&self, t: f32, fold: Fold, reduced: bool) {
        let (base, halo, outer, ink, highlight) = match fold {
            Fold::Ink => (
                [5, 2, 8],
                [34, 7, 25],
                [16, 6, 20],
                [173, 34, 123],
                [215, 101, 172],
            ),
            Fold::Contour => (
                [6, 4, 10],
                [31, 11, 31],
                [16, 6, 20],
                [237, 86, 190],
                [255, 209, 236],
            ),
            Fold::Terminal => (
                [4, 6, 9],
                [16, 16, 28],
                [8, 10, 18],
                [212, 74, 182],
                [158, 245, 225],
            ),
        };
        self.ground(base, base);
        let r = (self.w * 0.45).max(self.h * 0.65);
        self.mesh(radial(
            self.at(0.51, 0.51),
            Vec2::splat(r),
            &[
                (0.0, color(halo, 1.0)),
                (0.62, color(outer, 1.0)),
                (1.0, color(base, 1.0)),
            ],
        ));
        let cycle = (t + 0.4) % 13.0;
        let k = if reduced {
            1.0
        } else {
            smooth(
                0.0,
                1.0,
                smooth(2.0, 5.0, cycle) * (1.0 - smooth(9.3, 12.5, cycle)),
            )
        };
        if fold == Fold::Contour {
            self.contours(t, k, ink, highlight);
            let mut veil = Mesh::default();
            let clear = Color32::TRANSPARENT;
            let dim = color(base, 1.0);
            quad(
                &mut veil,
                Rect::from_min_max(self.rect.min, self.at(0.22, 1.0)),
                [dim, clear, clear, dim],
            );
            quad(
                &mut veil,
                Rect::from_min_max(self.at(0.80, 0.0), self.rect.max),
                [clear, dim, dim, clear],
            );
            self.mesh(veil);
        } else {
            self.fold_field(t, k, &fold, ink, highlight);
        }
        if fold != Fold::Ink && k >= 0.025 {
            self.fold_logo(t, k, &fold, ink, highlight);
        }
    }

    fn fold_field(&self, t: f32, k: f32, fold: &Fold, ink: Rgb, highlight: Rgb) {
        let terminal = *fold == Fold::Terminal;
        let step = if terminal {
            10.0_f32.max(self.w / 116.0).max(self.h / 80.0)
        } else {
            4.6_f32.max(self.w / 172.0).max(self.h / 86.0)
        };
        let lw = (self.w * 0.75).min(self.h * 1.32).min(690.0);
        let lh = lw * MARK_ASPECT;
        let left = (self.w - lw) * 0.5;
        let top = (self.h - lh) * 0.5;
        let glyphs = terminal.then(|| self.glyphs((step * 0.85).max(7.0)));
        let mut mesh = Mesh::default();
        for row in 0..(self.h / step).ceil() as usize + 2 {
            let y = (row as f32 - 1.0) * step;
            let v = y / self.h;
            for col in 0..(self.w / step).ceil() as usize {
                let x = col as f32 * step;
                let u = x / self.w;
                let height = surface(u, v, t);
                let slope = surface(u + step / self.w, v, t) - height;
                let edge = 1.0 - smooth(0.62, 1.12, ((u - 0.5) * 1.7).hypot((v - 0.51) * 1.75));
                let band = 1.0 - smooth(0.1, 0.60, (v - 0.51 + height * 0.2).abs());
                let mid = 1.0 - smooth(0.08, 0.65, (v - 0.5).abs() * 1.7 + (u - 0.5).abs() * 0.70);
                let light = (0.32 + slope * 2.6 + height * 0.3).clamp(0.0, 1.0);
                let py = y + height * self.h * 0.103;
                let dye = if terminal {
                    0.0
                } else {
                    smooth(
                        0.23,
                        0.75,
                        mask((x - left) / lw, (py - height * self.h * 0.018 - top) / lh),
                    )
                };
                let base = edge * (0.12 + band * 0.88) * (0.19 + light * 0.68);
                let opacity = if terminal {
                    base * (1.0 - k * (0.43 + mid * 0.52))
                } else {
                    mix(
                        base * (0.66 - mid * 0.36),
                        0.68 + 0.24 * light + 0.08 * k,
                        dye,
                    )
                };
                if opacity < 0.024 {
                    continue;
                }
                let p = self.pos(x, py);
                if let Some(glyphs) = &glyphs {
                    let seed = rand(row * 128 + col);
                    let index = ((light * 0.69 + seed * 0.3) * 7.0) as usize;
                    let fill = if seed > 0.92 { highlight } else { ink };
                    self.painter.galley(
                        p - Vec2::new(0.0, glyphs[index].size().y * 0.80),
                        glyphs[index].clone(),
                        color(fill, opacity),
                    );
                } else {
                    let radius =
                        (step * (0.115 + light * 0.25 + dye * (0.16 + 0.045 * k))).clamp(0.6, 3.8);
                    let fill = if dye > 0.3 {
                        if light > 0.54 {
                            [224, 84, 167]
                        } else {
                            [182, 29, 112]
                        }
                    } else if light > 0.83 {
                        highlight
                    } else {
                        ink
                    };
                    dot(&mut mesh, p, radius, color(fill, opacity));
                }
            }
        }
        self.mesh(mesh);
    }

    fn glyphs(&self, size: f32) -> [std::sync::Arc<egui::Galley>; 7] {
        [".", ":", "+", "*", "#", "%", "@"].map(|ch| {
            self.painter.layout_no_wrap(
                ch.to_owned(),
                FontId::monospace(size),
                Color32::PLACEHOLDER,
            )
        })
    }

    fn contours(&self, t: f32, k: f32, ink: Rgb, highlight: Rgb) {
        let step = (self.h / 42.0).max(10.0);
        let samples = (self.w / 8.0).ceil().clamp(2.0, 180.0) as usize;
        for row in 0..(1.2 * self.h / step).ceil() as usize {
            let y = -self.h * 0.1 + row as f32 * step;
            let v = y / self.h;
            let local = (v - 0.53).abs();
            let a = (1.0 - smooth(0.30, 0.66, local))
                * (0.33 + 0.16 * (row as f32 * 0.92 + t * 0.35).cos())
                * (1.0 - k * (0.45 + 0.43 * (1.0 - smooth(0.04, 0.45, local))));
            let bright = row % 6 == 0;
            let line = (0..=samples)
                .map(|i| {
                    let u = i as f32 / samples as f32;
                    self.pos(u * self.w, y + surface(u, v, t) * self.h * 0.14)
                })
                .collect();
            self.painter.add(Shape::line(
                line,
                Stroke::new(
                    if bright { 1.2 } else { 0.65 },
                    color(if bright { highlight } else { ink }, a),
                ),
            ));
        }
    }

    fn fold_logo(&self, t: f32, k: f32, fold: &Fold, ink: Rgb, highlight: Rgb) {
        let w = (self.w * 0.72).min(self.h * 1.28).min(670.0);
        let h = w * MARK_ASPECT;
        self.glow(
            self.at(0.5, 0.5),
            Vec2::splat(w * 0.58),
            if *fold == Fold::Terminal {
                [23, 26, 37]
            } else {
                [52, 16, 40]
            },
            k * 0.52,
        );
        let position = |u: f32, v: f32, seed: f32, fuzz: f32| {
            let tx = (self.w - w) * 0.5 + u * w;
            let ty = (self.h - h) * 0.5 + v * h;
            let x = tx + (u - 0.5) * self.w * 0.35 + (seed - 0.5) * 22.0;
            let v0 = 0.51 + (v - 0.5) * 0.32;
            let y = self.h * (v0 + surface(x / self.w, v0, t) * 0.15) + (fuzz - 0.5) * 11.0;
            self.pos(mix(x, tx, k), mix(y, ty, k))
        };
        if *fold == Fold::Contour {
            for &(a, b, v, seed) in mark_runs() {
                let fill = color(
                    if seed > 0.67 { highlight } else { ink },
                    (0.27 + seed * 0.69) * smooth(0.03, 0.85, k),
                );
                self.painter.line_segment(
                    [position(a, v, seed, 0.48), position(b, v, seed, 0.48)],
                    Stroke::new((1.8 * w / 600.0).max(0.7), fill),
                );
            }
        } else {
            let spacing = w * 5.0 / MARK_W as f32;
            let glyphs = self.glyphs((spacing * 1.22).max(4.5));
            for q in mark_points().iter().filter(|q| q.seed >= 0.18) {
                let p = position(q.u, q.v, q.seed, q.fuzz);
                let fill = color(
                    if q.seed > 0.48 { highlight } else { ink },
                    (0.32 + 0.68 * q.seed) * smooth(0.03, 0.9, k),
                );
                let glyph = &glyphs[(q.fuzz * 7.0) as usize];
                let origin = Align2::CENTER_CENTER.anchor_size(p, glyph.size()).min;
                self.painter.galley(origin, glyph.clone(), fill);
            }
        }
    }
}

fn connect_rows(mesh: &mut Mesh, a: u32, b: u32, columns: u32) {
    for i in 0..columns - 1 {
        mesh.add_triangle(a + i, a + i + 1, b + i);
        mesh.add_triangle(a + i + 1, b + i + 1, b + i);
    }
}

impl Scene {
    fn polar(&self, t: f32) {
        self.ground([4, 7, 19], [7, 19, 28]);
        self.stars(t, 112, 0.83);
        // Sample the glow at a fixed resolution. The renderer interpolates the soft sky;
        // the foreground silhouettes and mark stay sharp at the actual viewport size.
        const X: u32 = 112;
        const Y: u32 = 48;
        const STOPS: [Rgb; 4] = [[39, 34, 143], [41, 113, 170], [68, 216, 170], [94, 64, 191]];
        let mut sky = Mesh::default();
        for x in 0..X {
            let u = x as f32 / (X - 1) as f32;
            let hem = 0.52
                + (u * 5.2 - t * 0.21).sin() * 0.06
                + (u * 12.7 + t * 0.31).sin() * 0.034
                + (u * 2.6 + t * 0.17).sin() * 0.06;
            let stop = (u + 0.035 * (t * 0.23 + u * 4.0).sin()).clamp(0.0, 1.0) * 3.0;
            let low = (stop as usize).min(2);
            let fill = tint(STOPS[low], STOPS[low + 1], stop - low as f32);
            let wisps =
                0.91 + 0.09 * (u * 75.0 + t * 0.26 + 2.0 * (u * 11.0 - t * 0.19).sin()).sin();
            let drift = 0.43 * (u * 9.0 + t * 0.3).sin() - 0.4;
            for y in 0..Y {
                let v = y as f32 / (Y - 1) as f32;
                let d = (v - hem) / 0.105;
                let a = (0.52 / (1.0 + (d * 2.9).exp())
                    + 0.177 / (1.0 + ((d + drift) * 2.25).exp()))
                    * wisps
                    * (0.73 + 0.27 * (v * 3.2 + u * 1.4).sin());
                sky.colored_vertex(self.at(u, v), color(fill, a));
            }
            if x > 0 {
                connect_rows(&mut sky, (x - 1) * Y, x * Y, Y);
            }
        }
        self.mesh(sky);
        let mut mountains = Mesh::default();
        for (j, fill) in [[16, 27, 42], [11, 22, 35], [7, 14, 24]]
            .into_iter()
            .enumerate()
        {
            let j = j as f32;
            let base = mountains.vertices.len() as u32;
            for x in 0..=112 {
                let u = x as f32 / 112.0;
                let teeth =
                    ((u * 18.0 + j * 1.52).sin() + 0.46 * (u * 39.0 + j * 2.71).sin()).abs();
                let top = 0.86 + j * 0.052 - teeth * (0.09 - j * 0.019);
                mountains.colored_vertex(self.at(u, top), color(fill, 1.0));
                mountains.colored_vertex(self.at(u, 1.0), color(fill, 1.0));
                if x > 0 {
                    connect_rows(&mut mountains, base + (x - 1) * 2, base + x * 2, 2);
                }
            }
        }
        self.mesh(mountains);
        self.brand(
            self.at(0.5, 0.76),
            (self.w * 0.24).min(265.0),
            [183, 227, 214],
            0.64,
        );
    }

    fn eclipse(&self, t: f32) {
        self.ground([5, 3, 9], [10, 5, 13]);
        self.stars(t, 118, 1.0);
        let r = (self.w * 0.23).min(self.h * 0.307);
        let center = self.at(0.5, 0.48);
        self.glow(center, Vec2::splat(r * 2.5), [128, 28, 85], 0.46);
        self.glow(
            center + Vec2::new(r * 0.70, -r * 0.55),
            Vec2::splat(r * 1.55),
            [206, 72, 56],
            0.43,
        );
        let colors = [
            [244, 78, 70],
            [246, 110, 106],
            [180, 78, 190],
            [255, 174, 119],
        ];
        for j in 0..5 {
            for i in 0..100 {
                let a = TAU * i as f32 / 100.0;
                let k = a + t * 0.24 + j as f32 * 0.53;
                let hot = ((1.0 + (a + 0.68 - t * 0.12).cos()) * 0.5).powi(4);
                let rad = r * (1.025 + j as f32 * 0.035 + 0.013 * (k * 7.0 + j as f32).sin());
                let start = center + Vec2::angled(a) * rad;
                let len = r * (0.02 + 0.26 * hot) * rand(i * 11 + j + 1);
                let mid = start + Vec2::angled(a + 0.32) * (len * 0.60);
                let end = start + Vec2::angled(a + 0.59) * len;
                let pts = (0..=4)
                    .map(|k| {
                        let q = k as f32 / 4.0;
                        start.lerp(mid, q).lerp(mid.lerp(end, q), q)
                    })
                    .collect();
                let c = color(colors[j % colors.len()], (0.045 + hot * 0.27) * 1.385);
                self.painter
                    .add(Shape::line(pts, Stroke::new(0.8 + j as f32 * 0.25, c)));
            }
        }
        self.painter
            .circle_filled(center, r, Color32::from_rgb(4, 3, 6));
        self.glow(
            center + Vec2::new(-r * 0.20, r * 0.22),
            Vec2::splat(r * 0.69),
            [26, 18, 38],
            0.80,
        );
        let mut rim = Mesh::default();
        let width = (r * 0.018).max(1.0);
        const SIDES: u32 = 112;
        for i in 0..=SIDES {
            let axis = Vec2::angled(TAU * i as f32 / SIDES as f32);
            let p = (axis.x - axis.y + 2.0) * 0.25;
            let fill = if p < 0.38 {
                tint([80, 32, 76], [170, 61, 120], p / 0.38)
            } else if p < 0.76 {
                tint([170, 61, 120], [255, 144, 110], (p - 0.38) / 0.38)
            } else {
                tint([255, 144, 110], [255, 229, 186], (p - 0.76) / 0.24)
            };
            for (f, alpha) in [(-0.85, 0.0), (-0.45, 1.0), (0.45, 1.0), (0.85, 0.0)] {
                rim.colored_vertex(center + axis * (r + width * f), color(fill, alpha));
            }
            if i > 0 {
                connect_rows(&mut rim, (i - 1) * 4, i * 4, 4);
            }
        }
        self.mesh(rim);
        self.glow(
            center + Vec2::new(r * 0.71, -r * 0.71),
            Vec2::splat(r * 0.42),
            [255, 162, 116],
            0.77,
        );
        self.brand(center, (r * 1.21).min(self.w * 0.23), [227, 190, 214], 0.65);
    }

    fn transit(&self, t: f32) {
        self.ground([8, 5, 26], [8, 5, 13]);
        let y0 = self.h * 0.46;
        let cx = self.w * (0.52 + 0.04 * (t * 0.2).sin());
        self.glow(
            self.pos(cx, y0),
            Vec2::splat(self.h * 0.58),
            [203, 37, 107],
            0.46,
        );
        self.stars(t, 76, 0.47);
        let mut skyline = Mesh::default();
        for i in 0..60 {
            let x = i as f32 * self.w / 60.0;
            let w = self.w / 59.0;
            let h = (0.014 + 0.101 * rand(i * 7)) * self.h;
            let low = color([13, 9, 28], 1.0);
            let high = color(tint([13, 9, 28], [36, 17, 59], h / y0), 1.0);
            quad(
                &mut skyline,
                Rect::from_min_max(self.pos(x, y0 - h), self.pos(x + w * 0.91, y0)),
                [high, high, low, low],
            );
            for j in 0..4 {
                if rand(i * 17 + j) > 0.63 {
                    skyline.add_colored_rect(
                        Rect::from_min_size(
                            self.pos(x + w * 0.25, y0 - h * (0.26 + j as f32 * 0.12)),
                            Vec2::new((w * 0.11).max(1.0), 1.4),
                        ),
                        color(
                            if i % 3 == 0 {
                                [255, 88, 147]
                            } else {
                                [90, 144, 181]
                            },
                            0.36,
                        ),
                    );
                }
            }
        }
        self.mesh(skyline);
        let road_pos = |lane: f32, z: f32| {
            self.pos(
                cx + self.w * (lane * 0.77 * z + 0.18 * z * z * (t * 0.22 + 0.62 + z * 1.6).sin()),
                y0 + z * z * self.h * 0.61,
            )
        };
        let mut road = Mesh::default();
        for i in 0..=40 {
            let z = i as f32 / 40.0;
            let fill = color(tint([18, 11, 25], [25, 18, 30], z * z), 1.0);
            road.colored_vertex(road_pos(-0.87, z), fill);
            road.colored_vertex(road_pos(0.87, z), fill);
            if i > 0 {
                connect_rows(&mut road, (i - 1) * 2, i * 2, 2);
            }
        }
        self.mesh(road);
        for lane in [-0.85, -0.4, 0.0, 0.4, 0.85] {
            let points = (0..=40).map(|j| road_pos(lane, j as f32 / 40.0)).collect();
            let (w, fill) = if lane < 0.0 {
                (1.6, [117, 54, 78])
            } else if lane > 0.0 {
                (1.6, [49, 87, 109])
            } else {
                (1.1, [75, 54, 74])
            };
            self.painter
                .add(Shape::line(points, Stroke::new(w, color(fill, 1.0))));
        }
        const SPEED: f32 = 0.47;
        for i in 0..110 {
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let lane = side * (0.12 + 0.58 * rand(i * 5 + 2));
            let z = (rand(i * 5) + t * (0.064 + SPEED * 0.13) * (0.65 + rand(i * 5 + 1) * 0.51))
                .fract();
            let tail = (z - (0.034 + SPEED * 0.13) * (0.55 + z) * rand(i * 5 + 3)).max(0.0);
            let c = if lane < 0.0 {
                if i % 3 == 0 {
                    [255, 178, 96]
                } else {
                    [255, 53, 113]
                }
            } else if i % 3 == 0 {
                [151, 246, 255]
            } else {
                [63, 161, 214]
            };
            let points: Vec<_> = (0..=7)
                .map(|j| road_pos(lane, mix(tail, z, j as f32 / 7.0)))
                .collect();
            let a = (z * 2.5).min(1.0);
            let w = 0.25 + z * 2.3;
            self.painter.add(Shape::line(
                points.clone(),
                Stroke::new(w * 4.4, color(c, 0.20 * a)),
            ));
            self.painter
                .add(Shape::line(points, Stroke::new(w, color(c, 0.77 * a))));
        }
        self.brand(
            self.at(0.5, 0.23),
            (self.w * 0.25).min(262.0),
            [244, 175, 203],
            0.73,
        );
    }
}

fn streak(mesh: &mut Mesh, head: Pos2, base: f32, length: f32, width: f32, a: f32) {
    if a < 0.015 || head.y >= base {
        return;
    }
    let length = length.min(base - head.y);
    if width > 1.0 {
        mesh.add_colored_rect(
            Rect::from_center_size(head + Vec2::new(0.0, 1.0), Vec2::new(width * 3.0, 4.0)),
            color([248, 199, 255], a * 0.13),
        );
    }
    let stops = [
        (0.0, color([255, 244, 255], a)),
        (0.1, color([255, 169, 246], a * 0.78)),
        (0.37, color([203, 93, 236], a * 0.47)),
        (1.0, Color32::TRANSPARENT),
    ];
    let start = mesh.vertices.len() as u32;
    for (i, (f, fill)) in stops.into_iter().enumerate() {
        let y = head.y + length * f;
        mesh.colored_vertex(Pos2::new(head.x - width * 0.5, y), fill);
        mesh.colored_vertex(Pos2::new(head.x + width * 0.5, y), fill);
        if i > 0 {
            let n = start + i as u32 * 2;
            connect_rows(mesh, n - 2, n, 2);
        }
    }
}

impl Scene {
    fn rising(&self, elapsed: f32, reduced: bool) {
        const DENSITY: f32 = 0.57;
        let t = if reduced { 7.0 } else { elapsed * 1.28 };
        self.ground([7, 6, 11], [9, 6, 16]);
        let horizon = self.h * 0.895;
        let center = self.w * 0.5;
        let reveal = (0.25 + t * 0.7).min(1.0);
        let strength = 0.44 + DENSITY * 0.54;
        self.mesh(radial(
            self.pos(center, horizon),
            Vec2::new(self.w * 0.52, self.h * 0.29),
            &[
                (0.0, color([171, 65, 235], strength * 0.68)),
                (0.27, color([134, 40, 204], strength * 0.29)),
                (0.67, color([77, 32, 150], strength * 0.09)),
                (1.0, Color32::TRANSPARENT),
            ],
        ));
        let half = self.w * 0.47 * reveal;
        let mut beam = Mesh::default();
        let beam_height = (self.h * 0.0036).max(1.0);
        let stops = [
            (0.0, Color32::TRANSPARENT),
            (0.20, color([121, 57, 208], 0.35)),
            (0.5, color([245, 196, 255], 1.0)),
            (0.8, color([121, 57, 208], 0.35)),
            (1.0, Color32::TRANSPARENT),
        ];
        for (i, (f, c)) in stops.into_iter().enumerate() {
            let x = center - half + f * half * 2.0;
            beam.colored_vertex(self.pos(x, horizon), c);
            beam.colored_vertex(self.pos(x, horizon + beam_height), c);
            if i > 0 {
                let i = i as u32 * 2;
                connect_rows(&mut beam, i - 2, i, 2);
            }
        }
        self.mesh(beam);

        let mut particles = Mesh::default();
        let total = ((95.0 + 135.0 * DENSITY) * (self.w / 670.0).min(1.0)).round() as usize;
        for i in 0..total {
            let u = (rand(i * 11) - 0.5) * 0.98;
            let near = rand(i * 11 + 1);
            let tall = rand(i * 11 + 3);
            let x = center + u * self.w * reveal;
            let life = (rand(i * 11 + 2) + t * (0.076 + near * 0.098)).fract();
            let base = horizon - self.h * (1.0 - near) * 0.026;
            let head = base - self.h * (0.10 + 0.81 * tall.powf(2.35)) * life;
            let shade = (1.0 - u.abs() * 1.42) * (0.15 + near * 0.81);
            let fade = (life * 10.0).min((1.0 - life) * 5.0).min(1.0);
            streak(
                &mut particles,
                self.pos(x, head),
                self.rect.min.y + base,
                self.h * (0.024 + near * 0.33 * tall),
                0.30 + near.powi(3) * 1.42,
                fade * shade * (0.48 + DENSITY * 0.52),
            );
        }
        for i in 0..72 {
            let u = (rand(i * 7 + 2) - 0.5) * 0.88;
            let x = center + u * self.w * reveal;
            let s = (rand(i * 7 + 3) + t * (0.08 + rand(i * 7 + 4) * 0.20)).fract();
            let y = horizon - s * self.h * (0.04 + rand(i * 7 + 5).powf(2.6) * 0.61);
            let a = (1.0 - u.abs() * 2.0) * (s * PI).sin() * strength * 0.56;
            dot(
                &mut particles,
                self.pos(x, y),
                0.36 + rand(i * 7 + 6) * 0.86,
                color([217, 126, 248], a),
            );
        }

        // Each point samples the boot mark. Launch from the horizon, hold a legible
        // mark, then release independently before starting the next cycle.
        let clock = t % 14.0;
        let width = (self.w * 0.74).min(self.h * 1.32).min(610.0);
        let height = width * MARK_ASPECT;
        let left = (self.w - width) * 0.5;
        let top = self.h * 0.42 - height * 0.5;
        for q in mark_points() {
            let start = 1.05 + q.u * 1.28 + q.seed * 0.68;
            let travel = (clock - start) / (1.44 + q.fuzz * 0.94);
            let release = (clock - 9.4 - q.seed * 0.84 - q.u * 0.48) / (1.85 + q.fuzz * 0.52);
            if travel <= 0.0 || release >= 1.0 {
                continue;
            }
            let toward = 1.0 - (1.0 - travel.clamp(0.0, 1.0)).powf(2.35);
            let depart = smooth(0.0, 1.0, release);
            let tx = left + q.u * width;
            let ty = top + q.v * height;
            let spread = (q.seed - 0.5) * self.w * 0.15;
            let x = mix(tx + spread, tx, toward) + (q.fuzz - 0.5) * self.w * 0.12 * depart;
            let y = mix(horizon, ty, toward) - depart * self.h * (0.18 + q.seed * 0.51);
            let fade = smooth(0.0, 1.0, travel * 5.0) * (1.0 - depart);
            let tail = if travel < 1.0 {
                (1.0 - toward) * self.h * (0.032 + q.seed * 0.115)
            } else {
                depart * self.h * 0.08
            };
            if tail > 0.7 {
                let c = if q.fuzz > 0.5 {
                    [189, 104, 239]
                } else {
                    [242, 165, 227]
                };
                particles.add_colored_rect(
                    Rect::from_min_max(
                        self.pos(x - 0.42, y),
                        self.pos(x + 0.42, (y + tail).min(horizon)),
                    ),
                    color(c, fade * 0.36),
                );
            }
            let c = if q.seed > 0.63 {
                [255, 228, 255]
            } else if q.fuzz > 0.42 {
                [236, 164, 236]
            } else {
                [169, 124, 230]
            };
            let radius = (width / MARK_W as f32 * (1.13 + q.seed * 0.85)).max(0.48);
            dot(
                &mut particles,
                self.pos(x, y),
                radius,
                color(c, fade * (0.58 + q.seed * 0.42)),
            );
        }
        self.mesh(particles);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const SCENES: [Saver; 7] = [
        Saver::SilkInk,
        Saver::SignalContour,
        Saver::TerminalWeave,
        Saver::NightTransit,
        Saver::RisingLines,
        Saver::EventHorizon,
        Saver::PolarVeil,
    ];

    fn frame(
        ctx: &egui::Context,
        style: Saver,
        seconds: u64,
        reduced: bool,
        size: [f32; 2],
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
                ..Default::default()
            },
            |ctx| show(ctx, style, Duration::from_secs(seconds), reduced, false),
        )
    }

    #[test]
    fn native_scenes_cover_the_viewport_and_keep_geometry_bounded() {
        let ctx = egui::Context::default();
        for size in [[320.0, 640.0], [3840.0, 2160.0]] {
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
            for style in SCENES {
                let output = frame(&ctx, style, 3, false, size);
                let Some(egui::epaint::ClippedShape {
                    shape: Shape::Mesh(background),
                    ..
                }) = output.shapes.first()
                else {
                    panic!("{style:?} is missing its opaque background");
                };
                assert_eq!(background.calc_bounds(), viewport);
                assert!(
                    background
                        .vertices
                        .iter()
                        .all(|vertex| vertex.color.is_opaque())
                );
                assert!(
                    output
                        .shapes
                        .iter()
                        .all(|shape| shape.clip_rect == viewport),
                    "{style:?} is not clipped to the viewport"
                );
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                let mut vertices = 0;
                for primitive in primitives {
                    let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                        panic!("{style:?} needs a custom renderer");
                    };
                    assert!(
                        mesh.is_valid() && mesh.vertices.iter().all(|v| v.pos.is_finite()),
                        "{style:?} paints invalid geometry"
                    );
                    vertices += mesh.vertices.len();
                }
                assert!(
                    (500..180_000).contains(&vertices),
                    "{style:?} draws {vertices} vertices"
                );
            }
        }
    }

    #[test]
    fn off_paints_no_saver() {
        let ctx = egui::Context::default();
        assert!(
            frame(&ctx, Saver::Off, 0, false, [800.0, 450.0])
                .shapes
                .is_empty()
        );
    }

    #[test]
    fn every_scene_moves_and_reduced_motion_freezes_its_composition() {
        let ctx = egui::Context::default();
        for style in SCENES {
            let size = [800.0, 450.0];
            assert_ne!(
                frame(&ctx, style, 1, false, size).shapes,
                frame(&ctx, style, 3, false, size).shapes,
                "{style:?} does not move"
            );
            assert_eq!(
                frame(&ctx, style, 1, true, size).shapes,
                frame(&ctx, style, 60, true, size).shapes,
                "{style:?} changes with reduced motion"
            );
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod native_gallery {
    use super::*;
    use winit::platform::x11::EventLoopBuilderExtX11;

    // Run on an isolated display:
    // xvfb-run -a -s '-screen 0 1600x1050x24' cargo test -p des-ui native_saver_gallery -- --ignored
    #[test]
    #[ignore = "requires an X11 display and writes a visual review image"]
    fn native_saver_gallery() {
        struct Gallery {
            frame: usize,
            wrote_screenshot: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }

        impl eframe::App for Gallery {
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                let screenshot = ctx.input(|i| {
                    i.events.iter().find_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                });
                if let Some(image) = screenshot {
                    let path = std::env::var_os("DES_UI_SAVER_GALLERY")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::temp_dir().join("des-ui-native-savers.png"));
                    let bytes: Vec<_> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    self.wrote_screenshot
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                let stage = ctx.screen_rect();
                let p = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Background,
                    egui::Id::new("gallery"),
                ));
                p.rect_filled(stage, 0.0, Color32::from_rgb(15, 12, 19));
                let samples = [
                    (Saver::SilkInk, "Silk Fold · Ink", 3.0),
                    (Saver::SignalContour, "Signal Contour", 6.0),
                    (Saver::TerminalWeave, "Terminal Weave", 6.0),
                    (Saver::NightTransit, "Night Transit", 3.4),
                    (Saver::EventHorizon, "Event Horizon", 2.6),
                    (Saver::PolarVeil, "Polar Veil", 4.2),
                    (Saver::RisingLines, "Rising Lines · 2 s", 2.0),
                    (Saver::RisingLines, "Rising Lines · 4.6 s", 4.6),
                    (Saver::RisingLines, "Rising Lines · 8.4 s", 8.4),
                ];
                let gutter = 12.0;
                let cell = (stage.size() - Vec2::splat(gutter * 4.0)) / 3.0;
                for (i, (style, label, t)) in samples.into_iter().enumerate() {
                    let origin = stage.min
                        + Vec2::new(
                            gutter + (i % 3) as f32 * (cell.x + gutter),
                            gutter + (i / 3) as f32 * (cell.y + gutter),
                        );
                    let art = Rect::from_min_size(origin, cell - Vec2::new(0.0, 23.0));
                    Scene {
                        painter: p.with_clip_rect(art),
                        rect: art,
                        w: art.width(),
                        h: art.height(),
                    }
                    .draw(style, t, false);
                    p.text(
                        art.left_bottom() + Vec2::new(0.0, 7.0),
                        Align2::LEFT_TOP,
                        label,
                        FontId::monospace(12.0),
                        Color32::from_rgb(224, 212, 232),
                    );
                }
                self.frame += 1;
                if self.frame == 4 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                assert!(self.frame < 60, "egui did not return a native screenshot");
                ctx.request_repaint_after(Duration::from_millis(20));
            }
        }

        let written = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture = written.clone();
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]),
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_x11().with_any_thread(true);
            })),
            ..Default::default()
        };
        eframe::run_native(
            "Native DEs-UI saver gallery",
            options,
            Box::new(move |cc| {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "saver-mono".into(),
                    egui::FontData::from_static(include_bytes!(
                        "../assets/fonts/JetBrainsMono.ttf"
                    ))
                    .into(),
                );
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "saver-mono".into());
                cc.egui_ctx.set_fonts(fonts);
                Ok(Box::new(Gallery {
                    frame: 0,
                    wrote_screenshot: capture,
                }))
            }),
        )
        .unwrap();
        assert!(written.load(std::sync::atomic::Ordering::Relaxed));
    }
}
