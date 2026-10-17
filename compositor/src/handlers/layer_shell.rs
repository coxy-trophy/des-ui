//! Layer surfaces: wallpapers, bars, launchers and notification daemons that other
//! desktops' tools draw (`swaybg`, `waybar`, `wofi`, `mako` and the like).
//!
//! Where they go, the frame being the shell's own fullscreen window:
//! - **overlay** and **top** layers are drawn above every application, beneath the
//!   shell's toasts, dock and drop-down;
//! - **bottom** and **background** layers are drawn beneath every application and
//!   above the shell's window, so that they can be seen at all. On the main display
//!   they are cut to the workspace, leaving the frame alone; on the others they cover
//!   the display, over the shell's backdrop;
//! - while locked none of them is drawn or touched, and none has the keyboard;
//! - while a screenshot is being chosen they stay in view but take no input.
//!
//! An exclusive zone keeps applications clear of a bar: the area applications use on
//! that display is what remains of it once each edge's reservations are taken off. The
//! frame itself never moves, so a bar small enough to lie over the frame's top changes
//! nothing on the main display.

use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements,
            surface::WaylandSurfaceRenderElement,
            utils::CropRenderElement,
        },
        gles::GlesRenderer,
    },
    delegate_layer_shell,
    desktop::{LayerSurface, PopupKind, WindowSurfaceType, layer_map_for_output, space::SpaceRenderElements},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Scale},
    wayland::{
        compositor::with_states,
        shell::{
            wlr_layer::{
                Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface,
                LayerSurfaceData, Margins, WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::PopupSurface,
        },
    },
};

use crate::{DesComp, focus::Focus};

type Surface = WaylandSurfaceRenderElement<GlesRenderer>;

/// How much of each edge of a display the layers keep to themselves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Insets {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

impl Insets {
    fn add(self, other: Insets) -> Insets {
        Insets {
            top: self.top + other.top,
            bottom: self.bottom + other.bottom,
            left: self.left + other.left,
            right: self.right + other.right,
        }
    }
}

/// What one layer surface reserves. As in the protocol, a zone counts only for a
/// surface anchored to one edge, alone or together with both edges beside it; a
/// surface anchored to opposite edges, or to a corner, reserves nothing.
pub fn reserved(anchor: Anchor, zone: ExclusiveZone, margin: Margins) -> Insets {
    let ExclusiveZone::Exclusive(amount) = zone else {
        return Insets::default();
    };
    let amount = amount.min(i32::MAX as u32) as i32;
    let horizontal = anchor.contains(Anchor::LEFT) == anchor.contains(Anchor::RIGHT);
    let vertical = anchor.contains(Anchor::TOP) == anchor.contains(Anchor::BOTTOM);
    let mut insets = Insets::default();
    if anchor.contains(Anchor::TOP) && !anchor.contains(Anchor::BOTTOM) && horizontal {
        insets.top = amount + margin.top.max(0);
    } else if anchor.contains(Anchor::BOTTOM) && !anchor.contains(Anchor::TOP) && horizontal {
        insets.bottom = amount + margin.bottom.max(0);
    } else if anchor.contains(Anchor::LEFT) && !anchor.contains(Anchor::RIGHT) && vertical {
        insets.left = amount + margin.left.max(0);
    } else if anchor.contains(Anchor::RIGHT) && !anchor.contains(Anchor::LEFT) && vertical {
        insets.right = amount + margin.right.max(0);
    }
    insets
}

/// What remains of `display` once `insets` are taken off; reservations larger than the
/// display leave nothing rather than a negative size.
pub fn usable(display: Rectangle<i32, Logical>, insets: Insets) -> Rectangle<i32, Logical> {
    let left = insets.left.clamp(0, display.size.w);
    let right = insets.right.clamp(0, display.size.w - left);
    let top = insets.top.clamp(0, display.size.h);
    let bottom = insets.bottom.clamp(0, display.size.h - top);
    Rectangle::new(
        (display.loc.x + left, display.loc.y + top).into(),
        (display.size.w - left - right, display.size.h - top - bottom).into(),
    )
}

