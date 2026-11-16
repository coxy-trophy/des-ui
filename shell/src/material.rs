//! Shared wallpaper-backed surface materials. Foreground widgets retain ownership
//! of text, layout and input; this module only paints their background.
use eframe::egui::{self, Color32, Rect, Shape};
use des_common::appearance::{AppearanceSettings, SurfaceStyle};
use crate::{ui, wallpaper::{WallpaperView, geometry}};

#[derive(Clone)]
struct State { appearance: AppearanceSettings, wallpaper: Option<WallpaperView> }
fn key() -> egui::Id { egui::Id::new("des-ui-materials") }
pub fn publish(ctx: &egui::Context, appearance: AppearanceSettings, wallpaper: Option<WallpaperView>) {
    ctx.data_mut(|data| data.insert_temp(key(), State { appearance, wallpaper }));
}
/// The wallpaper as published for the surfaces, and how it is fitted, if any.
pub fn wallpaper(ctx: &egui::Context) -> Option<(WallpaperView, des_common::appearance::WallpaperFit)> {
    let state = ctx.data(|data| data.get_temp::<State>(key()))?;
    Some((state.wallpaper?, state.appearance.fit))
}
#[derive(Clone, Copy)]
pub enum Role { Background, Panel, Terminal, Floating, Notification }
pub fn opacity(settings: &AppearanceSettings, role: Role) -> f32 {
    if settings.effective_style(false) == SurfaceStyle::Solid { return 1.0; }
    (settings.opacity / 100.0).max(match role { Role::Terminal => 0.35, Role::Notification => 0.9, _ => 0.75 })
}
fn contrast(foreground: [u8; 3], background: [u8; 3]) -> f32 {
    let luminance = |rgb: [u8; 3]| rgb.into_iter().zip([0.2126, 0.7152, 0.0722]).map(|(channel, weight)| {
        let value = channel as f32 / 255.0;
        weight * if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    }).sum::<f32>();
    (luminance(foreground) + 0.05) / (luminance(background) + 0.05)
}
fn readable_opacity(settings: &AppearanceSettings, role: Role, peak: [u8; 3]) -> f32 {
    let base = match role { Role::Terminal | Role::Background => ui::GROUND, _ => ui::PANEL };
    let mut low = opacity(settings, role); let mut high = 1.0;
    let readable = |alpha: f32| {
        let base = [base.r(), base.g(), base.b()];
        let background = std::array::from_fn(|i| (base[i] as f32 * alpha + peak[i] as f32 * (1.0 - settings.dim / 100.0) * (1.0 - alpha)).ceil() as u8);
        // Terminal glyphs use the bright terminal palette; applying the faint
        // chrome-label floor here made the entire workspace effectively opaque.
        let ink = if matches!(role, Role::Terminal) { [180, 192, 204] }
            else { [ui::LABEL.r(), ui::LABEL.g(), ui::LABEL.b()] };
        contrast(ink, background) >= 4.5
    };
    if readable(low) { return low; }
    for _ in 0..10 { let mid = (low + high) / 2.0; if readable(mid) { high = mid; } else { low = mid; } }
    (high * 255.0).ceil() / 255.0
}

/// The rounded shapes floating surfaces were painted with this frame, so that the
/// compositor can show applications past their corners where they are lifted above
/// them (`Command::Rounded`).
fn rounded_key() -> egui::Id { egui::Id::new("material-rounded-shapes") }

/// Notes a rounded surface painted this frame.
pub fn record(ctx: &egui::Context, rect: Rect, radius: f32) {
    if radius > 0.0 && rect.is_positive() {
        ctx.data_mut(|data| data.get_temp_mut_or_default::<Vec<(Rect, f32)>>(rounded_key()).push((rect, radius)));
    }
}

/// The rounded surfaces painted since this was last called.
pub fn take_recorded(ctx: &egui::Context) -> Vec<(Rect, f32)> {
    ctx.data_mut(|data| std::mem::take(data.get_temp_mut_or_default::<Vec<(Rect, f32)>>(rounded_key())))
}

