//! The hardware backend: runs the compositor directly on displays and input devices,
//! as a login session, instead of inside another desktop's window.
//!
//! It drives the boot GPU and every monitor connected to it, including ones plugged in
//! later. Modes and arrangement follow the layout saved in GNOME where there is one;
//! otherwise a laptop's own panel is the main display and the others extend to its right.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDevice, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            element::{
                Kind,
                memory::MemoryRenderBufferRenderElement,
                render_elements,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, primary_gpu},
    },
    desktop::space::SpaceRenderElements,
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{
            EventLoop,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{self, Device as ControlDevice, ModeFlags, ModeTypeFlags, connector, crtc, property},
        input::{self, Libinput},
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::{DeviceFd, IsAlive, Rectangle, Transform},
    wayland::{compositor::with_states, dmabuf::DmabufFeedbackBuilder},
};

use crate::{DesComp, monitors};
use des_common::ipc::{DisplayInfo, Mode as DisplayMode, Setup, Usage};

/// How long the first frame may take to reach a screen before the backend gives up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// What travels with each frame to its page flip: whom to tell it was shown.
type Feedback = Option<smithay::desktop::utils::OutputPresentationFeedback>;
type Manager =
    DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;
type Surface = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;

render_elements! {
    pub OutputElements<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Moving=smithay::backend::renderer::element::utils::RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    ClientCursor=WaylandSurfaceRenderElement<GlesRenderer>,
    Toast=crate::toast::ToastElement,
    Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
    Phosphor=smithay::backend::renderer::gles::element::TextureShaderElement,
    Cropped=smithay::backend::renderer::element::utils::CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
}

/// The pointer as something to draw at `position` on a display: the arrow, or the
/// image the application under it supplies.
pub fn cursor_elements(
    renderer: &mut GlesRenderer,
    cursors: &crate::handlers::cursor_shape::Cursors,
    status: &CursorImageStatus,
    position: smithay::utils::Point<f64, smithay::utils::Logical>,
    scale: f64,
) -> Vec<OutputElements> {
    let mut elements = Vec::new();
    match status {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .map(|attributes| attributes.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            elements.extend(render_elements_from_surface_tree(
                renderer,
                surface,
                (position - hotspot.to_f64()).to_physical(scale).to_i32_round(),
                scale,
                1.0,
                Kind::Cursor,
            ));
        }
        CursorImageStatus::Named(icon) => {
            let (picture, hotspot) = cursors.get(*icon);
            let at = (position - hotspot.to_f64()).to_physical(scale);
            if let Ok(pointer) = MemoryRenderBufferRenderElement::from_buffer(renderer, at, picture, None, None, None, Kind::Cursor) {
                elements.push(OutputElements::Cursor(pointer));
            }
        }
    }
    elements
}

/// The splash as something to draw on a display, in this backend's terms.
pub fn splash_elements(
    splash: &mut crate::splash::Splash,
    renderer: &mut GlesRenderer,
    size: smithay::utils::Size<i32, smithay::utils::Logical>,
    scale: f64,
) -> Vec<OutputElements> {
    splash
        .elements(renderer, size, scale)
        .into_iter()
        .map(|element| match element {
            crate::splash::TopElements::Buffer(buffer) => OutputElements::Cursor(buffer),
            crate::splash::TopElements::Solid(solid) => OutputElements::Solid(solid),
            crate::splash::TopElements::Toast(toast) => OutputElements::Toast(toast),
            crate::splash::TopElements::DragIcon(icon) => OutputElements::ClientCursor(icon),
            crate::splash::TopElements::Space(space) => OutputElements::Space(space),
            crate::splash::TopElements::Moving(moving) => OutputElements::Moving(moving),
            crate::splash::TopElements::Phosphor(frame) => OutputElements::Phosphor(frame),
            crate::splash::TopElements::Cropped(cropped) => OutputElements::Cropped(cropped),
            crate::splash::TopElements::_GenericCatcher(never) => match never {},
        })
        .collect()
}

/// One monitor and the means to draw on it.
struct Monitor {
    connector: connector::Handle,
    crtc: crtc::Handle,
    /// A laptop's built-in panel, which is preferred as the main display.
    internal: bool,
    surface: Surface,
    output: Output,
    global: GlobalId,
    refresh: Duration,
    /// The modes it can run at, best first.
    modes: Vec<DisplayMode>,
    /// How it was asked to be set: scaled, turned, mirroring.
    setup: Setup,
    /// When the frame currently waiting for its page flip was queued.
    pending_since: Option<Instant>,
    /// Whether a frame has reached this monitor yet.
    lit: bool,
    /// Not to be drawn before this: just after a refresh, the next frame waits until
    /// shortly before the one after, so that what it shows is fresh and frames come
    /// evenly spaced however this monitor's rate and the others' line up.
    draw_after: Option<Instant>,
    /// How long a frame for it has lately taken to draw.
    draw_cost: Duration,
    /// Since the last report: frames shown, the longest drawing, and when it began.
    shown: u32,
    worst: Duration,
    counting_since: Instant,
}

/// How often each monitor's frame rate is written to the log.
const REPORT_EVERY: Duration = Duration::from_secs(60);

/// How long after a refresh a monitor's next frame waits to be drawn: as late as
/// leaves three times what drawing has lately cost, and two milliseconds more, but
/// never past half the interval, so that a slow frame still makes the next refresh.
fn wait_after_refresh(refresh: Duration, draw_cost: Duration) -> Duration {
    refresh.saturating_sub(draw_cost * 3 + Duration::from_millis(2)).min(refresh / 2)
}