/// The applications' area on a display, kept clear of the layers' reservations. When
/// nothing would be left, the area is kept as it was rather than shrunk to nothing.
pub fn clear_of(area: Rectangle<i32, Logical>, usable: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    match area.intersection(usable) {
        Some(rest) if rest.size.w > 0 && rest.size.h > 0 => rest,
        _ => area,
    }
}

/// Whether a layer surface may have the keyboard: never behind the lock or while a
/// screenshot is chosen; an exclusive one above the windows while it is shown; an
/// on-demand one (or an exclusive one beneath the windows) once it is clicked.
pub fn takes_keyboard(interactivity: KeyboardInteractivity, layer: Layer, locked: bool, picking: bool, clicked: bool) -> bool {
    if locked || picking {
        return false;
    }
    match interactivity {
        KeyboardInteractivity::Exclusive => matches!(layer, Layer::Top | Layer::Overlay) || clicked,
        KeyboardInteractivity::OnDemand => clicked,
        KeyboardInteractivity::None => false,
    }
}

/// Whether a layer surface has something to show.
fn mapped(layer: &LayerSurface) -> bool {
    smithay::backend::renderer::utils::with_renderer_surface_state(layer.wl_surface(), |state| state.buffer().is_some())
        .unwrap_or(false)
}

/// What the main display's workspace is, for cutting what is drawn beneath the windows;
/// worked out before drawing, while the state can still be read.
#[derive(Debug, Clone, Copy)]
pub struct Beneath {
    pub main: Point<i32, Logical>,
    pub workspace: Rectangle<i32, Logical>,
    pub locked: bool,
}

/// The layer surfaces of `output` to draw, front to back: those above the windows when
/// `above`, else those beneath them, cut to the workspace on the main display. Nothing
/// is drawn while locked.
pub fn layer_elements<C>(
    renderer: &mut GlesRenderer,
    space: &smithay::desktop::Space<smithay::desktop::Window>,
    output: &Output,
    above: bool,
    beneath: Beneath,
) -> Vec<C>
where
    C: From<SpaceRenderElements<GlesRenderer, Surface>> + From<CropRenderElement<Surface>>,
{
    if beneath.locked {
        return Vec::new();
    }
    let Some(geometry) = space.output_geometry(output) else {
        return Vec::new();
    };
    let scale = output.current_scale().fractional_scale();
    let kinds = if above { [Layer::Overlay, Layer::Top] } else { [Layer::Bottom, Layer::Background] };
    let crop = (!above && geometry.contains(beneath.main)).then(|| {
        let mut area = beneath.workspace;
        area.loc -= geometry.loc;
        area.to_physical_precise_round(scale)
    });
    let map = layer_map_for_output(output);
    let mut elements = Vec::new();
    for kind in kinds {
        // The map lists each layer back to front; drawing lists front to back.
        for layer in map.layers_on(kind).rev() {
            let Some(at) = map.layer_geometry(layer) else {
                continue;
            };
            let surfaces: Vec<Surface> =
                layer.render_elements(renderer, at.loc.to_physical_precise_round(scale), Scale::from(scale), 1.0);
            match crop {
                Some(crop) => elements.extend(
                    surfaces
                        .into_iter()
                        .filter_map(|surface| CropRenderElement::from_element(surface, scale, crop))
                        .map(C::from),
                ),
                None => elements.extend(surfaces.into_iter().map(|surface| C::from(SpaceRenderElements::Surface(surface)))),
            }
        }
    }
    elements
}

impl WlrLayerShellHandler for DesComp {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        // Without a display named, it goes on the one in use.
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.output_of_screen(self.focus_screen));
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        let layer = LayerSurface::new(surface, namespace);
        if let Err(error) = layer_map_for_output(&output).map_layer(&layer) {
            tracing::warn!("cannot place a layer surface: {error}");
        }
    }

    fn new_popup(&mut self, parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_layer_popup(parent.wl_surface(), &popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let gone = map.layers().find(|layer| layer.layer_surface() == &surface).cloned();
            if let Some(layer) = gone {
                map.unmap_layer(&layer);
            }
        }
        if self.layer_focus.as_ref().is_some_and(|layer| layer.layer_surface() == &surface) {
            self.layer_focus = None;
        }
        self.layers_changed();
    }
}

delegate_layer_shell!(DesComp);

