//! Shared separated-panel geometry for shell drawing and native window placement.

use crate::{Panels, PanelBehavior, Rect, TOP_BAR_HEIGHT, STATUS_HEIGHT, COLLAPSED_SIDE_WIDTH};

pub const GAP: f32 = 8.0;
pub const RADIUS: f32 = 8.0;
const RIM: f32 = 4.0;

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub center: Rect,
    pub workspace: Rect,
    pub header: Rect,
    pub left: Option<Rect>,
    pub right: Option<Rect>,
    pub bottom: Option<Rect>,
}

impl Layout {
    pub fn new(width: f32, height: f32, panels: Panels, behavior: PanelBehavior) -> Self {
        let width=width.max(1.0); let height=height.max(STATUS_HEIGHT+1.0);
        let edge=GAP.min(width/8.0).min((height-STATUS_HEIGHT)/8.0);
        let occupied=|open| if open {crate::side_width(width).min(width*0.4)}
            else if behavior==PanelBehavior::Collapse {COLLAPSED_SIDE_WIDTH.min(width*0.4)} else {0.0};
        let left=occupied(panels.left);let right=occupied(panels.right);
        let x=if left>0.0 {left+edge/2.0}else{edge};
        let end=if right>0.0 {width-right-edge/2.0}else{width-edge};
        let floor=(height-STATUS_HEIGHT-edge).max(edge+1.0);
        let bottom=panels.bottom.then(|| {
            let available=(floor-edge-TOP_BAR_HEIGHT-edge).max(2.0);
            let size=(crate::bottom_height(height)-edge*1.5).max(1.0).min(available*0.45);
            Rect{x,y:floor-size,width:(end-x).max(1.0),height:size}
        });
        let center_end=bottom.map_or(floor,|rect|rect.y-edge);
        let center=Rect{x,y:edge,width:(end-x).max(1.0),height:(center_end-edge).max(1.0)};
        let rim=RIM.min(center.width/4.0).min(center.height/4.0);
        let header_height=TOP_BAR_HEIGHT.min((center.height-rim-1.0).max(1.0));
        let workspace=Rect{x:center.x+rim,y:center.y+header_height,
            width:(center.width-2.0*rim).max(1.0),height:(center.height-header_height-rim).max(1.0)};
        let header=Rect{x:workspace.x,y:center.y+rim,width:workspace.width,
            height:(workspace.y-center.y-2.0*rim).max(1.0)};
        let side=|start: f32,end: f32|Rect{x:start,y:edge,width:(end-start).max(1.0),height:(floor-edge).max(1.0)};
        Self {center,workspace,header,bottom,
            left:(left>0.0).then(||side(edge,center.x-edge)),
            right:(right>0.0).then(||side(center.x+center.width+edge,width-edge))}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Panels, PanelBehavior, Rect, STATUS_HEIGHT};
    fn right(rect: Rect) -> f32 { rect.x + rect.width }
    fn bottom(rect: Rect) -> f32 { rect.y + rect.height }
    #[test]
    fn default_frame_has_eight_pixel_channels_and_an_inset_app() {
        let layout = Layout::new(1920.0, 1080.0, Panels::default(), PanelBehavior::Collapse);
        let left = layout.left.unwrap(); let right_panel = layout.right.unwrap(); let files = layout.bottom.unwrap();
        assert_eq!(layout.center.x - right(left), 8.0);
        assert_eq!(right_panel.x - right(layout.center), 8.0);
        assert!((files.y - bottom(layout.center) - 8.0).abs() < 0.01);
        assert_eq!(left.x, 8.0); assert_eq!(left.y, 8.0);
        assert_eq!(1920.0-right(right_panel), 8.0);
        assert_eq!(1080.0-STATUS_HEIGHT-bottom(files), 8.0);
        assert_eq!(layout.workspace.x-layout.center.x, 4.0);
        assert_eq!(right(layout.center)-right(layout.workspace), 4.0);
        assert_eq!(bottom(layout.center)-bottom(layout.workspace), 4.0);
        assert!(bottom(layout.header) <= layout.workspace.y);
    }
    #[test]
    fn collapse_rails_and_hidden_panels_keep_the_correct_ownership() {
        let panels = Panels::from_bits(0).unwrap();
        let rails = Layout::new(1280.0, 900.0, panels, PanelBehavior::Collapse);
        let hidden = Layout::new(1280.0, 900.0, panels, PanelBehavior::Hide);
        assert!(rails.left.is_some() && rails.right.is_some());
        assert!(hidden.left.is_none() && hidden.right.is_none() && hidden.bottom.is_none());
        assert!(hidden.workspace.width > rails.workspace.width);
        assert_eq!(hidden.center.x, 8.0);
    }
    #[test]
    fn all_panel_states_stay_inside_small_and_fractional_display_bounds() {
        for (width,height) in [(320.0,240.0),(640.0,480.0),(1280.0,720.0),(1098.0,618.0),(1920.0,1080.0)] {
            for bits in 0..8 { for behavior in [PanelBehavior::Collapse,PanelBehavior::Hide] {
                let layout=Layout::new(width,height,Panels::from_bits(bits).unwrap(),behavior);
                for rect in [Some(layout.center),Some(layout.workspace),Some(layout.header),layout.left,layout.right,layout.bottom].into_iter().flatten() {
                    assert!(rect.width>=1.0 && rect.height>=1.0,"{rect:?}");
                    assert!(rect.x>=0.0 && rect.y>=0.0 && right(rect)<=width+0.01 && bottom(rect)<=height-STATUS_HEIGHT+0.01,"{width}x{height}: {rect:?}");
                }
            }}
        }
    }
}