pub struct UdevData {
    pub renderer: GlesRenderer,
    /// Connected monitors, the main display first.
    monitors: Vec<Monitor>,
    /// Connected monitors that are turned off in the settings.
    dark: Vec<DisplayInfo>,
    manager: Manager,
    pub cursor: crate::handlers::cursor_shape::Cursors,
    /// False while another virtual terminal owns the displays.
    active: bool,
    /// Frames that have reached a screen.
    flips: u64,
    /// The input devices present, so that settings can be applied to them later.
    devices: Vec<input::Device>,
    // Declared last so that it is dropped last: the devices above were opened through it.
    pub session: LibSeatSession,
}

pub fn init_udev(
    event_loop: &mut EventLoop<'static, DesComp>,
    state: &mut DesComp,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &state.display_handle.clone();

    // The session grants access to the display and input devices, and takes it away
    // again while the user is on another virtual terminal.
    let (mut session, session_notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    let gpu_path = primary_gpu(&seat_name)?.ok_or("no GPU found for this seat")?;
    tracing::info!(gpu = %gpu_path.display(), seat = %seat_name, "starting on hardware");
    let fd = session.open(
        &gpu_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let egl_display = unsafe { EGLDisplay::new(gbm.clone())? };
    let render_node = EGLDevice::device_for_display(&egl_display)
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten())
        .or_else(|| DrmNode::from_path(&gpu_path).ok());
    let egl_context = EGLContext::new(&egl_display)?;
    let renderer = unsafe { GlesRenderer::new(egl_context)? };

    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let manager: Manager = DrmOutputManager::new(
        drm,
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
        GbmFramebufferExporter::new(gbm.clone(), render_node),
        Some(gbm),
        [Fourcc::Abgr8888, Fourcc::Argb8888],
        render_formats,
    );

    // Let clients hand over GPU buffers (see the winit backend for why feedback matters).
    let formats = renderer.dmabuf_formats();
    let feedback =
        render_node.and_then(|node| DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build().ok());
    state.dmabuf_global = Some(match &feedback {
        Some(feedback) => state
            .dmabuf_state
            .create_global_with_default_feedback::<DesComp>(display_handle, feedback),
        None => state.dmabuf_state.create_global::<DesComp>(display_handle, formats),
    });

    state.udev = Some(UdevData {
        renderer,
        monitors: Vec::new(),
        dark: Vec::new(),
        manager,
        cursor: crate::handlers::cursor_shape::Cursors::new(),
        active: true,
        flips: 0,
        devices: Vec::new(),
        session: session.clone(),
    });
    state.rescan_monitors();
    if state.udev.as_ref().is_some_and(|udev| udev.monitors.is_empty()) {
        return Err("no connected monitor could be set up".into());
    }

    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|_| "cannot assign the seat to libinput")?;
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), |mut event, _, data| {
            match &mut event {
                InputEvent::DeviceAdded { device } => {
                    configure_device(device, &data.input_settings);
                    if let Some(udev) = data.udev.as_mut() {
                        udev.devices.push(device.clone());
                    }
                }
                InputEvent::DeviceRemoved { device } => {
                    if let Some(udev) = data.udev.as_mut() {
                        udev.devices.retain(|known| known != device);
                    }
                }
                _ => {}
            }
            data.process_input_event(event);
        })?;

    event_loop
        .handle()
        .insert_source(session_notifier, move |event, _, data| {
            let Some(udev) = data.udev.as_mut() else {
                return;
            };
            match event {
                SessionEvent::PauseSession => {
                    tracing::info!("session paused");
                    libinput.suspend();
                    udev.manager.pause();
                    udev.active = false;
                }
                SessionEvent::ActivateSession => {
                    tracing::info!("session resumed");
                    if libinput.resume().is_err() {
                        tracing::error!("cannot resume input devices");
                    }
                    if let Err(e) = udev.manager.activate(false) {
                        tracing::error!("cannot take the displays back: {e}");
                    }
                    for monitor in &mut udev.monitors {
                        monitor.surface.reset_buffers();
                        monitor.pending_since = None;
                    }
                    udev.active = true;
                    // Monitors may have come or gone while we were away.
                    data.rescan_monitors();
                }
            }
        })?;

    event_loop
        .handle()
        .insert_source(drm_notifier, |event, metadata, data| match event {
            DrmEvent::VBlank(crtc) => {
                let Some(udev) = data.udev.as_mut() else {
                    return;
                };
                if let Some(monitor) = udev.monitors.iter_mut().find(|monitor| monitor.crtc == crtc) {
                    match monitor.surface.frame_submitted() {
                        Ok(Some(Some(mut feedback))) => {
                            crate::handlers::presentation::presented(&mut feedback, metadata.as_ref(), monitor.refresh)
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!("page flip bookkeeping failed: {e}"),
                    }
                    monitor.pending_since = None;
                    monitor.draw_after = Some(Instant::now() + wait_after_refresh(monitor.refresh, monitor.draw_cost));
                    monitor.shown += 1;
                    if monitor.counting_since.elapsed() >= REPORT_EVERY {
                        let seconds = monitor.counting_since.elapsed().as_secs_f64();
                        tracing::info!(
                            output = %monitor.output.name(),
                            fps = format!("{:.1}", monitor.shown as f64 / seconds),
                            rate = format!("{:.1}", 1.0 / monitor.refresh.as_secs_f64()),
                            draw_ms = format!("{:.1}", monitor.draw_cost.as_secs_f64() * 1000.0),
                            worst_ms = format!("{:.1}", monitor.worst.as_secs_f64() * 1000.0),
                            "frames"
                        );
                        (monitor.shown, monitor.worst, monitor.counting_since) = (0, Duration::ZERO, Instant::now());
                    }
                    if !std::mem::replace(&mut monitor.lit, true) {
                        tracing::info!(output = %monitor.output.name(), "first frame on screen");
                    }
                    udev.flips += 1;
                }
            }
            DrmEvent::Error(e) => tracing::error!("display error: {e}"),
        })?;

    // The kernel announces a plugged or unplugged monitor as a change to the GPU.
    event_loop
        .handle()
        .insert_source(UdevBackend::new(&seat_name)?, |_, _, data| {
            if data.udev.as_ref().is_some_and(|udev| udev.active) {
                data.rescan_monitors();
            }
        })?;

    // Frames are attempted once per refresh of the fastest monitor. Nothing is drawn or
    // queued for a monitor unless something on it changed, and never while its previous
    // frame is still waiting for its page flip.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), move |_, _, data| {
            let state = data;
            if !state.before_frame() {
                return TimeoutAction::Drop;
            }
            // Give up rather than sit at a black screen: the session launcher can then
            // fall back to another way of starting the desktop.
            let stuck = state.udev.as_ref().is_some_and(|udev| udev.active && udev.flips == 0);
            if stuck && state.start_time.elapsed() > STARTUP_TIMEOUT {
                state.fatal = Some("no frame reached the screen".to_string());
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }
            state.render_udev();
            state.record_tick();
            state.after_frame();
            let _ = state.display_handle.flush_clients();
            let frame_time = state
                .udev
                .as_ref()
                .and_then(|udev| udev.monitors.iter().map(|monitor| monitor.refresh).min())
                .unwrap_or(Duration::from_millis(16));
            // Woken as well when a monitor's next frame is due.
            let now = Instant::now();
            let due = state
                .udev
                .as_ref()
                .and_then(|udev| udev.monitors.iter().filter_map(|monitor| monitor.draw_after).filter(|at| *at > now).min())
                .map(|at| at - now);
            TimeoutAction::ToDuration(due.map_or(frame_time, |due| due.min(frame_time)))
        })?;

    Ok(())
}

