//! Toasts: a part of the shell's window shown above everything else.
//!
//! The shell draws its notifications into its own window, which lies beneath the
//! applications, and says where they are. That rectangle of the shell is then drawn
//! once more on top, and the pointer there belongs to the shell.

use smithay::{
    backend::renderer::{
        element::{
            Element, Id, Kind, RenderElement, UnderlyingStorage,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            utils::CropRenderElement,
        },
        gles::GlesRenderer,
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    desktop::Window,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform},
};

use crate::state::DesComp;

/// The cropped copy of the shell. It goes by a name of its own, so that what tracks
/// changes on screen does not take it for the shell's window beneath.
pub struct ToastElement {
    inner: CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    id: Id,
}

impl Element for ToastElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for ToastElement {
    fn draw(
        &self,
        frame: &mut <GlesRenderer as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <GlesRenderer as smithay::backend::renderer::RendererSuper>::Error> {
        RenderElement::<GlesRenderer>::draw(&self.inner, frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

impl DesComp {
    pub fn shell_on(&self,screen:usize)->Option<&Window>{if screen==0{self.shell.as_ref()}else{self.backdrops.get(screen-1)}}

    pub fn dock_area_on(&self,screen:usize)->Option<Rectangle<i32,Logical>>{
        if self.locked || self.picking || self.shell_on(screen).is_none(){return None;}
        let display=self.screens.get(screen)?;
        let mut area=if screen==0{self.dock?}else{self.docks.get(&screen)?.0};
        area.loc+=display.loc;
        area.intersection(*display)
    }
    pub fn dock_trigger_area_on(&self,screen:usize)->Option<Rectangle<i32,Logical>>{
        if self.locked || self.picking || self.shell_on(screen).is_none(){return None;}
        let display=self.screens.get(screen)?;
        let area=des_common::dock_trigger(display.size.w as f32,display.size.h as f32);
        Some(Rectangle::new(display.loc+Point::from((area.x as i32,area.y as i32)),(area.width as i32,area.height as i32).into()))
    }
    pub fn dock_layers(&self)->Vec<(Window,Rectangle<i32,Logical>,Point<i32,Logical>,Id)>{
        (0..self.screens.len()).filter_map(|screen|Some((self.shell_on(screen)?.clone(),self.dock_area_on(screen)?,self.screens.get(screen)?.loc,
            if screen==0{self.dock_id.clone()}else{self.docks.get(&screen)?.1.clone()}))).collect()
    }
    pub fn shell_overlay_contains(&self,pos:Point<f64,Logical>)->bool{
        let screen=self.screen_at(pos);
        [self.toast_area().filter(|_|screen==0),self.overlay_area().filter(|_|screen==0),self.dock_area_on(screen),self.dock_trigger_area_on(screen)].into_iter().flatten().any(|area|area.to_f64().contains(pos))
    }

    /// A visual caption only. Deliberately excluded from shell_overlay_contains.
    pub fn companion_bubble_area(&self) -> Option<Rectangle<i32, Logical>> {
        if self.locked || self.picking || self.shell.is_none() { return None; }
        let (mut area, until) = self.companion_bubble?;
        if std::time::Instant::now() >= until { return None; }
        let main = *self.screens.first()?;
        area.loc += main.loc;
        area.intersection(main)
    }

    /// Where on the desktop the shell's overlay is, such as its drop-down terminal.
    /// Never behind the lock, and never while a screenshot is being chosen.
    pub fn overlay_area(&self) -> Option<Rectangle<i32, Logical>> {
        let main = *self.screens.first()?;
        let mut area = self.overlay.filter(|_| !self.locked && !self.picking && self.shell.is_some())?;
        area.loc += main.loc;
        area.intersection(main)
    }

    /// Where on the desktop the shell's toasts are, if it is showing any. Locked, the
    /// shell is all there is to see, and nothing needs lifting above anything.
    pub fn toast_area(&self) -> Option<Rectangle<i32, Logical>> {
        let mut area = self.toast.filter(|_| !self.locked && self.shell.is_some())?;
        area.loc += self.screens.first()?.loc;
        Some(area)
    }
}

/// The toasts as something to draw on the display whose top left corner is at
/// `origin`, above all else there. Taken from parts, since the renderer is borrowed
/// from the same state while frames are drawn.
pub fn elements(
    renderer: &mut GlesRenderer,
    shell: Option<&Window>,
    area: Option<Rectangle<i32, Logical>>,
    primary: Option<Point<i32, Logical>>,
    id: &Id,
    origin: Point<i32, Logical>,
    scale: f64,
) -> Vec<ToastElement> {
    let (Some(area), Some(shell), Some(primary)) = (area, shell, primary) else {
        return Vec::new();
    };
    let Some(surface) = shell.toplevel().map(|toplevel| toplevel.wl_surface().clone()) else {
        return Vec::new();
    };
    let crop = Rectangle::new(area.loc - origin, area.size).to_f64().to_physical(scale).to_i32_round();
    let whole: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
        renderer,
        &surface,
        (primary - origin).to_f64().to_physical(scale).to_i32_round(),
        scale,
        1.0,
        Kind::Unspecified,
    );
    // The shell's window is one surface; were there more, the last is the window itself.
    whole
        .into_iter()
        .next_back()
        .and_then(|element| CropRenderElement::from_element(element, scale, crop))
        .map(|inner| ToastElement { inner, id: id.clone() })
        .into_iter()
        .collect()
}
