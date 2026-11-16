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
    inner: Inner,
    id: Id,
}

/// The whole rectangle, or one rounded surface in it through the clipping shader.
enum Inner {
    Crop(CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>),
    Rounded(smithay::backend::renderer::gles::element::TextureShaderElement),
}

macro_rules! each {
    ($inner:expr, $e:ident => $body:expr) => {
        match $inner { Inner::Crop($e) => $body, Inner::Rounded($e) => $body }
    };
}

impl Element for ToastElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        each!(&self.inner, e => e.current_commit())
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        each!(&self.inner, e => e.src())
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        each!(&self.inner, e => e.geometry(scale))
    }
    fn transform(&self) -> Transform {
        each!(&self.inner, e => e.transform())
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        each!(&self.inner, e => e.damage_since(scale, commit))
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        each!(&self.inner, e => e.opaque_regions(scale))
    }
    fn alpha(&self) -> f32 {
        each!(&self.inner, e => e.alpha())
    }
    fn kind(&self) -> Kind {
        each!(&self.inner, e => e.kind())
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
        each!(&self.inner, e => RenderElement::<GlesRenderer>::draw(e, frame, src, dst, damage, opaque_regions))
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        each!(&self.inner, e => e.underlying_storage(renderer))
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
    pub fn dock_layers(&self)->Vec<(Window,Rectangle<i32,Logical>,Point<i32,Logical>,Id)>{
        (0..self.screens.len()).filter_map(|screen|Some((self.shell_on(screen)?.clone(),self.dock_area_on(screen)?,self.screens.get(screen)?.loc,
            if screen==0{self.dock_id.clone()}else{self.docks.get(&screen)?.1.clone()}))).collect()
    }
    pub fn shell_overlay_contains(&self,pos:Point<f64,Logical>)->bool{
        let screen=self.screen_at(pos);
        [self.toast_area().filter(|_|screen==0),self.island_area().filter(|_|screen==0),self.overlay_area().filter(|_|screen==0),self.dock_area_on(screen)].into_iter().flatten().any(|area|area.to_f64().contains(pos))
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

    /// Where on the desktop the status island reaches out of the strip, if it does.
    /// Not behind the lock, nor while a screenshot is being chosen.
    pub fn island_area(&self) -> Option<Rectangle<i32, Logical>> {
        self.island_shape().map(|(area, _)| area)
    }

    /// The island's area on the desktop and its corner radius.
    pub fn island_shape(&self) -> Option<(Rectangle<i32, Logical>, i32)> {
        let main = *self.screens.first()?;
        let (mut area, radius) = self.island.filter(|_| !self.locked && !self.picking && self.shell.is_some())?;
        area.loc += main.loc;
        Some((area.intersection(main)?, radius))
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
    shader: &mut IslandShader,
    rounded: &[(Rectangle<i32, Logical>, i32)],
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
    // Made of rounded surfaces (Command::Rounded, in the shell's window): drawn as
    // them, so that applications show past their corners. All of them or none.
    let inside: Vec<(Rectangle<i32, Logical>, i32)> = rounded.iter()
        .map(|(rect, radius)| (Rectangle::new(rect.loc + primary, rect.size), *radius))
        .filter(|(rect, _)| area.contains_rect(*rect))
        .collect();
    if !inside.is_empty() {
        let shaped: Vec<ToastElement> = inside.iter()
            .filter_map(|shape| island_element(renderer, shader, Some(shell), Some(*shape), Some(primary), origin, scale))
            .map(|element| ToastElement { inner: Inner::Rounded(element), id: Id::new() })
            .collect();
        if shaped.len() == inside.len() {
            return shaped;
        }
    }
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
        .map(|inner| ToastElement { inner: Inner::Crop(inner), id: id.clone() })
        .into_iter()
        .collect()
}


/// The status island, drawn from the shell's window but only inside its rounded
/// shape, so that the applications show past its corners (a plain crop showed the
/// shell's own background there in square patches).
const ISLAND_SHADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// In the shell's buffer, in pixels: the whole buffer, the island, its corner radius.
uniform vec2 size;
uniform vec4 card;
uniform float radius;

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    // Distance outside a rounded rectangle; one pixel of soft edge.
    vec2 p = v_coords * size - (card.xy + card.zw * 0.5);
    vec2 q = abs(p) - card.zw * 0.5 + vec2(radius);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    color = color * clamp(0.5 - d, 0.0, 1.0) * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// The island's shader, compiled the first time it is needed.
#[derive(Default)]
pub struct IslandShader {
    program: Option<smithay::backend::renderer::gles::GlesTexProgram>,
    broken: bool,
}

/// The island on the display whose top left corner is at `origin`: the shell's
/// window inside the rounded `area`. `None` when there is nothing to draw, or the
/// shader is not available (the caller then draws a plain crop).
pub fn island_element(
    renderer: &mut GlesRenderer,
    shader: &mut IslandShader,
    shell: Option<&Window>,
    shape: Option<(Rectangle<i32, Logical>, i32)>,
    primary: Option<Point<i32, Logical>>,
    origin: Point<i32, Logical>,
    scale: f64,
) -> Option<smithay::backend::renderer::gles::element::TextureShaderElement> {
    use smithay::backend::renderer::{
        element::texture::TextureRenderElement,
        gles::{GlesTexture, Uniform, UniformName, UniformType, element::TextureShaderElement},
        utils::{RendererSurfaceStateUserData, import_surface_tree},
        Renderer, Texture,
    };
    let ((area, radius), shell, primary) = (shape?, shell?, primary?);
    if shader.broken {
        return None;
    }
    if shader.program.is_none() {
        let uniforms = [
            UniformName::new("size", UniformType::_2f),
            UniformName::new("card", UniformType::_4f),
            UniformName::new("radius", UniformType::_1f),
        ];
        match renderer.compile_custom_texture_shader(ISLAND_SHADER, &uniforms) {
            Ok(program) => shader.program = Some(program),
            Err(e) => {
                tracing::error!("cannot compile the island shader, drawing it square: {e:?}");
                shader.broken = true;
                return None;
            }
        }
    }
    let surface = shell.toplevel()?.wl_surface().clone();
    import_surface_tree(renderer, &surface).ok()?;
    let context = renderer.context_id();
    let (texture, buffer_scale, transform) = smithay::wayland::compositor::with_states(&surface, |states| {
        let data = states.data_map.get::<RendererSurfaceStateUserData>()?.lock().ok()?;
        Some((data.texture::<GlesTexture>(context.clone())?.clone(), data.buffer_scale(), data.buffer_transform()))
    })?;
    // Where the island is in the shell's window, which starts at `primary`.
    let within = Rectangle::new(area.loc - primary, area.size);
    let element = TextureRenderElement::from_static_texture(
        smithay::backend::renderer::element::Id::new(),
        context,
        (area.loc - origin).to_f64().to_physical(scale),
        texture.clone(),
        buffer_scale,
        transform,
        None,
        Some(within.to_f64()),
        Some(area.size),
        None,
        Kind::Unspecified,
    );
    let pixels = texture.size();
    let at = within.to_f64().upscale(buffer_scale as f64);
    let uniforms = vec![
        Uniform::new("size", (pixels.w as f32, pixels.h as f32)),
        Uniform::new("card", [at.loc.x as f32, at.loc.y as f32, at.size.w as f32, at.size.h as f32]),
        Uniform::new("radius", radius as f32 * buffer_scale as f32),
    ];
    Some(TextureShaderElement::new(element, shader.program.clone()?, uniforms))
}