impl DesComp {
    /// The output a display is shown on.
    pub fn output_of_screen(&self, screen: usize) -> Option<Output> {
        let rect = self.screens.get(screen).or(self.screens.first())?;
        self.space
            .outputs()
            .find(|output| self.space.output_geometry(output).is_some_and(|geometry| geometry.contains(rect.loc)))
            .cloned()
    }

    /// What the layers reserve on each output, in logical coordinates of the whole desktop.
    fn reservations(&self) -> Vec<(Rectangle<i32, Logical>, Insets)> {
        self.space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let map = layer_map_for_output(output);
                let insets = map
                    .layers()
                    .filter(|layer| mapped(layer))
                    .map(|layer| {
                        let state = layer.cached_state();
                        reserved(state.anchor, state.exclusive_zone, state.margin)
                    })
                    .fold(Insets::default(), Insets::add);
                Some((geometry, insets))
            })
            .collect()
    }

    /// The applications' area on a display, kept clear of the bars on it.
    pub fn clear_of_layers(&self, screen: usize, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let Some(display) = self.screens.get(screen) else {
            return area;
        };
        self.reservations()
            .into_iter()
            .filter(|(geometry, _)| geometry.contains(display.loc))
            .fold(area, |area, (geometry, insets)| clear_of(area, usable(geometry, insets)))
    }

    /// Called on every commit: places and configures a layer surface, gives the
    /// keyboard to one that asks for it, and moves the windows when a bar's
    /// reservation changed. Returns whether the surface was a layer surface.
    pub fn layer_commit(&mut self, surface: &WlSurface) -> bool {
        let Some(output) = self
            .space
            .outputs()
            .find(|output| layer_map_for_output(output).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some())
            .cloned()
        else {
            return false;
        };
        let initial_sent = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().map(|data| data.lock().unwrap().initial_configure_sent)
        })
        .unwrap_or(true);
        let layer = {
            let mut map = layer_map_for_output(&output);
            map.arrange();
            map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).cloned()
        };
        let Some(layer) = layer else {
            return true;
        };
        if !initial_sent {
            layer.layer_surface().send_configure();
        }

        let shown = mapped(&layer);
        let state = layer.cached_state();
        let mut changed = false;
        if shown
            && self.layer_focus.as_ref() != Some(&layer)
            && takes_keyboard(state.keyboard_interactivity, state.layer, self.locked, self.picking, false)
        {
            // A launcher opening takes the keyboard while it is up.
            self.layer_focus = Some(layer.clone());
            changed = true;
        } else if !shown && self.layer_focus.as_ref() == Some(&layer) {
            self.layer_focus = None;
            changed = true;
        }
        let reservations = self.reservations();
        if reservations != self.layer_reservations {
            self.layer_reservations = reservations;
            changed = true;
        }
        if changed {
            self.arrange();
        }
        true
    }

    /// After a layer surface goes: forget its reservation and give the keyboard back.
    fn layers_changed(&mut self) {
        self.layer_reservations = self.reservations();
        self.arrange();
    }

    /// The layer surface holding the keyboard, while it still may.
    pub fn layer_keyboard(&self) -> Option<Focus> {
        let layer = self.layer_focus.as_ref()?;
        let state = layer.cached_state();
        let allowed = layer.alive()
            && mapped(layer)
            && state.keyboard_interactivity != KeyboardInteractivity::None
            && !self.locked
            && !self.picking;
        allowed.then(|| Focus::Surface(layer.wl_surface().clone()))
    }

    /// A click: an on-demand layer surface under it takes the keyboard, and one that
    /// had it on demand gives it back when the click is elsewhere. Returns true when
    /// the click went to a layer surface.
    pub fn layer_click(&mut self, pos: Point<f64, Logical>) -> bool {
        if let Some((layer, _, _)) = self.layer_hit(pos) {
            let state = layer.cached_state();
            if takes_keyboard(state.keyboard_interactivity, state.layer, self.locked, self.picking, true) {
                if self.layer_focus.as_ref() != Some(&layer) {
                    self.layer_focus = Some(layer);
                    self.arrange();
                }
                return true;
            }
            return false;
        }
        let on_demand = self
            .layer_focus
            .as_ref()
            .is_some_and(|layer| layer.cached_state().keyboard_interactivity != KeyboardInteractivity::Exclusive);
        if on_demand {
            self.layer_focus = None;
            self.arrange();
        }
        false
    }

    /// The layer surface at `pos`, in the order they are drawn: overlay and top layers,
    /// then (where no application is) bottom and background layers. With it, the
    /// surface hit and where that surface's origin is.
    pub fn layer_hit(&self, pos: Point<f64, Logical>) -> Option<(LayerSurface, WlSurface, Point<f64, Logical>)> {
        if self.locked || self.picking {
            return None;
        }
        // Worked out before the layer map is locked: the workspace reads every map.
        let on_app = self
            .space
            .element_under(pos)
            .is_some_and(|(window, _)| self.apps.iter().any(|app| app.window == *window) || self.x_popups.contains(window));
        let in_frame = self.screen_at(pos) == 0 && !self.workspace_of(0).to_f64().contains(pos);
        let output = self.space.output_under(pos).next()?;
        let origin = self.space.output_geometry(output)?.loc.to_f64();
        let local = pos - origin;
        let map = layer_map_for_output(output);
        let hit = |kind: Layer| {
            let layer = map.layer_under(kind, local)?;
            let at = map.layer_geometry(layer)?.loc.to_f64();
            let (surface, offset) = layer.surface_under(local - at, WindowSurfaceType::ALL)?;
            Some((layer.clone(), surface, offset.to_f64() + at + origin))
        };
        if let Some(found) = hit(Layer::Overlay).or_else(|| hit(Layer::Top)) {
            return Some(found);
        }
        // An application covers what is beneath it; on the main display, what is
        // beneath the windows shows only in the workspace.
        if on_app || in_frame {
            return None;
        }
        hit(Layer::Bottom).or_else(|| hit(Layer::Background))
    }

    /// What drawing the layers needs to know of the state.
    pub fn beneath(&self) -> Beneath {
        Beneath {
            main: self.screens.first().map_or_else(|| (0, 0).into(), |main| main.loc),
            workspace: self.workspace_of(0),
            locked: self.locked,
        }
    }

    /// Lets every layer surface draw its next frame, and forgets those that are gone.
    pub fn layer_frames(&self, elapsed: std::time::Duration) {
        for output in self.space.outputs() {
            let mut map = layer_map_for_output(output);
            for layer in map.layers() {
                layer.send_frame(output, elapsed, Some(std::time::Duration::ZERO), |_, _| Some(output.clone()));
            }
            map.cleanup();
        }
    }

    /// A popup of a layer surface may use the display that surface is on.
    fn unconstrain_layer_popup(&self, parent: &WlSurface, popup: &PopupSurface) {
        for output in self.space.outputs() {
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            let map = layer_map_for_output(output);
            let Some(layer) = map.layer_for_surface(parent, WindowSurfaceType::TOPLEVEL) else {
                continue;
            };
            let Some(at) = map.layer_geometry(layer) else {
                return;
            };
            // The positioner's target is relative to the parent surface.
            let mut target = Rectangle::from_size(geometry.size);
            target.loc -= at.loc;
            target.loc -= smithay::desktop::get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
            popup.with_pending_state(|state| {
                state.geometry = state.positioner.get_unconstrained_geometry(target);
            });
            return;
        }
    }
}