/// Applies the settings to one input device: touchpads and mice each get their own.
fn configure_device(device: &mut input::Device, settings: &des_common::input::InputSettings) {
    // Only touchpads can tap.
    let touchpad = device.config_tap_finger_count() > 0;
    if touchpad {
        let _ = device.config_tap_set_enabled(settings.touchpad_tap);
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(settings.touchpad_while_typing_off);
        }
    } else if !device.has_capability(input::DeviceCapability::Pointer) {
        return;
    }
    let (natural, speed) = if touchpad {
        (settings.touchpad_natural_scroll, settings.touchpad_speed)
    } else {
        (settings.mouse_natural_scroll, settings.mouse_speed)
    };
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(natural);
    }
    if device.config_accel_is_available() {
        let _ = device.config_accel_set_speed(speed as f64);
    }
    if !touchpad && device.config_left_handed_is_available() {
        let _ = device.config_left_handed_set(settings.mouse_left_handed);
    }
}

/// The modes a monitor offers, without interlaced ones or repeats, largest and fastest
/// first.
fn offered_modes(info: &connector::Info, in_use: Option<(u16, u16)>) -> Vec<DisplayMode> {
    let mut modes: Vec<DisplayMode> = Vec::new();
    for mode in info.modes() {
        if mode.flags().contains(ModeFlags::INTERLACE) {
            continue;
        }
        let wl = WlMode::from(*mode);
        let offered = DisplayMode {
            width: wl.size.w as u32,
            height: wl.size.h as u32,
            refresh: wl.refresh as u32,
        };
        if offered.refresh > 0 && !modes.contains(&offered) {
            modes.push(offered);
        }
    }
    let (width, height) = info.size().unwrap_or((0, 0));
    let mut modes = in_shape(modes, (width, height), in_use);
    modes.sort_by_key(|mode| std::cmp::Reverse((mode.width * mode.height, mode.refresh)));
    modes.truncate(48);
    modes
}

/// Leaves out modes of another shape than the screen where one of the same height has
/// its shape: 4096 by 2160 on a 16:9 TV, which would only be squeezed onto it, when it
/// also offers 3840 by 2160. Other shapes with no such twin, such as 4:3, stay; so
/// does the size in use. A screen whose size is not known keeps everything.
fn in_shape(modes: Vec<DisplayMode>, physical: (u32, u32), in_use: Option<(u16, u16)>) -> Vec<DisplayMode> {
    if physical.0 < 100 || physical.1 < 60 {
        return modes;
    }
    let screen = physical.0 as f64 / physical.1 as f64;
    let shaped = |mode: &DisplayMode| ((mode.width as f64 / mode.height.max(1) as f64) / screen - 1.0).abs() < 0.05;
    let keep: Vec<bool> = modes
        .iter()
        .map(|mode| {
            shaped(mode)
                || in_use == Some((mode.width as u16, mode.height as u16))
                || !modes.iter().any(|other| other.height == mode.height && shaped(other))
        })
        .collect();
    modes.into_iter().zip(keep).filter_map(|(mode, keep)| keep.then_some(mode)).collect()
}

/// What a monitor's EDID says about how it may be driven beyond its modes.
#[derive(Debug, Default, PartialEq)]
struct Sink {
    /// It can be told whether colour comes in the full range or the TV range, 16 to 235.
    full_range: bool,
    /// It has a mode for computer graphics, which on a TV turns off overscan and sharpening.
    graphics: bool,
}

/// Reads the CTA extension blocks of an EDID, where a TV says what it supports.
fn sink_of(edid: &[u8]) -> Sink {
    let mut sink = Sink::default();
    for block in edid.chunks_exact(128).skip(1).filter(|block| block[0] == 0x02) {
        let end = (block[2] as usize).clamp(4, 127);
        let mut at = 4;
        while at < end {
            let (tag, length) = (block[at] >> 5, (block[at] & 0x1f) as usize);
            let Some(data) = block.get(at + 1..at + 1 + length) else {
                break;
            };
            match (tag, data) {
                // Video capability: whether the RGB range can be chosen.
                (7, [0, flags, ..]) => sink.full_range = flags & 0x40 != 0,
                // HDMI 1.4: the content types it understands, graphics the lowest bit.
                (3, [0x03, 0x0c, 0x00, _, _, _, _, content, ..]) => sink.graphics = content & 0x01 != 0,
                _ => {}
            }
            at += 1 + length;
        }
    }
    sink
}

