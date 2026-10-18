//! What the shell and the compositor must agree on: the frame's geometry, and the
//! protocol they use to talk about open applications.
//!
//! The shell draws the frame; the compositor places application windows inside it.
//! All geometry is in logical pixels.

pub mod input;
pub mod ipc;
pub mod music;
pub mod splash;
pub mod control;
pub mod transport;
pub mod terminal;

/// Height of the bar along the top of the workspace that lists terminal tabs and open
/// applications. It stays visible while an application is shown.
pub const TOP_BAR_HEIGHT: f32 = 44.0;

/// Height of the status strip along the bottom edge, across the full width.
pub const STATUS_HEIGHT: f32 = 28.0;

/// Width of the gap between the two halves of a split workspace.
pub const SPLIT_GAP: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// The two halves of this area when it is split side by side.
    pub fn halves(&self) -> (Rect, Rect) {
        let left = ((self.width - SPLIT_GAP) / 2.0).floor().max(1.0);
        let right_x = self.x + left + SPLIT_GAP;
        (
            Rect { width: left, ..*self },
            Rect {
                x: right_x,
                width: (self.x + self.width - right_x).max(1.0),
                ..*self
            },
        )
    }
}

/// Which of the frame's panels are open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panels {
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Panels {
            left: true,
            right: true,
            bottom: true,
        }
    }
}

/// One of the frame's collapsible panels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    Left,
    Right,
    Bottom,
}

impl Panels {
    pub fn toggle(&mut self, panel: Panel) {
        match panel {
            Panel::Left => self.left = !self.left,
            Panel::Right => self.right = !self.right,
            Panel::Bottom => self.bottom = !self.bottom,
        }
    }

    pub fn bits(&self) -> u8 {
        self.left as u8 | (self.right as u8) << 1 | (self.bottom as u8) << 2
    }

    pub fn from_bits(bits: u8) -> Option<Panels> {
        (bits < 8).then_some(Panels {
            left: bits & 1 != 0,
            right: bits & 2 != 0,
            bottom: bits & 4 != 0,
        })
    }
}

/// Width of each monitoring column, when open.
pub fn side_width(screen_width: f32) -> f32 {
    (screen_width / 6.0).clamp(240.0, 340.0).round()
}

/// Height of the file browser, when open.
pub fn bottom_height(screen_height: f32) -> f32 {
    (screen_height * 0.2333).clamp(160.0, 280.0).round()
}

/// Width retained by a minimized monitoring column in collapse mode.
pub const COLLAPSED_SIDE_WIDTH: f32 = 36.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelBehavior {
    Hide,
    #[default]
    Collapse,
}

impl PanelBehavior {
    pub fn parse(text: &str) -> Self {
        let mut behavior = Self::default();
        for line in text.lines() {
            if let Some((key, value)) = line.split('#').next().unwrap_or("").split_once('=') {
                if key.trim() == "side_panel_behavior" {
                    behavior = match value.trim() {
                        "hide" => Self::Hide,
                        "collapse" => Self::Collapse,
                        _ => behavior,
                    };
                }
            }
        }
        behavior
    }

    pub fn load() -> Self {
        input::config_path().and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text)).unwrap_or_default()
    }
}

pub fn workspace_with_behavior(screen_width: f32, screen_height: f32, panels: Panels, behavior: PanelBehavior) -> Rect {
    let mut area = workspace(screen_width, screen_height, panels);
    if behavior == PanelBehavior::Collapse {
        if !panels.left {
            area.x += COLLAPSED_SIDE_WIDTH;
            area.width -= COLLAPSED_SIDE_WIDTH;
        }
        if !panels.right {
            area.width -= COLLAPSED_SIDE_WIDTH;
        }
        area.width = area.width.max(1.0);
    }
    area
}