trait Alive {
    fn alive(&self) -> bool;
}

impl Alive for LayerSurface {
    fn alive(&self) -> bool {
        use smithay::reexports::wayland_server::Resource;
        self.wl_surface().is_alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    }

    fn margin() -> Margins {
        Margins { top: 0, bottom: 0, left: 0, right: 0 }
    }

    #[test]
    fn a_bar_along_one_edge_reserves_that_edge() {
        let across = Anchor::LEFT | Anchor::RIGHT;
        assert_eq!(reserved(Anchor::TOP | across, ExclusiveZone::Exclusive(30), margin()), Insets { top: 30, ..Default::default() });
        assert_eq!(reserved(Anchor::BOTTOM | across, ExclusiveZone::Exclusive(40), margin()), Insets { bottom: 40, ..Default::default() });
        let down = Anchor::TOP | Anchor::BOTTOM;
        assert_eq!(reserved(Anchor::LEFT | down, ExclusiveZone::Exclusive(50), margin()), Insets { left: 50, ..Default::default() });
        assert_eq!(reserved(Anchor::RIGHT, ExclusiveZone::Exclusive(60), margin()), Insets { right: 60, ..Default::default() });
        // The margin on that edge counts too.
        let spaced = Margins { top: 8, ..margin() };
        assert_eq!(reserved(Anchor::TOP | across, ExclusiveZone::Exclusive(30), spaced), Insets { top: 38, ..Default::default() });
    }