/// What to tell the compositor of rounded surfaces in the areas lifted above the
/// applications. An area counts only when the rounded surfaces inside it reach its
/// edges (within `SLACK`, the margin areas are padded with): then nothing else is
/// in it and it can be drawn as those surfaces alone. Any other area, a full-screen
/// modal, say, or one with something unrounded beside the surfaces, is left out and
/// drawn whole. In whole pixels, rounded outward.
pub fn rounded_shapes(areas: &[Rect], recorded: &[(Rect, f32)]) -> Vec<(i32, i32, i32, i32, i32)> {
    const SLACK: f32 = 12.0;
    let mut out = Vec::new();
    for area in areas {
        let inside: Vec<&(Rect, f32)> = recorded.iter().filter(|(rect, _)| area.expand(0.5).contains_rect(*rect)).collect();
        let Some(reach) = inside.iter().map(|(rect, _)| *rect).reduce(|all, rect| all.union(rect)) else { continue };
        if !reach.expand(SLACK).contains_rect(*area) {
            continue;
        }
        for (rect, radius) in inside {
            let (left, top) = (rect.left().floor() as i32, rect.top().floor() as i32);
            let shape = (left, top, rect.right().ceil() as i32 - left, rect.bottom().ceil() as i32 - top, radius.round() as i32);
            if !out.contains(&shape) {
                out.push(shape);
            }
        }
    }
    out.truncate(des_common::ipc::MAX_ROUNDED);
    out
}

pub fn shapes(ctx: &egui::Context, rect: Rect, radius: f32, role: Role) -> Vec<Shape> {
    if matches!(role, Role::Floating | Role::Notification) {
        record(ctx, rect, radius);
    }
    let state = ctx.data(|data| data.get_temp::<State>(key()));
    let Some(state) = state else { return vec![Shape::rect_filled(rect, radius, match role { Role::Terminal | Role::Background => ui::GROUND, _ => ui::PANEL })]; };
    shapes_for(ctx.screen_rect(), rect, radius, role, &state)
}
fn shapes_for(screen: Rect, rect: Rect, radius: f32, role: Role, state: &State) -> Vec<Shape> {
    let base = match role { Role::Terminal | Role::Background => ui::GROUND, _ => ui::PANEL };
    let mut shapes = vec![Shape::rect_filled(rect, radius, base)];
    let background = matches!(role, Role::Background);
    if !background && state.appearance.effective_style(false) == SurfaceStyle::Solid { return shapes; }
    let Some(wallpaper) = &state.wallpaper else { return shapes; };
    let texture = wallpaper.texture(state.appearance.blur > 0.0);
    let (dest, uv) = geometry(screen, texture.size_vec2(), state.appearance.fit);
    let visible = rect.intersect(dest);
    if visible.is_positive() {
        let local_min = (visible.min - dest.min) / dest.size();
        let local_max = (visible.max - dest.min) / dest.size();
        let sample = Rect::from_min_max(uv.min + local_min * uv.size(), uv.min + local_max * uv.size());
        shapes.push(egui::epaint::RectShape::filled(visible, radius, Color32::WHITE).with_texture(texture.id(), sample).into());
        shapes.push(Shape::rect_filled(visible, radius, Color32::from_black_alpha((state.appearance.dim * 2.55).round() as u8)));
    }
    if !background {
        let alpha = (readable_opacity(&state.appearance, role, wallpaper.peak()) * 255.0).ceil().min(255.0) as u8;
        shapes.push(Shape::rect_filled(rect, radius, Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)));
    }
    shapes
}
pub fn paint(ctx: &egui::Context, painter: &egui::Painter, rect: Rect, radius: f32, role: Role) {
    painter.add(Shape::Vec(shapes(ctx, rect, radius, role)));
}

/// Paint a content-sized native frame with the shared backdrop, behind its widgets.
/// The frame retains native menu/modal geometry and clipping ownership.
pub fn surface<R>(ui: &mut egui::Ui, frame: egui::Frame, role: Role, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let background = ui.painter().add(Shape::Noop);
    let radius = frame.corner_radius.nw as f32;
    let response = frame.fill(Color32::TRANSPARENT).show(ui, add);
    ui.painter().set(background, Shape::Vec(shapes(ui.ctx(), response.response.rect, radius, role)));
    response
}