/// Sets up a monitor's connector for the sharpest picture its EDID allows: full-range
/// colour where the monitor can be told about it, which the driver would otherwise
/// squeeze into the TV range for TV resolutions, and on a TV the mode for computer
/// graphics instead of the one for film.
fn tune_connector(drm: &impl ControlDevice, connector: connector::Handle, name: &str) {
    let Ok(properties) = drm.get_properties(connector) else {
        return;
    };
    let properties: Vec<(property::Info, property::RawValue)> = properties
        .iter()
        .filter_map(|(handle, value)| Some((drm.get_property(*handle).ok()?, *value)))
        .collect();
    let edid = properties
        .iter()
        .find(|(info, _)| info.name().to_bytes() == b"EDID")
        .and_then(|(_, blob)| drm.get_property_blob(*blob).ok())
        .unwrap_or_default();
    let sink = sink_of(&edid);
    let wanted = [("Broadcast RGB", "Full", sink.full_range), ("content type", "Graphics", sink.graphics)];
    for (property, choice, _) in wanted.into_iter().filter(|(_, _, supported)| *supported) {
        let Some((info, _)) = properties.iter().find(|(info, _)| info.name().to_bytes() == property.as_bytes()) else {
            continue;
        };
        let property::ValueType::Enum(choices) = info.value_type() else {
            continue;
        };
        let (_, choices) = choices.values();
        let Some(value) = choices.iter().find(|value| value.name().to_bytes() == choice.as_bytes()) else {
            continue;
        };
        match drm.set_property(connector, info.handle(), value.value()) {
            Ok(()) => tracing::info!(output = %name, property, choice, "connector set for a sharper picture"),
            Err(e) => tracing::warn!(output = %name, property, "cannot set: {e}"),
        }
    }
}

/// The mode to start a monitor in when nothing is saved for it, given each mode's
/// width, height, refresh in millihertz and whether the monitor prefers it: the one
/// it prefers or, when its driver leaves that out because the cable or port cannot
/// carry it, the largest mode in the shape of the screen that is smooth enough to
/// work at, at least 50 Hz.
fn default_mode(modes: &[(u16, u16, u32, bool)], physical: (u32, u32)) -> Option<usize> {
    if let Some(preferred) = modes.iter().position(|mode| mode.3) {
        return Some(preferred);
    }
    let shaped = |mode: &(u16, u16, u32, bool)| {
        let (width, height) = physical;
        width == 0 || height == 0 || {
            let screen = width as f64 / height as f64;
            ((mode.0 as f64 / mode.1.max(1) as f64) / screen - 1.0).abs() < 0.05
        }
    };
    let smooth = |mode: &(u16, u16, u32, bool)| mode.2 >= 49_000;
    let fits = |mode: &(u16, u16, u32, bool)| (shaped(mode), smooth(mode));
    // Shape and smoothness, where any mode has them, before size and speed.
    let best = modes.iter().map(fits).max()?;
    modes
        .iter()
        .enumerate()
        .filter(|(_, mode)| fits(mode) == best)
        .max_by_key(|(_, mode)| (mode.0 as u32 * mode.1 as u32, mode.2))
        .map(|(index, _)| index)
}

/// The scale for a monitor nobody has set one for, from its resolution and its size
/// in millimetres: one that brings it near the sharpness things are drawn for, about
/// 110 dots an inch on a desk and 140 on a laptop, which is held closer. A screen
/// over 40 inches is a TV, seen from across the room, and is scaled to show as much
/// as 1080 lines; so is a monitor whose size is unknown. Nothing is drawn so large
/// that fewer than 720 lines fit.
fn auto_scale(size: (i32, i32), physical: (i32, i32), internal: bool) -> u32 {
    let (width, height) = (size.0.max(1) as f64, size.1.max(1) as f64);
    // Some monitors give only their shape here, as in 16 by 9 centimetres.
    let known = physical.0 >= 100 && physical.1 >= 60;
    let inches = (physical.0 as f64).hypot(physical.1 as f64) / 25.4;
    let wanted = if !known || inches > 40.0 {
        height / 1080.0
    } else {
        let dpi = width / (physical.0 as f64 / 25.4);
        dpi / if internal { 140.0 } else { 110.0 }
    };
    let fits = |scale: &&u32| height * 100.0 / **scale as f64 >= 720.0;
    Setup::SCALES
        .iter()
        .filter(fits)
        .min_by_key(|scale| ((**scale as f64 / 100.0 - wanted).abs() * 1000.0) as u64)
        .copied()
        .unwrap_or(100)
}

/// How a saved layout sets a monitor beyond its mode.
fn setup_of(saved: &monitors::Saved) -> Setup {
    Setup {
        scale: (saved.scale * 100.0).round() as u32,
        turn: saved.turn,
        usage: saved.usage,
    }
}

/// The kernel's name for a connector, such as `HDMI-A-1`.
fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

