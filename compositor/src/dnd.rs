//! Files dragged out of the shell's file panel. The shell is an ordinary client whose
//! toolkit cannot start a Wayland drag, so it names the files over IPC and the
//! compositor starts the drag itself, as its own data source, while the button that
//! began it is still held.

use std::io::Write;
use std::os::fd::OwnedFd;

use smithay::input::Seat;
use smithay::input::pointer::{GrabStartData, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::{ServerDndGrabHandler, SourceMetadata, start_dnd};

use crate::DesComp;

/// What a dropped file is offered as: the list browsers and file managers take, and a
/// plain path for anything that only reads text.
pub const MIME_TYPES: [&str; 2] = ["text/uri-list", "text/plain;charset=utf-8"];
const LEFT_BUTTON: u32 = 0x110;

/// The bytes a client is sent for `mime`: URIs one per line for `text/uri-list`, as the
/// list's RFC asks (CRLF), and the decoded paths for plain text.
pub fn payload(uris: &[String], mime: &str) -> Option<Vec<u8>> {
    if mime == "text/uri-list" {
        return Some(uris.iter().map(|uri| format!("{uri}\r\n")).collect::<String>().into_bytes());
    }
    if mime.starts_with("text/plain") {
        let paths: Vec<Vec<u8>> = uris.iter().filter_map(|uri| path_bytes(uri)).collect();
        return Some(paths.join(&b'\n'));
    }
    None
}

/// `file:///a%20b` → `/a b`, byte for byte; `None` for anything else or bad escapes.
pub fn path_bytes(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    bytes.starts_with(b"/").then_some(bytes)
}

/// Whether a drag may start now: never while locked or while a screenshot is being
/// marked, only from the shell, and only while the button that began it is still down.
pub fn may_start(locked: bool, picking: bool, from_shell: bool, button_down: bool) -> bool {
    !locked && !picking && from_shell && button_down
}

impl DesComp {
    /// Starts dragging `uris` from where the pointer is, if the shell is the one being
    /// dragged from; otherwise does nothing.
    pub fn start_file_drag(&mut self, uris: Vec<String>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let shell_client = self.shell.as_ref().and_then(|shell| shell.wl_surface()).and_then(|surface| surface.client());
        let from_shell = pointer
            .current_focus()
            .and_then(|surface| surface.client())
            .zip(shell_client)
            .is_some_and(|(focus, shell)| focus == shell);
        // The press that began it holds an implicit click grab, which the drag replaces.
        let button_down = self.buttons_down > 0;
        // An earlier target may still request its URI data after releasing the
        // button. Never replace that payload with a subsequent drag's paths.
        if uris.is_empty() || self.dragged_files.is_some() || self.drag_session.is_some() || !may_start(self.locked, self.picking, from_shell, button_down) {
            return;
        }
        let location = pointer.current_location();
        let start = GrabStartData { focus: None, button: LEFT_BUTTON, location };
        let metadata = SourceMetadata {
            mime_types: MIME_TYPES.iter().map(|mime| mime.to_string()).collect(),
            dnd_action: DndAction::Copy,
        };
        self.dragged_files = Some(uris);
        self.drag_session = Some(Session::new(None,None));
        let dh = self.display_handle.clone();
        let seat = self.seat.clone();
        start_dnd(&dh, &seat, self, SERIAL_COUNTER.next_serial(), Some(start), None, metadata);
    }
}

impl DesComp {
    /// Ends a drag out of the shell without dropping anything, as when the screen locks.
    pub fn cancel_file_drag(&mut self) {
        if self.dragged_files.take().is_none() && self.drag_session.is_none() {
            return;
        }
        if let Some(pointer) = self.seat.get_pointer() {
            let time = monotonic_msec();
            // Leave whatever the drag is over first: ending the grab drops on the surface
            // under it, and a lock must drop nothing.
            let location = pointer.current_location();
            pointer.motion(self, None, &MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time });
            pointer.frame(self);
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
        self.drag_session=None;
        self.drag_icon=None;
        self.close_drag_overview();
    }
}

impl ServerDndGrabHandler for DesComp {
    fn send(&mut self, mime_type: String, fd: OwnedFd, _seat: Seat<Self>) {
        let Some(bytes) = self.dragged_files.as_deref().and_then(|uris| payload(uris, &mime_type)) else { return };
        // A slow reader must not hold up the compositor.
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(&bytes);
        });
    }

    fn dropped(&mut self, _seat: Seat<Self>) {
        if let Some(session)=self.drag_session.as_mut() {session.ended=true;}
    }

    fn cancelled(&mut self, _seat: Seat<Self>) {
        if let Some(session)=self.drag_session.as_mut() {session.ended=true;}
        self.dragged_files = None;
    }

    fn finished(&mut self, _seat: Seat<Self>) {
        self.dragged_files = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_and_plain_paths() {
        let uris = vec!["file:///home/a/notes%20one.txt".to_string(), "file:///tmp/%C3%A9t%C3%A9".to_string()];
        assert_eq!(payload(&uris, "text/uri-list").unwrap(), b"file:///home/a/notes%20one.txt\r\nfile:///tmp/%C3%A9t%C3%A9\r\n");
        assert_eq!(payload(&uris, "text/plain;charset=utf-8").unwrap(), "/home/a/notes one.txt\n/tmp/été".as_bytes());
        assert_eq!(payload(&uris, "image/png"), None);
    }

    #[test]
    fn bad_uris_give_no_path() {
        assert_eq!(path_bytes("http://x/y"), None);
        assert_eq!(path_bytes("file://%zz"), None);
        assert_eq!(path_bytes("file:///a%2"), None);
        assert_eq!(path_bytes("file://relative"), None);
        assert_eq!(path_bytes("file:///a%2Fb").unwrap(), b"/a/b");
    }

    #[test]
    fn drags_start_only_from_the_shell_with_the_button_down_and_never_locked() {
        assert!(may_start(false, false, true, true));
        assert!(!may_start(true, false, true, true));
        assert!(!may_start(false, true, true, true));
        assert!(!may_start(false, false, false, true));
        assert!(!may_start(false, false, true, false));
    }
}


