//! What goes around the compositor's pictures of the windows, in the overview and in
//! the window switcher: the ground, each workspace's name, the windows' names, the
//! selection, and what has been typed. The compositor draws the pictures themselves on
//! top, in the places it said, and takes every key and click while either is open, so
//! nothing here is interactive.
//!
//! Only the main display's window can be drawn into; on the others the compositor
//! shows the pictures on a plain ground without names.

use des_common::ipc::{Area, DesktopState, OverviewKind, OverviewLayout};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Order, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::{theme::Theme, ui};

fn rect((x, y, w, h): Area) -> Rect {
    Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32))
}

/// `text` cut to fit `width` at `font`, with an ellipsis when it had to be.
pub fn fitted(painter: &egui::Painter, text: &str, font: &FontId, width: f32) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let fits = |text: &str| painter.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x <= width;
    if fits(&text) {
        return text;
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&candidate) { low = mid } else { high = mid - 1 }
    }
    chars[..low].iter().collect::<String>() + "…"
}

/// The hint along the top of the overview, or what has been typed.
pub fn heading(layout: &OverviewLayout) -> String {
    if layout.kind==OverviewKind::Drag {
        return "HOLD OVER A WINDOW TO OPEN IT · THEN DROP INSIDE · ESC CANCELS".into();
    }
    if layout.kind==OverviewKind::Windows {
        return format!("OPEN WINDOWS · {} · PGUP/PGDN · ESC RETURNS TO DOCK",layout.filter);
    }
    if layout.filter.is_empty() {
        "TYPE TO FIND A WINDOW · ENTER OPENS · DRAG TO ANOTHER WORKSPACE · ESC CLOSES".to_string()
    } else {
        let found = layout.thumbs.len();
        format!("{} · {found} {}", layout.filter.to_uppercase(), if found == 1 { "WINDOW" } else { "WINDOWS" })
    }
}

/// Draws around the pictures of whatever the compositor has open, on the main display.
pub fn show(ctx: &egui::Context, theme: &Theme, layout: &OverviewLayout, state: Option<&DesktopState>) {
    show_on(ctx,theme,layout,state,0);
}