impl DesComp {
    /// Brings the set of monitors in line with what is plugged in, then lays them out.
    fn rescan_monitors(&mut self) {
        let Some(udev) = self.udev.as_mut() else {
            return;
        };
        let (resources, connected) = {
            let drm = udev.manager.device();
            let resources = match drm.resource_handles() {
                Ok(resources) => resources,
                Err(e) => {
                    tracing::warn!("cannot list display connectors: {e}");
                    return;
                }
            };
            let connected: Vec<connector::Info> = resources
                .connectors()
                .iter()
                .filter_map(|handle| drm.get_connector(*handle, true).ok())
                .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
                .collect();
            (resources, connected)
        };

        let names: Vec<String> = connected
            .iter()
            .map(|info| monitors::gnome_name(&connector_name(info)))
            .collect();
        // A layout just chosen in the settings comes before the saved ones.
        let saved = match &self.display_override {
            Some(layout) if layout.len() == names.len() && names.iter().all(|name| layout.iter().any(|saved| &saved.connector == name)) => {
                monitors::Layout {
                    monitors: layout.clone(),
                    arranged: true,
                }
            }
            _ => monitors::pick(&monitors::load_all(), &names),
        };

        // Monitors turned off in the settings are left alone, unless that is all of them.
        let off = |name: &str| saved.get(&monitors::gnome_name(name)).is_some_and(|saved| saved.own && saved.usage == Usage::Off);
        let any_on = connected.iter().any(|info| !off(&connector_name(info)));
        let off = |name: &str| any_on && off(name);
        udev.dark.clear();

        let mut index = 0;
        while index < udev.monitors.len() {
            let monitor = &udev.monitors[index];
            if connected.iter().any(|info| info.handle() == monitor.connector) && !off(&monitor.output.name()) {
                index += 1;
                continue;
            }
            let gone = udev.monitors.remove(index);
            tracing::info!(output = %gone.output.name(), "monitor disconnected or turned off");
            // Dropping the surface is not enough to stop the monitor showing its last picture.
            if let Err(e) = gone.surface.with_compositor(|compositor| compositor.clear()) {
                tracing::warn!("cannot turn the screen off: {e}");
            }
            self.space.unmap_output(&gone.output);
            self.display_handle.remove_global::<DesComp>(gone.global.clone());
        }

        for info in &connected {
            if udev.monitors.iter().any(|monitor| monitor.connector == info.handle()) {
                continue;
            }
            let name = connector_name(info);
            if off(&name) {
                let modes = offered_modes(info, None);
                let wanted = saved.get(&monitors::gnome_name(&name));
                let current = wanted
                    .and_then(|saved| {
                        modes
                            .iter()
                            .filter(|mode| (mode.width as i32, mode.height as i32) == (saved.width, saved.height))
                            .min_by_key(|mode| (mode.refresh as f64 - saved.rate * 1000.0).abs() as u64)
                    })
                    .or(modes.first())
                    .copied();
                if let Some(current) = current {
                    udev.dark.push(DisplayInfo {
                        name,
                        main: false,
                        x: i32::MAX,
                        y: 0,
                        current,
                        modes,
                        setup: wanted.map_or(Setup::default(), setup_of),
                    });
                }
                continue;
            }
            // Each monitor needs a CRTC of its own, out of those its connector can use.
            let crtc = info
                .encoders()
                .iter()
                .filter_map(|encoder| udev.manager.device().get_encoder(*encoder).ok())
                .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
                .find(|crtc| !udev.monitors.iter().any(|monitor| monitor.crtc == *crtc));
            let Some(crtc) = crtc else {
                tracing::warn!(output = %name, "no free CRTC for this monitor");
                continue;
            };
            // A mode the user already runs this monitor in beats the one it advertises as
            // its best, which the cable or the monitor may not actually manage.
            let wanted = saved.get(&monitors::gnome_name(&name));
            let drm_mode = wanted
                .and_then(|saved| {
                    info.modes()
                        .iter()
                        .filter(|mode| mode.size() == (saved.width as u16, saved.height as u16))
                        .filter(|mode| !mode.flags().contains(ModeFlags::INTERLACE))
                        .min_by_key(|mode| (WlMode::from(**mode).refresh as f64 - saved.rate * 1000.0).abs() as u64)
                })
                .or_else(|| {
                    let modes: Vec<&control::Mode> =
                        info.modes().iter().filter(|mode| !mode.flags().contains(ModeFlags::INTERLACE)).collect();
                    let described: Vec<(u16, u16, u32, bool)> = modes
                        .iter()
                        .map(|mode| {
                            let (width, height) = mode.size();
                            (width, height, WlMode::from(**mode).refresh as u32, mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                        })
                        .collect();
                    default_mode(&described, info.size().unwrap_or((0, 0))).map(|index| modes[index])
                })
                .copied()
                .unwrap_or(info.modes()[0]);
            let wl_mode = WlMode::from(drm_mode);
            let internal = matches!(
                info.interface(),
                connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS | connector::Interface::DSI
            );
            if !internal {
                tune_connector(udev.manager.device(), info.handle(), &name);
            }

            // Only the primary and cursor planes are used: every frame is composited by
            // the renderer, which keeps this backend simple.
            let mut planes = match udev.manager.device().planes(&crtc) {
                Ok(planes) => planes,
                Err(e) => {
                    tracing::warn!(output = %name, "cannot query planes: {e}");
                    continue;
                }
            };
            planes.overlay.clear();

            let (phys_w, phys_h) = info.size().unwrap_or((0, 0));
            let output = Output::new(
                name.clone(),
                PhysicalProperties {
                    size: (phys_w as i32, phys_h as i32).into(),
                    subpixel: info.subpixel().into(),
                    make: "Unknown".into(),
                    model: "Unknown".into(),
                },
            );
            output.set_preferred(wl_mode);
            output.change_current_state(Some(wl_mode), None, None, None);

            let surface = match udev.manager.initialize_output::<_, OutputElements>(
                crtc,
                drm_mode,
                &[info.handle()],
                &output,
                Some(planes),
                &mut udev.renderer,
                &DrmOutputRenderElements::default(),
            ) {
                Ok(surface) => surface,
                Err(e) => {
                    tracing::warn!(output = %name, "cannot set up this monitor: {e}");
                    continue;
                }
            };
            tracing::info!(output = %name, mode = ?wl_mode, from_saved_layout = wanted.is_some(), "monitor connected");
            udev.monitors.push(Monitor {
                modes: offered_modes(info, Some(drm_mode.size())),
                connector: info.handle(),
                crtc,
                internal,
                surface,
                global: output.create_global::<DesComp>(&self.display_handle),
                output,
                refresh: Duration::from_secs_f64(1000.0 / wl_mode.refresh.max(1) as f64),
                setup: Setup::default(),
                pending_since: None,
                lit: false,
                draw_after: None,
                draw_cost: Duration::from_millis(4),
                shown: 0,
                worst: Duration::ZERO,
                counting_since: Instant::now(),
            });
        }

        // Arrange the monitors as saved, if what is saved describes exactly this set.
        let place = |monitor: &Monitor| saved.get(&monitors::gnome_name(&monitor.output.name()));
        let arranged = saved.arranged && udev.monitors.iter().all(|monitor| place(monitor).is_some());
        // Scale, turn and mirroring are this desktop's own settings, never GNOME's.
        for monitor in udev.monitors.iter_mut() {
            let own = saved.get(&monitors::gnome_name(&monitor.output.name())).filter(|saved| saved.own);
            monitor.setup = match own {
                Some(saved) => setup_of(saved),
                // Positions saved in GNOME allow for things at their own size.
                None if arranged => Setup::default(),
                // Never set up here: things as large as suit the monitor's size.
                None => {
                    let size = monitor.output.current_mode().map_or((0, 0), |mode| (mode.size.w, mode.size.h));
                    let physical = monitor.output.physical_properties().size;
                    Setup { scale: auto_scale(size, (physical.w, physical.h), monitor.internal), ..Setup::default() }
                }
            };
        }
        if arranged {
            udev.monitors.sort_by_key(|monitor| {
                place(monitor).map(|saved| (!saved.primary, monitor.setup.usage == Usage::Mirror, saved.x, saved.y))
            });
        } else {
            // The built-in panel leads; everything else follows it, left to right.
            udev.monitors.sort_by_key(|monitor| (monitor.setup.usage == Usage::Mirror, !monitor.internal));
        }
        // Whatever leads is a display of its own: there has to be one.
        if let Some(main) = udev.monitors.first_mut() {
            main.setup.usage = Usage::Own;
        }
        let mut screens: Vec<Rectangle<i32, smithay::utils::Logical>> = Vec::new();
        let mut next_x = 0;
        for monitor in &udev.monitors {
            let Some(mode) = monitor.output.current_mode() else {
                continue;
            };
            let transform = match monitor.setup.turn {
                90 => Transform::_90,
                180 => Transform::_180,
                270 => Transform::_270,
                _ => Transform::Normal,
            };
            let turned = transform.transform_size(mode.size);
            let mirrored = screens.first().copied().filter(|_| monitor.setup.usage == Usage::Mirror);
            let (scale, position) = match mirrored {
                // A mirror shows the main display whole: as large as fits its own shape.
                Some(main) => {
                    let scale = (turned.w as f64 / main.size.w.max(1) as f64).min(turned.h as f64 / main.size.h.max(1) as f64);
                    (scale.max(0.1), (main.loc.x, main.loc.y))
                }
                None => {
                    let position = match place(monitor) {
                        Some(saved) if arranged => (saved.x, saved.y),
                        _ => (next_x, 0),
                    };
                    (monitor.setup.scale as f64 / 100.0, position)
                }
            };
            monitor.output.change_current_state(
                None,
                Some(transform),
                Some(smithay::output::Scale::Fractional(scale)),
                Some(position.into()),
            );
            self.space.map_output(&monitor.output, position);
            if mirrored.is_none() {
                let size = turned.to_f64().to_logical(scale).to_i32_round::<i32>();
                next_x += size.w;
                screens.push(Rectangle::new(position.into(), size));
            }
        }
        self.set_screens(screens);
    }

    /// Applies the input settings in force to every device present.
    pub fn configure_devices(&mut self) {
        let settings = self.input_settings.clone();
        if let Some(udev) = self.udev.as_mut() {
            for device in &mut udev.devices {
                configure_device(device, &settings);
            }
        }
    }

    /// The monitors being driven, left to right; `None` when this backend is not in use.
    pub fn hardware_displays(&self) -> Option<Vec<DisplayInfo>> {
        let udev = self.udev.as_ref()?;
        let mut displays: Vec<DisplayInfo> = udev
            .monitors
            .iter()
            .enumerate()
            .filter_map(|(index, monitor)| {
                let mode = monitor.output.current_mode()?;
                let location = self.space.output_geometry(&monitor.output)?.loc;
                Some(DisplayInfo {
                    name: monitor.output.name(),
                    // The main display is kept first.
                    main: index == 0,
                    x: location.x,
                    y: location.y,
                    current: DisplayMode {
                        width: mode.size.w as u32,
                        height: mode.size.h as u32,
                        refresh: mode.refresh as u32,
                    },
                    modes: monitor.modes.clone(),
                    setup: monitor.setup,
                })
            })
            .collect();
        displays.extend(udev.dark.iter().cloned());
        // Left to right; a mirror follows what it shows, and the dark ones come last.
        displays.sort_by_key(|display| (display.x, display.setup.usage != Usage::Own));
        Some(displays)
    }

    /// Makes the monitors follow the layout now in force. A monitor whose mode has to
    /// change is set up afresh, the way a newly plugged one is.
    pub fn reapply_displays(&mut self) {
        let Some(udev) = self.udev.as_mut() else {
            return;
        };
        let wanted = self.display_override.clone().unwrap_or_default();
        let mut index = 0;
        while index < udev.monitors.len() {
            let monitor = &udev.monitors[index];
            let name = monitors::gnome_name(&monitor.output.name());
            let current = monitor.output.current_mode();
            let differs = wanted.iter().find(|saved| saved.connector == name).is_none_or(|saved| {
                current.is_none_or(|mode| {
                    (mode.size.w, mode.size.h) != (saved.width, saved.height)
                        || (mode.refresh as f64 - saved.rate * 1000.0).abs() > 1.0
                })
            });
            if differs {
                let gone = udev.monitors.remove(index);
                self.space.unmap_output(&gone.output);
                self.display_handle.remove_global::<DesComp>(gone.global.clone());
            } else {
                index += 1;
            }
        }
        self.rescan_monitors();
    }

    /// Turns every screen off, or back on. Off, nothing is drawn; on, the next frame
    /// lights them again.
    pub fn blank(&mut self, off: bool) {
        self.screens_off = off;
        let Some(udev) = self.udev.as_mut() else {
            tracing::info!(off, "screens would be switched, were this not a window");
            return;
        };
        for monitor in udev.monitors.iter_mut() {
            monitor.pending_since = None;
            if off {
                if let Err(e) = monitor.surface.with_compositor(|compositor| compositor.clear()) {
                    tracing::warn!("cannot turn a screen off: {e}");
                }
            } else {
                // Everything is drawn afresh, which is what switches the screen on.
                monitor.surface.reset_buffers();
            }
        }
    }

    fn render_udev(&mut self) {
        if self.screens_off {
            return;
        }
        let toast_area = self.toast_area();
        let overlay_area = self.overlay_area();
        let docks = self.dock_layers();
        let primary = self.screens.first().map(|screen| screen.loc);
        let now = Instant::now();
        let beneath: Vec<&smithay::desktop::Window> = self.shell.iter().chain(self.backdrops.iter()).collect();
        let layers = self.beneath();
        self.phosphor.begin_frame();
        let Some(udev) = self.udev.as_mut() else {
            return;
        };
        if !udev.active {
            return;
        }
        if matches!(&self.cursor_status, CursorImageStatus::Surface(surface) if !surface.alive()) {
            self.cursor_status = CursorImageStatus::default_named();
        }
        let pointer = self.seat.get_pointer().unwrap().current_location();
        let UdevData {
            renderer,
            monitors,
            cursor,
            ..
        } = udev;

        // A display that was unplugged lets go of its off-screen frame.
        let names: Vec<String> = monitors.iter().map(|monitor| monitor.output.name()).collect();
        self.phosphor.retain(&names);

        for monitor in monitors.iter_mut() {
            // A page flip that never completes must not freeze the screen for good.
            match monitor.pending_since {
                Some(since) if since.elapsed() < Duration::from_secs(1) => continue,
                Some(_) => tracing::warn!("page flip timed out, drawing again"),
                None => {}
            }
            if monitor.draw_after.is_some_and(|at| Instant::now() < at) {
                continue;
            }
            monitor.draw_after = None;
            let began = Instant::now();
            let Some(geometry) = self.space.output_geometry(&monitor.output) else {
                continue;
            };

            // Front to back: the pointer is above everything.
            let mut elements: Vec<OutputElements> = Vec::new();
            let scale = monitor.output.current_scale().fractional_scale();
            // The splash covers everything, the pointer included.
            if let Some(splash) = self.splash.as_mut() {
                elements.extend(splash_elements(splash, renderer, geometry.size, scale));
            } else if geometry.to_f64().contains(pointer) {
                let position = pointer - geometry.loc.to_f64();
                // Over a divider between windows, or dragging one, the double arrow.
                let status = if self.resize_hover {
                    CursorImageStatus::Named(smithay::input::pointer::CursorIcon::ColResize)
                } else {
                    self.cursor_status.clone()
                };
                elements.extend(cursor_elements(renderer, cursor, &status, position, scale));
                elements.extend(crate::dnd::icon_elements::<OutputElements>(renderer,self.drag_icon.as_ref(),position,scale));
            }
            // The overview, or the switcher's strip, above all but the pointer. The
            // overview is everything on its display; the switcher sits over the desktop.
            let overview = self.overview.as_ref().filter(|overview| overview.layout.display == geometry);
            if let Some(overview) = overview {
                elements.extend(crate::overview::elements::<OutputElements>(renderer, overview, &self.space, if overview.screen==0 {self.shell.as_ref()}else{self.backdrops.get(overview.screen-1)}, &monitor.output, now));
            }
            if !overview.is_some_and(|overview| matches!(overview.kind, des_common::ipc::OverviewKind::Workspaces | des_common::ipc::OverviewKind::Drag)) {
            elements.extend(
                crate::toast::elements(renderer, self.shell.as_ref(), toast_area, primary, &self.toast_id, geometry.loc, scale)
                    .into_iter()
                    .map(OutputElements::Toast),
            );
            for (shell,area,origin,id) in &docks {
                elements.extend(crate::toast::elements(renderer,Some(shell),Some(*area),Some(*origin),id,geometry.loc,scale).into_iter().map(OutputElements::Toast));
            }
            // The overlay is beneath the toasts and the dock, above every window.
            elements.extend(
                crate::toast::elements(renderer, self.shell.as_ref(), overlay_area, primary, &self.overlay_id, geometry.loc, scale)
                    .into_iter()
                    .map(OutputElements::Toast),
            );
            // Bars and launchers above the windows; wallpapers and the like beneath them.
            elements.extend(crate::handlers::layer_shell::layer_elements::<OutputElements>(renderer, &self.space, &monitor.output, true, layers));
            let under = crate::handlers::layer_shell::layer_elements::<OutputElements>(renderer, &self.space, &monitor.output, false, layers);
            elements.extend(crate::motion::window_elements::<OutputElements>(renderer, &self.space, &monitor.output, &self.motions, &beneath, under, now));
            }

            // Glow, scanlines and the night light: the frame drawn off screen, then shown
            // through one shader. With none of them on, the scene is drawn as it is.
            let size = monitor
                .output
                .current_mode()
                .map(|mode| monitor.output.current_transform().transform_size(mode.size))
                .unwrap_or_default();
            match self.phosphor.process(renderer, &monitor.output.name(), size, geometry.size, scale, &elements, false) {
                crate::phosphor::Pass::Direct => {}
                crate::phosphor::Pass::Unchanged => continue,
                crate::phosphor::Pass::Draw(frame) => elements = vec![OutputElements::Phosphor(frame)],
            }

            let drawn = monitor
                .surface
                .render_frame(renderer, &elements, [0.0, 0.0, 0.0, 1.0], FrameFlags::empty())
                .map(|result| !result.is_empty);
            match drawn {
                Ok(true) => match monitor.surface.queue_frame(Some(crate::handlers::presentation::feedback_for(&self.space, &monitor.output))) {
                    Ok(()) => {
                        monitor.pending_since = Some(Instant::now());
                        let cost = began.elapsed();
                        monitor.draw_cost = (monitor.draw_cost * 3 + cost) / 4;
                        monitor.worst = monitor.worst.max(cost);
                    }
                    Err(e) => tracing::error!("cannot present frame: {e}"),
                },
                Ok(false) => {}
                Err(e) => tracing::error!("cannot draw frame: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CTA extension of a 4K TV's EDID, as read from the TV itself.
    const TV: &str = "020362f052616066653f5f5e5d1f2220051404130312102f09070715075057070067040035072883010000e200cb6e030c001000b844210080010203046ad85dc40178886f023090e305e301e20f0fe3060f01e5018b849001681a00000101309000000000000000000000000000000000000000000000000000000000000054";

    fn edid(extension: &str) -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes.extend((0..extension.len()).step_by(2).map(|at| u8::from_str_radix(&extension[at..at + 2], 16).unwrap()));
        bytes
    }

    #[test]
    fn reads_what_a_tv_supports() {
        assert_eq!(sink_of(&edid(TV)), Sink { full_range: true, graphics: true });
        // A monitor without an extension says nothing, and is left as the driver sets it.
        assert_eq!(sink_of(&[0; 128]), Sink::default());
        // Nor does a truncated or nonsensical block upset anything.
        assert_eq!(sink_of(&edid(&TV[..40])), Sink::default());
        let mut lying = edid(TV);
        lying[130] = 0xff;
        sink_of(&lying);
    }

    #[test]
    fn scales_a_new_monitor_to_suit_its_size() {
        // A 15.6 inch 1080p laptop and a 24 inch 1080p monitor are fine as they are.
        assert_eq!(auto_scale((1920, 1080), (344, 194), true), 100);
        assert_eq!(auto_scale((1920, 1080), (531, 299), false), 100);
        // A 13 inch laptop at 2560 by 1600, and 4K monitors at 27 and 32 inches.
        assert_eq!(auto_scale((2560, 1600), (286, 179), true), 150);
        assert_eq!(auto_scale((3840, 2160), (597, 336), false), 150);
        assert_eq!(auto_scale((3840, 2160), (708, 398), false), 125);
        // A 55 inch TV shows as much as 1080 lines, whether at 4K or at 1080p.
        assert_eq!(auto_scale((3840, 2160), (1210, 680), false), 200);
        assert_eq!(auto_scale((1920, 1080), (1210, 680), false), 100);
        // Unknown size, or only a shape: by its lines alone.
        assert_eq!(auto_scale((3840, 2160), (0, 0), false), 200);
        assert_eq!(auto_scale((2560, 1440), (16, 9), false), 125);
        // A tiny, dense panel is never scaled past 720 lines.
        assert_eq!(auto_scale((1280, 800), (120, 75), true), 100);
    }

    #[test]
    fn offers_modes_in_the_shape_of_the_screen() {
        let mode = |width, height, refresh| DisplayMode { width, height, refresh };
        let tv = vec![mode(4096, 2160, 24_000), mode(3840, 2160, 30_000), mode(1920, 1080, 120_000), mode(1024, 768, 60_000)];
        let shaped = in_shape(tv.clone(), (1210, 680), None);
        assert_eq!(shaped, [mode(3840, 2160, 30_000), mode(1920, 1080, 120_000), mode(1024, 768, 60_000)], "4:3 has no twin, and stays");
        // What is in use stays offered, so the settings can show it.
        assert_eq!(in_shape(tv.clone(), (1210, 680), Some((4096, 2160))).len(), 4);
        assert_eq!(in_shape(tv.clone(), (16, 9), None).len(), 4, "a size that is only a shape tells nothing");
    }

    #[test]
    fn draws_late_but_in_time() {
        let ms = Duration::from_millis;
        // At 30 Hz, drawing that costs 4 ms waits half the interval.
        assert_eq!(wait_after_refresh(Duration::from_micros(33_334), ms(4)), Duration::from_micros(16_667));
        // At 120 Hz there is little room to wait; at worst, none.
        assert_eq!(wait_after_refresh(Duration::from_micros(8_333), ms(2)), Duration::from_micros(333));
        assert_eq!(wait_after_refresh(ms(16), ms(10)), Duration::ZERO);
    }

    #[test]
    fn starts_a_monitor_in_a_mode_it_can_work_at() {
        // The TV wants 4K at 60, which this port cannot carry, so the driver lists
        // nothing as preferred: 1080p at 120 beats 4K at 30 and the wider 4096 at 24.
        let tv = [(4096, 2160, 24_000, false), (3840, 2160, 30_000, false), (1920, 1080, 120_000, false), (1920, 1080, 60_000, false), (1280, 720, 60_000, false)];
        assert_eq!(default_mode(&tv, (1210, 680)), Some(2));
        // What the monitor prefers wins.
        let mut preferred = tv;
        preferred[1].3 = true;
        assert_eq!(default_mode(&preferred, (1210, 680)), Some(1));
        // Without a size, shape cannot count; with nothing smooth, the largest will do.
        assert_eq!(default_mode(&tv, (0, 0)), Some(2));
        assert_eq!(default_mode(&[(3840, 2160, 30_000, false), (1920, 1080, 30_000, false)], (600, 340)), Some(0));
        assert_eq!(default_mode(&[], (0, 0)), None);
    }
}