/// Hover state is keyed by the compositor's window object, never a recycled app index.
#[derive(Debug)]
pub struct Dwell<K> { target:Option<(K,std::time::Instant)> }
impl<K:PartialEq+Clone> Default for Dwell<K> { fn default()->Self {Self{target:None}} }
impl<K:PartialEq+Clone> Dwell<K> {
    pub fn hover(&mut self,target:Option<K>,now:std::time::Instant)->f32 {
        if self.target.as_ref().map(|(target,_)|target)!=target.as_ref() {
            self.target=target.map(|target|(target,now));
        }
        self.target.as_ref().map_or(0.0,|(_,since)|
            (now.saturating_duration_since(*since).as_secs_f32()/0.45).clamp(0.0,1.0))
    }
}

pub struct Session {
    pub source:Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
    pub origin:Option<smithay::desktop::Window>,
    pub ended:bool,
    initialized:bool,
    edge_armed:bool,
    dwell:Dwell<smithay::desktop::Window>,
}
impl Session {
    pub fn new(source:Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,origin:Option<smithay::desktop::Window>)->Self {
        Self{source,origin,ended:false,initialized:false,edge_armed:false,dwell:Dwell::default()}
    }
}

pub fn parse_overview_wanted(text:&str)->bool {
    text.lines().filter_map(|line|line.split('#').next()?.split_once('='))
        .filter(|(key,_)|key.trim()=="drag_overview").map(|(_,value)|value.trim()!="off").last().unwrap_or(true)
}
pub fn overview_wanted()->bool {
    des_common::input::config_path().and_then(|path|std::fs::read_to_string(path).ok())
        .is_none_or(|text|parse_overview_wanted(&text))
}

