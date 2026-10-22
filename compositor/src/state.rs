use std::{
    ffi::OsString,
    path::PathBuf,
    process::Child,
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{renderer::gles::GlesRenderer, winit::WinitGraphicsBackend},
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState, pointer::CursorImageStatus},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        dmabuf::{DmabufGlobal, DmabufState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::{
    ipc::IpcServer,
    policy::{App, ScreenMode},
    udev::UdevData,
};
use des_common::{Panels, ipc::Place};
use smithay::{reexports::calloop::LoopHandle, wayland::xwayland_shell::XWaylandShellState, xwayland::X11Wm};

/// How to start the shell.
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// The shell may crash this often within [`CRASH_WINDOW`] before the session gives up.
const MAX_CRASHES: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(30);

pub struct DesComp {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    pub loop_handle: LoopHandle<'static, DesComp>,
    /// The windowed backend, when running inside another desktop.
    pub backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    /// The hardware backend, when running as a login session.
    pub udev: Option<UdevData>,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub xwayland_shell_state: XWaylandShellState,
    /// The X11 window manager, once the X server is up.
    pub xwm: Option<X11Wm>,
    /// The X server's display number, for `DISPLAY`.
    pub x_display: Option<u32>,
    /// X11 menus, tooltips and the like: windows that place themselves.
    pub x_popups: Vec<Window>,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<DesComp>,
    pub data_device_state: DataDeviceState,
    /// Lets clipboard tools (such as `wl-copy`, which screenshots use) set and read
    /// the clipboard without a window of their own.
    pub data_control_state: smithay::wayland::selection::wlr_data_control::DataControlState,
    /// Select, then middle-click to paste.
    pub primary_selection_state: smithay::wayland::selection::primary_selection::PrimarySelectionState,
    /// Requests to bring a window forward, as a link opened elsewhere asks of the browser.
    pub xdg_activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    /// One application's window as the parent of another's.
    pub xdg_foreign_state: smithay::wayland::xdg_foreign::XdgForeignState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,

    // Desktop policy
    pub shell_spec: Option<ShellSpec>,
    pub shell_child: Option<Child>,
    /// Keeps terminal work alive across replacement shell processes.
    terminal_runtime: Option<des_common::terminal::Runtime>,
    /// When the shell last died without being asked to.
    shell_crashes: Vec<Instant>,
    /// Set while a restart of the shell was asked for and its old process is going away.
    shell_restarting: bool,
    pub shell_pid: Option<u32>,
    /// Input methods, pointer constraints, shortcut inhibitors, the virtual keyboard.
    pub input_protocols: crate::handlers::input_protocols::InputProtocols,
    /// The des-ui shell window: always fullscreen, always at the bottom.
    pub shell: Option<Window>,
    /// What is shown while the shell is being restarted, until it has a window again.
    pub splash: Option<crate::splash::Splash>,
    /// When a key, button or pointer was last used.
    pub last_input: Instant,
    /// Lines for the shell that arose while its socket was being read.
    pub unsaid: Vec<String>,
    /// When the windowed backend last drew a frame.
    pub last_frame: Instant,
    /// The recording under way, if any.
    pub recording: Option<crate::capture::Recording>,
    /// How many screenshots have been taken, to name each one's file.
    pub shots: u32,
    /// The shell is asking what to picture, and has the keyboard for it.
    pub picking: bool,
    /// Whether the screens are turned off for want of anyone looking.
    pub screens_off: bool,
    /// Whether the lock was already asked for since then.
    pub idle_lock_asked: bool,
    /// Whether the shell says an application is showing something to watch.
    pub idle_held: bool,
    /// Windows that asked, themselves, that the screen be left alone.
    pub idle_inhibitors: Vec<WlSurface>,
    /// Where in the shell's window its toasts are, to be shown above everything.
    pub toast: Option<Rectangle<i32, Logical>>,
    /// The shell-local dock and its pointer corridor, independent of notifications.
    pub dock: Option<Rectangle<i32, Logical>>,
    /// Temporarily routes keys to the dock while preserving application selection.
    pub dock_keyboard: bool,
    pub dock_keyboard_screen: usize,
    pub docks: std::collections::HashMap<usize,(Rectangle<i32, Logical>,smithay::backend::renderer::element::Id)>,
    pub dock_id: smithay::backend::renderer::element::Id,
    /// The name the toasts go by when drawn.
    pub toast_id: smithay::backend::renderer::element::Id,
    /// A part of the shell's window on the main display shown above everything, such
    /// as its drop-down terminal; the keyboard is the shell's while it shows.
    pub overlay: Option<Rectangle<i32, Logical>>,
    pub overlay_id: smithay::backend::renderer::element::Id,
    /// Whether the pinned application has the keyboard, having been clicked.
    pub pin_focused: bool,
    /// The shell's backdrop windows: the nth covers display n + 1 behind its applications.
    pub backdrops: Vec<Window>,
    /// Application windows, in the order they were opened.
    pub apps: Vec<App>,
    /// The displays, in logical coordinates; the first is the one with the frame.
    pub screens: Vec<Rectangle<i32, Logical>>,
    /// How each display's workspace is being used; parallel to `screens`.
    pub modes: Vec<ScreenMode>,
    /// The workspace in view, and how the displays of each other one were being used
    /// when it was last in view.
    pub workspace: usize,
    pub kept_modes: Vec<Vec<ScreenMode>>,
    /// Counts uses of applications, so that the one used most recently can be found.
    pub use_clock: u64,
    /// A divider between columns being dragged with the pointer.
    pub resize: Option<crate::layout::Drag>,
    /// Whether the pointer is over a divider it could drag, or dragging one.
    pub resize_hover: bool,
    /// Which of the frame's panels are open.
    pub panels: Panels,
    /// The display in use: its application has the keyboard, and shortcuts act on it.
    pub focus_screen: usize,
    /// What the client under the pointer wants the pointer to look like.
    pub cursor_status: CursorImageStatus,
    /// Testing aid: treat the left and right halves of the window as two displays.
    pub split: bool,

    pub ipc: Option<IpcServer>,
    /// Set when the compositor must exit with an error.
    pub fatal: Option<String>,
    /// A display layout chosen in the settings and not yet saved, which overrides
    /// the saved ones.
    /// The keyboard, mouse and touchpad settings in force.
    pub input_settings: des_common::input::InputSettings,
    pub shortcut_bindings: des_common::shortcuts::Bindings,
    pub side_panel_behavior: des_common::PanelBehavior,
    /// The keyboard layout and variant last given to the keymap compiler.
    pub keymap_applied: Option<(String, String)>,
    /// While locked, only the shell is on screen, showing the lock screen, and the
    /// keyboard is its alone.
    pub locked: bool,
    pub display_override: Option<Vec<crate::monitors::Saved>>,
    /// Set while a new layout waits to be confirmed: what to go back to, and since when.
    pub display_pending: Option<(Option<Vec<crate::monitors::Saved>>, Instant)>,
    pub screenshot: Option<PathBuf>,
    /// Where windows are on their way to, for drawing them on the way.
    pub motions: crate::motion::Motions<Window>,
    /// Glow, scanlines and the night light, applied to each frame on its way out.
    pub phosphor: crate::phosphor::Phosphor,
    /// Bars, wallpapers, launchers and notification daemons of other desktops' tools.
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    /// The layer surface that has the keyboard, while it may keep it.
    pub layer_focus: Option<smithay::desktop::LayerSurface>,
    /// What the layers reserve on each output, as last applied to the windows.
    pub layer_reservations: Vec<(Rectangle<i32, Logical>, crate::handlers::layer_shell::Insets)>,
    /// Pointer buttons held down now.
    pub buttons_down: u32,
    /// The files being dragged out of the shell, while a drag lasts.
    pub dragged_files: Option<Vec<String>>,
    pub drag_icon: Option<WlSurface>,
    pub drag_session: Option<crate::dnd::Session>,
    pub drag_overview_enabled: bool,
    /// The overview or the window switcher, while open or closing.
    pub overview: Option<crate::overview::Overview>,
    /// Application windows, most recently given the keyboard first.
    pub recency: crate::switcher::Recency<Window>,
    /// Super pressed on its own, which opens the overview when let go.
    pub super_tap: crate::overview::SuperTap,
    /// Set while a key is read: the overview is to toggle, or the switcher to choose,
    /// once the key has gone to whoever it was for.
    pub overview_wanted: bool,
    pub switcher_released: bool,
    /// A touchpad swipe the desktop took, while the fingers are down.
    pub swipe: Option<crate::gestures::Swipe>,
}

/// A length of time in milliseconds that a test may set through the environment.
pub fn test_millis(name: &str, usual: u64) -> u64 {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(usual)
}

impl DesComp {
    pub fn new(event_loop: &mut EventLoop<'static, DesComp>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let data_control_state =
            smithay::wayland::selection::wlr_data_control::DataControlState::new::<Self, _>(&dh, None, |_| true);
        let primary_selection_state = smithay::wayland::selection::primary_selection::PrimarySelectionState::new::<Self>(&dh);
        let xdg_activation_state = smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&dh);
        let xdg_foreign_state = smithay::wayland::xdg_foreign::XdgForeignState::new::<Self>(&dh);
        // Globals that need nothing kept: what applications draw at, and how they hear of it.
        smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        smithay::wayland::viewporter::ViewporterState::new::<Self>(&dh);
        smithay::wayland::single_pixel_buffer::SinglePixelBufferState::new::<Self>(&dh);
        smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        // Swipes and pinches the desktop does not take reach applications this way.
        smithay::wayland::pointer_gestures::PointerGesturesState::new::<Self>(&dh);
        smithay::wayland::presentation::PresentationState::new::<Self>(&dh, {
            use smithay::utils::ClockSource;
            smithay::utils::Monotonic::ID as u32
        });
        let popups = PopupManager::default();

        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh.clone(),

            space,
            loop_signal,
            loop_handle: event_loop.handle(),
            socket_name,
            backend: None,
            udev: None,

            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            xwayland_shell_state: XWaylandShellState::new::<Self>(&dh),
            xwm: None,
            x_display: None,
            x_popups: Vec::new(),
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            data_control_state,
            primary_selection_state,
            xdg_activation_state,
            xdg_foreign_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            popups,
            seat,

            shell_spec: None,
            shell_child: None,
            terminal_runtime: None,
            shell_crashes: Vec::new(),
            shell_restarting: false,
            shell_pid: None,
            input_protocols: crate::handlers::input_protocols::init(&dh),
            shell: None,
            last_input: Instant::now(),
            unsaid: Vec::new(),
            last_frame: Instant::now(),
            recording: None,
            shots: 0,
            picking: false,
            screens_off: false,
            idle_lock_asked: false,
            idle_held: false,
            idle_inhibitors: Vec::new(),
            toast: None,
            dock: None,
            dock_keyboard: false,
            dock_keyboard_screen: 0,
            docks: Default::default(),
            dock_id: smithay::backend::renderer::element::Id::new(),
            // A session starts quickly enough to need none; a test may ask for one.
            splash: std::env::var_os("DES_TEST_SPLASH_MS").map(|_| crate::splash::Splash::new(Duration::from_millis(test_millis("DES_TEST_SPLASH_MS", 1400)))),
            toast_id: smithay::backend::renderer::element::Id::new(),
            overlay: None,
            overlay_id: smithay::backend::renderer::element::Id::new(),
            pin_focused: false,
            backdrops: Vec::new(),
            apps: Vec::new(),
            screens: Vec::new(),
            modes: Vec::new(),
            workspace: 0,
            kept_modes: Vec::new(),
            use_clock: 0,
            resize: None,
            resize_hover: false,
            panels: Panels::default(),
            focus_screen: 0,
            cursor_status: CursorImageStatus::default_named(),
            split: false,

            ipc: None,
            fatal: None,
            input_settings: des_common::input::InputSettings::default(),
            shortcut_bindings: des_common::shortcuts::Bindings::load(),
            side_panel_behavior: des_common::PanelBehavior::load(),
            keymap_applied: None,
            locked: false,
            display_override: None,
            display_pending: None,
            screenshot: None,
            motions: crate::motion::Motions::new(crate::motion::wanted()),
            phosphor: crate::phosphor::Phosphor::new(),
            layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState::new::<Self>(&dh),
            layer_focus: None,
            layer_reservations: Vec::new(),
            buttons_down: 0,
            dragged_files: None,
            drag_icon: None,
            drag_session: None,
            drag_overview_enabled: crate::dnd::overview_wanted(),
            overview: None,
            recency: Default::default(),
            super_tap: Default::default(),
            overview_wanted: false,
            switcher_released: false,
            swipe: None,
        }
    }

    fn init_wayland_listener(
        display: Display<DesComp>,
        event_loop: &mut EventLoop<'static, DesComp>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Global visibility filters run under the Wayland backend lock.
                // Capture credentials now so they never need to re-enter it.
                let peer_pid = smithay::reexports::rustix::net::sockopt::socket_peercred(&client_stream)
                    .ok()
                    .map(|credentials| credentials.pid.as_raw_pid() as u32);
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState {
                        peer_pid,
                        ..Default::default()
                    }))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // Left alone for long enough, the screen locks.
        loop_handle
            .insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_secs(5)),
                |_, _, state| {
                    state.check_idle();
                    smithay::reexports::calloop::timer::TimeoutAction::ToDuration(Duration::from_secs(5))
                },
            )
            .expect("Failed to start the idle timer.");

        // The display itself is an event source, so that client requests get dispatched.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        // Notifications, the dock, and the reveal edge belong to the shell.
        let screen=self.screen_at(pos);
        if let (Some(shell),Some(display))=(self.shell_on(screen),self.screens.get(screen)) {
            if self.shell_overlay_contains(pos) {
                let origin=display.loc.to_f64();
                return shell.surface_under(pos-origin,WindowSurfaceType::ALL).map(|(surface,at)|(surface,at.to_f64()+origin));
            }
        }
        // Bars and launchers above the windows; beneath them, wallpapers and the like,
        // where no application covers them.
        if let Some((_, surface, at)) = self.layer_hit(pos) {
            return Some((surface, at));
        }
        self.space.element_under(pos).and_then(|(window, location)| {
            window
                .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + location).to_f64()))
        })
    }

    /// The renderer of whichever backend is running.
    pub fn renderer(&mut self) -> Option<&mut GlesRenderer> {
        match (&mut self.backend, &mut self.udev) {
            (Some(winit), _) => Some(winit.renderer()),
            (None, Some(udev)) => Some(&mut udev.renderer),
            (None, None) => None,
        }
    }

    /// Work due before drawing a frame. Returns false once the session is over.
    pub fn before_frame(&mut self) -> bool {
        self.motions.tick(Instant::now());
        self.tick_drag_overview();
        self.refresh_overview();
        self.prefer_scales();
        if self.splash.as_ref().is_some_and(|splash| splash.done()) {
            self.splash = None;
        }
        if let Some(child) = self.shell_child.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                self.shell_child = None;
                let restarting = std::mem::take(&mut self.shell_restarting);
                // The session ends when the user quits the shell. A shell that crashed
                // is started again instead: the applications are still running.
                if status.success() && !restarting {
                    tracing::info!("shell exited, ending session");
                    self.loop_signal.stop();
                    return false;
                }
                if !restarting {
                    tracing::error!(%status, "shell crashed, starting it again");
                    self.shell_crashes.retain(|at| at.elapsed() < CRASH_WINDOW);
                    self.shell_crashes.push(Instant::now());
                }
                let result = if self.shell_crashes.len() > MAX_CRASHES {
                    Err("the shell keeps crashing".to_string())
                } else {
                    self.spawn_shell().map_err(|e| e.to_string())
                };
                if let Err(reason) = result {
                    self.fatal = Some(reason);
                    self.loop_signal.stop();
                    return false;
                }
            }
        }
        // A display change nobody confirmed is taken back: the screen may be unreadable.
        if self.display_pending.as_ref().is_some_and(|(_, since)| since.elapsed() > crate::displays::CONFIRM_WITHIN) {
            self.revert_displays();
        }
        self.pump_ipc();
        true
    }

    /// End this desktop's terminal work before fast process exit skips destructors.
    /// Shell replacement deliberately leaves this owner intact.
    pub fn shutdown_terminals(&mut self) {
        drop(self.terminal_runtime.take());
    }

    /// Starts the shell, pointed at this compositor.
    pub fn spawn_shell(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.dock = None;
        self.docks.clear();
        self.dock_keyboard = false;
        self.overlay = None;
        let Some(spec) = &self.shell_spec else {
            return Err("no shell configured".into());
        };
        let mut command = std::process::Command::new(&spec.program);
        // Login screens never own a real user terminal backend.
        if !spec.args.iter().any(|arg| arg == "--greeter") {
            if self.terminal_runtime.is_none() {
                match des_common::terminal::Runtime::new() {
                    Ok(runtime) => self.terminal_runtime = Some(runtime),
                    Err(error) => tracing::warn!("terminal preservation unavailable: {error}"),
                }
            }
            if let Some(runtime) = &self.terminal_runtime {
                command.env(des_common::terminal::ENV, runtime.path());
            } else {
                command.env_remove(des_common::terminal::ENV);
            }
        } else {
            command.env_remove(des_common::terminal::ENV);
        }
        if let Some(server) = &self.ipc {
            command.env(des_common::ipc::SOCKET_ENV, server.path());
        }
        // Only the shell is pointed at our socket; programs started from it inherit it.
        // Never a host desktop's X server: programs must open here.
        match self.x_display {
            Some(number) => command.env("DISPLAY", format!(":{number}")),
            None => command.env_remove("DISPLAY"),
        };
        let child = command
            .args(&spec.args)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            // Programs should pick their Wayland backends where that is not yet their
            // default; those that only speak X11 get our own X server.
            .env("XDG_SESSION_TYPE", "wayland")
            .env("MOZ_ENABLE_WAYLAND", "1")
            .env("QT_QPA_PLATFORM", "wayland;xcb")
            .env("SDL_VIDEODRIVER", "wayland,x11")
            .env("ELECTRON_OZONE_PLATFORM_HINT", "auto")
            .spawn()
            .map_err(|e| format!("cannot start shell {}: {e}", spec.program.display()))?;
        tracing::info!(pid = child.id(), shell = %spec.program.display(), "started shell");
        self.shell_pid = Some(child.id());
        crate::handlers::input_protocols::SHELL_PID.store(child.id(), std::sync::atomic::Ordering::Relaxed);
        self.shell_child = Some(child);
        Ok(())
    }

    /// Replaces the running shell with a fresh one, which picks up a newly installed
    /// build. Managed terminals reconnect through the compositor-owned backend.
    pub fn restart_shell(&mut self) {
        self.dock = None;
        self.docks.clear();
        self.dock_keyboard = false;
        self.overlay = None;
        if let Some(child) = self.shell_child.as_mut() {
            tracing::info!("restarting the shell");
            self.shell_restarting = true;
            // Briefly, in place of the blank the shell leaves behind it.
            self.splash = Some(crate::splash::Splash::new(Duration::from_millis(500)));
            let _ = child.kill();
        }
    }

    /// Work due after drawing a frame: let clients draw their next one, and tidy up.
    pub fn after_frame(&mut self) {
        let Some(output) = self.space.outputs().next().cloned() else {
            return;
        };
        let elapsed = self.start_time.elapsed();
        self.space.elements().for_each(|window| {
            window.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()))
        });
        self.layer_frames(elapsed);
        // The shell draws all of its windows in one pass and waits for each to be shown.
        // A backdrop whose display has just gone is on no screen, and without this the
        // shell would wait on it forever, freezing the whole frame.
        for backdrop in &self.backdrops {
            backdrop.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        // Applications out of view are kept ticking slowly: enough that none waits
        // forever for a chance to draw, without having them all render at full rate.
        for app in self.apps.iter().filter(|app| app.place == Place::Hidden) {
            let preview=self.overview.as_ref().is_some_and(|overview|overview.layout.thumbs.iter().any(|(window,_,_)|window==&app.window));
            app.window.send_frame(&output,elapsed,Some(if preview {Duration::ZERO}else{Duration::from_secs(1)}),|_,_|preview.then(||output.clone()));
        }
        if let Some(icon)=self.drag_icon.as_ref() {
            smithay::desktop::utils::send_frames_surface_tree(icon,&output,elapsed,Some(Duration::ZERO),|_,_|Some(output.clone()));
        }
        self.space.refresh();
        self.popups.cleanup();
    }

}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    pub peer_pid: Option<u32>,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