pub fn show_on(ctx:&egui::Context,theme:&Theme,layout:&OverviewLayout,state:Option<&DesktopState>,display:usize) {
    if layout.screen != display {
        return;
    }
    let screen = ctx.screen_rect();
    let title_of = |app: usize| state.and_then(|state| state.apps.get(app)).map_or(String::new(), |app| app.title.clone());
    // The compositor owns input and geometry. Paint directly so the overview is
    // present at full opacity on its first frame, without Area's sizing/fade-in.
    // Frame outlines have their own Foreground layer; this presentation must sit
    // above that chrome even after those layers have been reordered.
    let layer = egui::LayerId::new(Order::Tooltip, egui::Id::new("overview-surround"));
    ctx.move_to_top(layer);
    let painter = ctx.layer_painter(layer).with_clip_rect(screen);
    let accent = theme.main;
    match layout.kind {
        OverviewKind::Workspaces | OverviewKind::Drag => {
            crate::material::paint(ctx,&painter,screen,0.0,crate::material::Role::Floating);
            painter.text(pos2(screen.left() + 48.0, screen.top() + 36.0), Align2::LEFT_CENTER, if layout.kind==OverviewKind::Drag {"DROP INTO A WINDOW"}else{"OVERVIEW"}, ui::display(18.0), ui::TEXT);
            let hint = heading(layout);
            let colour = if layout.filter.is_empty() { ui::LABEL } else { accent };
            if screen.width()>1100.0 {
                painter.text(pos2(screen.right() - 48.0, screen.top() + 36.0), Align2::RIGHT_CENTER, fitted(&painter,&hint,&ui::mono(11.0),screen.width()-330.0), ui::mono(11.0), colour);
            } else {
                painter.text(pos2(screen.left()+48.0,screen.top()+57.0),Align2::LEFT_CENTER,
                    fitted(&painter,&hint,&ui::mono(10.0),screen.width()-96.0),ui::mono(10.0),ui::SECONDARY);
            }
            for workspace in &layout.workspaces {
                let cell = rect(workspace.area);
                let line = if workspace.current { accent } else { ui::LINE };
                painter.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, line), StrokeKind::Inside);
                let count = layout.thumbs.iter().filter(|thumb| cell.contains(rect(thumb.area).center())).count();
                let name = format!("{}{}", workspace.workspace + 1, if workspace.current { " · IN VIEW" } else { "" });
                painter.text(cell.left_top() + vec2(14.0, 20.0), Align2::LEFT_CENTER, name, ui::display(14.0), if workspace.current { accent } else { ui::TEXT });
                let windows = match count {
                    0 => "EMPTY".to_string(),
                    1 => "1 WINDOW".to_string(),
                    n => format!("{n} WINDOWS"),
                };
                painter.text(cell.right_top() + vec2(-14.0, 20.0), Align2::RIGHT_CENTER, windows, ui::mono(11.0), ui::LABEL);
                painter.hline(cell.x_range(), cell.top() + 40.0, Stroke::new(1.0, ui::LINE));
            }
        }
        OverviewKind::Switcher | OverviewKind::Windows => {
            if let Some(strip) = layout.strip.map(rect) {
                crate::material::paint(ctx,&painter,strip,ui::SURFACE_RADIUS,crate::material::Role::Floating);
                painter.rect_stroke(strip, ui::SURFACE_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
                if layout.kind==OverviewKind::Windows {
                    let hint=heading(layout);
                    painter.text(strip.left_top()+vec2(16.0,16.0),Align2::LEFT_CENTER,
                        fitted(&painter,&hint,&ui::display(12.0),strip.width()-32.0),ui::display(12.0),ui::SECONDARY);
                }
            }
        }
    }
    for (index, thumb) in layout.thumbs.iter().enumerate() {
        let area = rect(thumb.area);
        let selected = layout.selected == Some(index);
        // Beneath the picture, for a window that has not drawn itself yet.
        painter.rect_filled(area, CornerRadius::ZERO, ui::TRACK);
        if selected {
            painter.rect_stroke(area.expand(4.0), CornerRadius::same(3), Stroke::new(2.0, accent), StrokeKind::Outside);
        } else {
            painter.rect_stroke(area, CornerRadius::ZERO, Stroke::new(1.0, ui::LINE), StrokeKind::Outside);
        }
        let font = ui::mono(11.0);
        let width = area.width().max(40.0);
        let title=title_of(thumb.app);
        let title=if layout.kind==OverviewKind::Windows {
            state.and_then(|s|s.apps.get(thumb.app)).map_or(title.clone(),|app|format!("{} · W{}",title,app.workspace+1))
        }else{title};
        let name = fitted(&painter, &title, &font, width);
        if selected && layout.kind==OverviewKind::Drag {
            let progress=layout.filter.parse::<f32>().unwrap_or(0.0).clamp(0.0,100.0)/100.0;
            painter.hline(area.left()..=(area.left()+area.width()*progress),area.bottom()+3.0,Stroke::new(3.0,accent));
        }
        let at: Pos2 = pos2(area.center().x, area.bottom() + 14.0);
        painter.text(at, Align2::CENTER_CENTER, name, font, if selected { ui::TEXT } else { ui::SECONDARY });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use des_common::ipc::{OverviewThumb, OverviewWorkspace};

    fn layout(filter: &str, thumbs: usize) -> OverviewLayout {
        OverviewLayout {
            kind: OverviewKind::Workspaces,
            screen: 0,
            selected: (thumbs > 0).then_some(0),
            filter: filter.into(),
            strip: None,
            workspaces: (0..4).map(|w| OverviewWorkspace { workspace: w, area: (48 + w as i32 * 300, 72, 280, 600), current: w == 0 }).collect(),
            thumbs: (0..thumbs).map(|i| OverviewThumb { app: i, area: (60, 120 + i as i32 * 100, 200, 80) }).collect(),
        }
    }

    #[test]
    fn the_heading_hints_until_something_is_typed() {
        assert!(heading(&layout("", 2)).starts_with("TYPE TO FIND"));
        assert_eq!(heading(&layout("fire", 1)), "FIRE · 1 WINDOW");
        assert_eq!(heading(&layout("term", 3)), "TERM · 3 WINDOWS");
    }

    #[test]
    fn long_names_are_cut_to_fit_and_short_ones_kept() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let font = FontId::monospace(11.0);
            assert_eq!(fitted(&painter, "short", &font, 400.0), "short");
            let cut = fitted(&painter, &"a very long window title ".repeat(10), &font, 120.0);
            assert!(cut.ends_with('…'));
            assert!(painter.layout_no_wrap(cut, font.clone(), Color32::WHITE).size().x <= 120.0);
            assert_eq!(fitted(&painter, "tab\tand\nline", &font, 400.0), "tabandline");
        });
    }

    fn opaque_fill(shape: &egui::Shape, bounds: Rect) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| opaque_fill(shape, bounds)),
            egui::Shape::Rect(shape) => shape.rect == bounds && shape.fill.a() == 255,
            _ => false,
        }
    }

    #[test]
    fn overview_covers_frame_chrome_on_first_frame_reopen_and_resize() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        for kind in [OverviewKind::Workspaces, OverviewKind::Drag] {
            for size in [vec2(1280.0, 800.0), vec2(1600.0, 900.0)] {
                let screen = Rect::from_min_size(pos2(12.0, 24.0), size);
                let chrome = screen.shrink(8.0);
                let output = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                    // Even a raised frame layer must stay below the overview.
                    let layer = egui::LayerId::new(Order::Foreground, egui::Id::new("frame-lines"));
                    ctx.move_to_top(layer);
                    ctx.layer_painter(layer).rect_filled(chrome, 10.0, Color32::RED);
                    show(ctx, &theme, &OverviewLayout { kind, ..layout("", 0) }, None);
                });
                let ground = output.shapes.iter().position(|shape| opaque_fill(&shape.shape, screen))
                    .expect("overview needs an opaque full-screen base on its first frame");
                let frame = output.shapes.iter().position(|shape| opaque_fill(&shape.shape, chrome)).unwrap();
                assert!(ground > frame, "frame chrome must be painted beneath the overview");
                assert_eq!(output.shapes[ground].clip_rect, screen);
                // Closing and opening must not introduce a transparent fade.
                let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |_| {});
            }
        }
    }

    #[test]
    fn switcher_and_window_chooser_keep_their_background_inside_the_strip() {
        for kind in [OverviewKind::Switcher, OverviewKind::Windows] {
            let ctx = egui::Context::default();
            ui::install_fonts(&ctx, None, None);
            let theme = Theme::load("signal");
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
            let strip = (100, 300, 600, 220);
            let output = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                show(ctx, &theme, &OverviewLayout { kind, strip: Some(strip), workspaces: Vec::new(), ..layout("", 0) }, None);
            });
            assert!(output.shapes.iter().any(|shape| opaque_fill(&shape.shape, rect(strip))));
            assert!(!output.shapes.iter().any(|shape| opaque_fill(&shape.shape, screen)));
        }
    }

    #[test]
    fn drawing_every_kind_does_not_fail() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        for open in [layout("", 0), layout("x", 3), OverviewLayout { kind: OverviewKind::Switcher, strip: Some((100, 300, 600, 220)), workspaces: Vec::new(), ..layout("", 2) }] {
            let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &theme, &open, None));
        }
        // On another display there is nothing to draw into.
        let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &theme, &OverviewLayout { screen: 1, ..layout("", 1) }, None));
    }
}