impl DesComp {
    pub fn drag_view_open(&self)->bool {
        self.overview_open()==Some(des_common::ipc::OverviewKind::Drag)
    }
    pub fn close_drag_overview(&mut self) {
        if self.overview.as_ref().is_some_and(|o|o.kind==des_common::ipc::OverviewKind::Drag) {self.close_overview_now();}
    }
    /// Called outside Smithay grab callbacks: those may hold the pointer's mutex.
    pub fn tick_drag_overview(&mut self) {
        use smithay::utils::IsAlive;
        let Some(session)=self.drag_session.as_ref() else {self.drag_icon=None;self.close_drag_overview();return};
        if session.ended {self.drag_session=None;self.drag_icon=None;self.close_drag_overview();return;}
        let Some(pointer)=self.seat.get_pointer() else {self.drag_session=None;self.drag_icon=None;self.close_drag_overview();return};
        if self.locked || self.picking || self.screens.is_empty() || session.source.as_ref().is_some_and(|s|!s.is_alive()) ||
            session.origin.as_ref().is_some_and(|window|!window.alive()) {
            self.cancel_file_drag();return;
        }
        if !pointer.is_grabbed() {self.drag_session=None;self.drag_icon=None;self.close_drag_overview();return;}
        if self.drag_icon.as_ref().is_some_and(|icon| !icon.is_alive()) {self.drag_icon=None;}
        if !self.drag_overview_enabled {self.close_drag_overview();return;}
        let pos=pointer.current_location();
        let screen=self.screen_at(pos);
        let display=self.screens[screen];
        let edge=pos.y>=f64::from(display.loc.y+display.size.h-5);
        let session=self.drag_session.as_mut().unwrap();
        let first=!session.initialized;
        session.initialized=true;
        if !edge {session.edge_armed=true;}
        let reopen=edge && session.edge_armed;
        let origin=session.origin.clone();
        let candidates=self.apps.iter().any(|app|app.screen==screen && Some(&app.window)!=origin.as_ref());
        if !candidates {self.close_drag_overview();return;}
        if !self.drag_view_open() && (first || reopen) && candidates {
            self.open_drag_windows(screen);
            self.drag_session.as_mut().unwrap().edge_armed=false;
        }
        if !self.drag_view_open() {return;}
        if self.overview.as_ref().is_some_and(|o|o.screen!=screen) {
            self.drag_session.as_mut().unwrap().dwell=Dwell::default();
            self.open_drag_windows(screen);
        }
        self.refresh_overview();
        let target=self.overview.as_ref().and_then(|o|o.thumb_at(pos)
            .and_then(|index|o.layout.thumbs.get(index).map(|(window,_,rect)|(index,window.clone(),*rect))))
            .filter(|(_,window,_)|Some(window)!=origin.as_ref());
        let now=std::time::Instant::now();
        let progress=self.drag_session.as_mut().unwrap().dwell.hover(target.as_ref().map(|(_,w,_)|w.clone()),now);
        if let Some(overview)=self.overview.as_mut() {
            overview.filter=target.as_ref().map_or_else(String::new,|_|format!("{}",(progress*100.0) as u32));
            if let Some((index,_,_))=target.as_ref(){overview.selected=*index;}
        }
        if progress<1.0 {return;}
        let Some((_,window,thumb))=target else {return};
        let Some(index)=self.apps.iter().position(|app|app.window==window) else {return};
        self.close_drag_overview();
        self.activate(index);
        self.arrange();
        self.motions.settle();
        self.drag_session.as_mut().unwrap().dwell=Dwell::default();
        self.drag_session.as_mut().unwrap().edge_armed=false;
        // Move from the scaled preview into the same relative place in the actual window.
        if let Some(loc)=self.space.element_location(&window) {
            let size=window.geometry().size;
            let at=handoff_position((pos.x,pos.y),(thumb.loc.x,thumb.loc.y,thumb.size.w,thumb.size.h),
                (loc.x,loc.y,size.w,size.h));
            let at=smithay::utils::Point::from(at);
            let under=self.surface_under(at);
            pointer.motion(self,under,&MotionEvent{location:at,serial:SERIAL_COUNTER.next_serial(),time:monotonic_msec()});
            pointer.frame(self);
        }
    }
    pub fn drag_motion(&mut self,pos:smithay::utils::Point<f64,smithay::utils::Logical>,time:u32) {
        if let Some(pointer)=self.seat.get_pointer() {
            // A preview is not a data target. Keep the existing grab, with no offer beneath it.
            pointer.motion(self,None,&MotionEvent{location:pos,serial:SERIAL_COUNTER.next_serial(),time});
            pointer.frame(self);
        }
        self.tick_drag_overview();
    }
}