/// The area between the open panels, below the top bar and above the status strip: the
/// terminal's home, and where application windows are placed.
pub fn workspace(screen_width: f32, screen_height: f32, panels: Panels) -> Rect {
    let side = side_width(screen_width);
    let left = if panels.left { side } else { 0.0 };
    let right = if panels.right { side } else { 0.0 };
    let bottom = if panels.bottom { bottom_height(screen_height) } else { 0.0 };
    Rect {
        x: left,
        y: TOP_BAR_HEIGHT,
        width: (screen_width - left - right).max(1.0),
        height: (screen_height - STATUS_HEIGHT - bottom - TOP_BAR_HEIGHT).max(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_sits_between_the_panels() {
        let rect = workspace(1920.0, 1080.0, Panels::default());
        assert_eq!(rect.x, 320.0);
        assert_eq!(rect.width, 1280.0);
        assert_eq!(rect.y, TOP_BAR_HEIGHT);
        assert_eq!(rect.y + rect.height, 1080.0 - STATUS_HEIGHT - bottom_height(1080.0));
    }

    #[test]
    fn collapsed_panels_give_their_space_to_the_workspace() {
        let all = Panels::from_bits(0).unwrap();
        let rect = workspace(1920.0, 1080.0, all);
        assert_eq!((rect.x, rect.width), (0.0, 1920.0));
        assert_eq!(rect.y + rect.height, 1080.0 - STATUS_HEIGHT);

        let mut panels = Panels::default();
        panels.toggle(Panel::Left);
        let rect = workspace(1920.0, 1080.0, panels);
        assert_eq!((rect.x, rect.width), (0.0, 1600.0));
    }

    #[test]
    fn panels_round_trip_as_bits() {
        for bits in 0..8 {
            assert_eq!(Panels::from_bits(bits).unwrap().bits(), bits);
        }
        assert_eq!(Panels::from_bits(8), None);
        assert_eq!(Panels::default().bits(), 7);
    }

    #[test]
    fn halves_cover_the_area_with_a_gap() {
        let (left, right) = workspace(1920.0, 1080.0, Panels::default()).halves();
        assert_eq!(left.x, 320.0);
        assert_eq!(right.x, left.x + left.width + SPLIT_GAP);
        assert_eq!(right.x + right.width, 1600.0);
        assert_eq!(left.height, right.height);
    }

    #[test]
    fn workspace_never_collapses() {
        let rect = workspace(100.0, 50.0, Panels::default());
        assert!(rect.width >= 1.0 && rect.height >= 1.0);
    }
}

#[cfg(test)]
mod rail_tests {
    use super::*;

    #[test]
    fn minimized_side_rails_reserve_space_without_changing_bottom_behavior() {
        let panels = Panels { left: false, right: false, bottom: false };
        let area = workspace_with_behavior(1920.0, 1080.0, panels, PanelBehavior::Collapse);
        assert_eq!(area.x, COLLAPSED_SIDE_WIDTH);
        assert_eq!(area.width, 1920.0 - 2.0 * COLLAPSED_SIDE_WIDTH);
        assert_eq!(area.y + area.height, 1080.0 - STATUS_HEIGHT);
        assert_eq!(workspace_with_behavior(1920.0, 1080.0, panels, PanelBehavior::Hide), workspace(1920.0, 1080.0, panels));
        let mixed = Panels { left: true, ..panels };
        let area = workspace_with_behavior(1920.0, 1080.0, mixed, PanelBehavior::Collapse);
        assert_eq!(area.x, side_width(1920.0));
        assert_eq!(area.x + area.width, 1920.0 - COLLAPSED_SIDE_WIDTH);
    }

    #[test]
    fn panel_behavior_defaults_and_config_values() {
        assert_eq!(PanelBehavior::parse(""), PanelBehavior::Collapse);
        assert_eq!(PanelBehavior::parse("side_panel_behavior = hide # preference"), PanelBehavior::Hide);
        assert_eq!(PanelBehavior::parse("side_panel_behavior = collapse"), PanelBehavior::Collapse);
        assert_eq!(PanelBehavior::parse("side_panel_behavior = nonsense"), PanelBehavior::Collapse);
    }
}

/// Thickness of the primary display bottom edge that reveals the dock.
pub const DOCK_TRIGGER_HEIGHT: f32 = 4.0;

/// Primary-display-local logical coordinates; remains inside tiny displays.
pub fn dock_trigger(width: f32, height: f32) -> Rect {
    let height = height.max(0.0);
    let thickness = height.min(DOCK_TRIGGER_HEIGHT);
    Rect { x: 0.0, y: height - thickness, width: width.max(0.0), height: thickness }
}

#[cfg(test)]
mod dock_geometry_tests {
    use super::*;

    #[test]
    fn trigger_covers_exactly_the_primary_bottom_edge() {
        assert_eq!(dock_trigger(1920.0, 1080.0), Rect { x: 0.0, y: 1076.0, width: 1920.0, height: 4.0 });
        assert_eq!(dock_trigger(3840.0, 2160.0), Rect { x: 0.0, y: 2156.0, width: 3840.0, height: 4.0 });
        assert_eq!(dock_trigger(2.0, 2.0), Rect { x: 0.0, y: 0.0, width: 2.0, height: 2.0 });
        assert_eq!(dock_trigger(0.0, 0.0), Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
    }
}