    #[test]
    fn corners_opposite_edges_and_neutral_zones_reserve_nothing() {
        assert_eq!(reserved(Anchor::TOP | Anchor::LEFT, ExclusiveZone::Exclusive(30), margin()), Insets::default());
        assert_eq!(reserved(Anchor::TOP | Anchor::BOTTOM, ExclusiveZone::Exclusive(30), margin()), Insets::default());
        assert_eq!(reserved(Anchor::all(), ExclusiveZone::Exclusive(30), margin()), Insets::default());
        assert_eq!(reserved(Anchor::TOP, ExclusiveZone::Neutral, margin()), Insets::default());
        assert_eq!(reserved(Anchor::TOP, ExclusiveZone::DontCare, margin()), Insets::default());
    }

    #[test]
    fn reservations_on_several_edges_combine() {
        let insets = Insets { top: 30, ..Default::default() }
            .add(Insets { bottom: 40, ..Default::default() })
            .add(Insets { left: 50, ..Default::default() })
            .add(Insets { top: 10, ..Default::default() });
        let rest = usable(display(), insets);
        assert_eq!(rest, Rectangle::new((50, 40).into(), (1870, 1000).into()));
    }

    #[test]
    fn a_reservation_larger_than_the_display_clamps() {
        let rest = usable(display(), Insets { top: 5000, bottom: 10, left: 0, right: 0 });
        assert_eq!(rest.size.h, 0);
        assert!(rest.size.w == 1920 && rest.loc.y == 1080);
        let rest = usable(display(), Insets { left: 1500, right: 1500, ..Default::default() });
        assert_eq!(rest.size.w, 0);
        // Applications are not squeezed to nothing: their area stays as it was.
        let area = Rectangle::new((100, 100).into(), (800, 600).into());
        assert_eq!(clear_of(area, rest), area);
    }

    #[test]
    fn the_frame_s_workspace_moves_only_where_a_bar_reaches_into_it() {
        // The main display's workspace starts below the frame's top bar.
        let workspace = Rectangle::new((300, 44).into(), (1320, 800).into());
        let small_bar = usable(display(), Insets { top: 30, ..Default::default() });
        assert_eq!(clear_of(workspace, small_bar), workspace);
        let tall_bar = usable(display(), Insets { top: 60, ..Default::default() });
        assert_eq!(clear_of(workspace, tall_bar), Rectangle::new((300, 60).into(), (1320, 784).into()));
        // A secondary display's applications take all of it but the bar.
        assert_eq!(clear_of(display(), small_bar), Rectangle::new((0, 30).into(), (1920, 1050).into()));
    }

    #[test]
    fn the_keyboard_goes_to_layers_only_when_allowed() {
        use KeyboardInteractivity::*;
        assert!(takes_keyboard(Exclusive, Layer::Overlay, false, false, false));
        assert!(takes_keyboard(Exclusive, Layer::Top, false, false, false));
        assert!(!takes_keyboard(Exclusive, Layer::Background, false, false, false));
        assert!(takes_keyboard(Exclusive, Layer::Background, false, false, true));
        assert!(!takes_keyboard(OnDemand, Layer::Top, false, false, false));
        assert!(takes_keyboard(OnDemand, Layer::Top, false, false, true));
        assert!(!takes_keyboard(None, Layer::Top, false, false, true));
        // Never behind the lock, never while a screenshot is chosen.
        assert!(!takes_keyboard(Exclusive, Layer::Overlay, true, false, false));
        assert!(!takes_keyboard(OnDemand, Layer::Top, true, false, true));
        assert!(!takes_keyboard(Exclusive, Layer::Overlay, false, true, false));
    }
}