fn monotonic_msec()->u32 {
    smithay::utils::Clock::<smithay::utils::Monotonic>::new().now().as_millis()
}

fn handoff_position(pos:(f64,f64),thumb:(i32,i32,i32,i32),window:(i32,i32,i32,i32))->(f64,f64) {
    let relative=|v:f64,start:i32,length:i32|((v-f64::from(start))/f64::from(length.max(1))).clamp(0.02,0.98);
    (f64::from(window.0)+relative(pos.0,thumb.0,thumb.2)*f64::from(window.2.max(1)),
     f64::from(window.1)+relative(pos.1,thumb.1,thumb.3)*f64::from(window.3.max(1)))
}


pub fn icon_elements<C>(renderer:&mut smithay::backend::renderer::gles::GlesRenderer,icon:Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,position:smithay::utils::Point<f64,smithay::utils::Logical>,scale:f64)->Vec<C>
where C:From<smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<smithay::backend::renderer::gles::GlesRenderer>> {
    let Some(icon)=icon.filter(|icon|icon.is_alive()) else {return Vec::new()};
    smithay::backend::renderer::element::surface::render_elements_from_surface_tree(renderer,icon,position.to_physical(scale).to_i32_round(),(scale,scale),1.0,smithay::backend::renderer::element::Kind::Cursor)
}


#[cfg(test)]
mod destination_tests {
    use super::*;
    use std::time::{Instant,Duration};
    #[test]
    fn a_destination_requires_uninterrupted_dwell_and_leaving_resets_it() {
        let now=Instant::now();let mut dwell=Dwell::<u64>::default();
        assert_eq!(dwell.hover(Some(4),now),0.0);
        assert!(dwell.hover(Some(4),now+Duration::from_millis(440))<1.0);
        assert_eq!(dwell.hover(Some(8),now+Duration::from_millis(450)),0.0);
        assert_eq!(dwell.hover(None,now+Duration::from_millis(900)),0.0);
        assert_eq!(dwell.hover(Some(8),now+Duration::from_millis(901)),0.0);
        assert_eq!(dwell.hover(Some(8),now+Duration::from_millis(1351)),1.0);
    }
    #[test]
    fn handoff_preserves_relative_position_and_stays_inside_the_real_window() {
        assert_eq!(handoff_position((200.0,150.0),(100,100,200,100),(1000,20,800,600)),(1400.0,320.0));
        let (x,y)=handoff_position((-50.0,9999.0),(100,100,200,100),(1000,20,800,600));
        assert!(x>1000.0 && x<1800.0 && y>20.0 && y<620.0);
    }
    #[test]
    fn drag_overview_defaults_on_and_respects_the_last_saved_preference() {
        assert!(parse_overview_wanted(""));
        assert!(!parse_overview_wanted("drag_overview = off # accessibility preference"));
        assert!(parse_overview_wanted("drag_overview = off\ndrag_overview = on\n"));
    }
}
