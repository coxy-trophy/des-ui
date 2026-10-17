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
    if layout.filter.is_empty() {
        "TYPE TO FIND A WINDOW · ENTER OPENS · DRAG TO ANOTHER WORKSPACE · ESC CLOSES".to_string()
    } else {
        let found = layout.thumbs.len();
        format!("{} · {found} {}", layout.filter.to_uppercase(), if found == 1 { "WINDOW" } else { "WINDOWS" })
    }
}

/// Draws around the pictures of whatever the compositor has open, on the main display.
pub fn show(ctx: &egui::Context, theme: &Theme, layout: &OverviewLayout, state: Option<&DesktopState>) {
    if layout.screen != 0 {
        return;
    }
    let screen = ctx.screen_rect();
    let title_of = |app: usize| state.and_then(|state| state.apps.get(app)).map_or(String::new(), |app| app.title.clone());
    egui::Area::new(egui::Id::new("overview-surround"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(false)
        .show(ctx, |ui| {
            let painter = ui.painter();
            let accent = theme.main;
            match layout.kind {
                OverviewKind::Workspaces => {
                    painter.rect_filled(screen, CornerRadius::ZERO, ui::GROUND);
                    painter.text(pos2(screen.left() + 48.0, screen.top() + 36.0), Align2::LEFT_CENTER, "OVERVIEW", ui::display(18.0), ui::TEXT);
                    let hint = heading(layout);
                    let colour = if layout.filter.is_empty() { ui::LABEL } else { accent };
                    painter.text(pos2(screen.right() - 48.0, screen.top() + 36.0), Align2::RIGHT_CENTER, hint, ui::mono(12.0), colour);
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
                OverviewKind::Switcher => {
                    if let Some(strip) = layout.strip.map(rect) {
                        painter.rect_filled(strip, CornerRadius::same(6), ui::PANEL);
                        painter.rect_stroke(strip, CornerRadius::same(6), Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
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
                let name = fitted(painter, &title_of(thumb.app), &font, width);
                let at: Pos2 = pos2(area.center().x, area.bottom() + 14.0);
                painter.text(at, Align2::CENTER_CENTER, name, font, if selected { ui::TEXT } else { ui::SECONDARY });
            }
        });
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