/// The editor uses the exact desktop painter with a preview-local screen origin.
pub fn preview(painter: &egui::Painter, rect: Rect, appearance: &AppearanceSettings, wallpaper: Option<&WallpaperView>) {
    let state = State { appearance: appearance.clone(), wallpaper: wallpaper.cloned() };
    painter.add(Shape::Vec(shapes_for(rect, rect, 0.0, Role::Background, &state)));
    painter.add(Shape::Vec(shapes_for(rect, rect.shrink(16.0), 0.0, Role::Terminal, &state)));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wallpaper_terminal_keeps_visible_depth_instead_of_forcing_ninety_percent_ink() {
        let settings = AppearanceSettings { style: SurfaceStyle::Frosted, opacity: 55.0, dim: 20.0, ..Default::default() };
        let alpha = readable_opacity(&settings, Role::Terminal, [255; 3]);
        assert!(alpha < 0.7, "wallpaper is buried under the terminal: {alpha}");
        let background = [6, 8, 11].map(|v| (v as f32 * alpha + 204.0 * (1.0 - alpha)).ceil() as u8);
        assert!(contrast([180, 192, 204], background) >= 4.5);
        let opaque = AppearanceSettings { reduce_transparency: true, ..settings };
        assert_eq!(opacity(&opaque, Role::Terminal), 1.0);
    }
    #[test]
    fn material_small_labels_remain_readable_over_white_wallpaper() {
        let mut settings = AppearanceSettings::default();
        settings.style = SurfaceStyle::Frosted;
        settings.dim = 0.0; settings.opacity = 75.0;
        let alpha = readable_opacity(&settings, Role::Panel, [255; 3]);
        let background = [10, 14, 18].map(|channel| (channel as f32 * alpha + 255.0 * (1.0 - alpha)).round() as u8);
        assert!(contrast([ui::LABEL.r(), ui::LABEL.g(), ui::LABEL.b()], background) >= 4.5);
        assert_eq!(readable_opacity(&settings, Role::Panel, [0; 3]), 0.75);
    }
    #[test]
    fn material_readability_and_reduced_transparency() {
        let mut settings = AppearanceSettings::default();
        settings.style = SurfaceStyle::Frosted;
        settings.opacity = 75.0;
        assert_eq!(opacity(&settings, Role::Panel), 0.75);
        assert_eq!(opacity(&settings, Role::Terminal), 0.75);
        assert_eq!(opacity(&settings, Role::Notification), 0.9);
        settings.reduce_transparency = true;
        assert_eq!(opacity(&settings, Role::Floating), 1.0);
    }
    #[test]
    fn material_without_image_stays_opaque_and_does_not_schedule_motion() {
        let ctx = egui::Context::default();
        publish(&ctx, AppearanceSettings::default(), None);
        assert_eq!(shapes(&ctx, Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 20.0)), 0.0, Role::Panel).len(), 1);
    }
}

#[cfg(test)]
mod rounded_tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect { Rect::from_min_size(pos2(x, y), vec2(w, h)) }

    #[test]
    fn a_lifted_area_made_of_rounded_surfaces_is_sent_as_them() {
        // Two toasts, the area around them padded by a pixel or so.
        let toasts = [(rect(1500.0, 52.0, 360.0, 96.0), 14.0), (rect(1500.0, 156.0, 360.0, 80.0), 14.0)];
        let area = rect(1499.0, 51.0, 362.0, 186.0);
        assert_eq!(rounded_shapes(&[area], &toasts), vec![(1500, 52, 360, 96, 14), (1500, 156, 360, 80, 14)]);
        // The quick settings pop-up, fractional, rounded outward.
        let quick = [(rect(1100.4, 600.6, 300.2, 380.0), 10.0)];
        assert_eq!(rounded_shapes(&[rect(1100.0, 600.0, 302.0, 382.0)], &quick), vec![(1100, 600, 301, 381, 10)]);
    }

    #[test]
    fn an_area_with_more_than_its_rounded_surfaces_is_left_whole() {
        // A full-screen modal with a card in it: the card does not reach the edges.
        let card = [(rect(700.0, 300.0, 500.0, 400.0), 10.0)];
        assert!(rounded_shapes(&[rect(0.0, 0.0, 1920.0, 1080.0)], &card).is_empty());
        // Toasts with the "+2 more" pill below them: the pill is not a recorded surface.
        let toasts = [(rect(1500.0, 52.0, 360.0, 96.0), 14.0)];
        assert!(rounded_shapes(&[rect(1499.0, 51.0, 362.0, 150.0)], &toasts).is_empty());
        // Nothing rounded at all, or a surface outside every lifted area.
        assert!(rounded_shapes(&[rect(0.0, 0.0, 100.0, 100.0)], &[]).is_empty());
        assert!(rounded_shapes(&[rect(0.0, 0.0, 100.0, 100.0)], &[(rect(400.0, 400.0, 50.0, 50.0), 10.0)]).is_empty());
    }

    #[test]
    fn the_dock_margin_is_within_the_slack() {
        let popup = [(rect(80.0, 500.0, 292.0, 200.0), 6.0)];
        let area = rect(70.0, 490.0, 313.0, 221.0); // the popup's content expanded by 10
        assert_eq!(rounded_shapes(&[area], &popup).len(), 1);
    }
}
