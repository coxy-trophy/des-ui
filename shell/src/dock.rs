//! Application switching without taking space from the terminal.
use crate::{app_icons::IconCache, apps::{self, AppEntry}, theme::Theme, ui::{self, Icon}};
use des_common::ipc::{Command, DesktopState};
use eframe::egui::{self, Align2, Key, Modifiers, Order, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const HEIGHT: f32 = 68.0;
const SLOT: f32 = 52.0;
const PAD: f32 = 8.0;
const REVEAL_DELAY: f64 = 0.08;
const HIDE_DELAY: f64 = 0.4;
const ANIMATION_SECONDS: f32 = 0.18;

#[derive(Debug)]
struct Item { id: Option<String>, label: String, entry: Option<usize>, windows: Vec<usize> }
fn normalized(id: &str) -> String { id.trim().to_lowercase().trim_end_matches(".desktop").to_string() }
pub(crate) fn entry_for(id: &str, entries: &[AppEntry]) -> Option<usize> {
    let key = normalized(id);
    if key.is_empty() { return None; }
    // Prefer the explicit window class over an executable shared by several launchers.
    entries.iter().position(|entry| entry.startup_wm_class.as_deref().is_some_and(|class| normalized(class) == key))
        .or_else(|| entries.iter().position(|entry| normalized(&entry.desktop_id) == key))
        .or_else(|| entries.iter().position(|entry| entry.exec.first().and_then(|exec| std::path::Path::new(exec).file_name()).is_some_and(|exec| normalized(&exec.to_string_lossy()) == key)))
}
/// How bright the attention dot is at `now` (seconds): a slow breath, or steady
/// without animation. Never fades out entirely, so it is always seen.
fn attention_alpha(now: f64, animate: bool) -> f32 {
    if !animate { return 1.0; }
    let phase = (now * std::f64::consts::TAU / 1.6).sin() as f32;
    0.65 + 0.35 * phase
}

#[cfg(test)]
fn items(entries: &[AppEntry],pins:&[String],state:&DesktopState)->Vec<Item>{items_on(entries,pins,state,None)}
fn items_on(entries: &[AppEntry], pins: &[String], state: &DesktopState, display:Option<usize>) -> Vec<Item> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for id in pins {
        if !seen.insert(id.clone()) { continue; }
        let entry = entries.iter().position(|entry| &entry.desktop_id == id);
        result.push(Item { id: Some(id.clone()), label: entry.map_or_else(|| id.trim_end_matches(".desktop").to_string(), |i| entries[i].name.clone()), entry, windows: Vec::new() });
        keys.push(format!("desktop:{id}"));
    }
    for (index, app) in state.apps.iter().enumerate() {
        if display.is_some_and(|screen|app.screen!=screen){continue;}
        let entry = entry_for(&app.app_id, entries);
        let missing_pin = pins.iter().find(|id| !app.app_id.is_empty() && normalized(id) == normalized(&app.app_id));
        let id = entry.map(|i| entries[i].desktop_id.clone()).or_else(|| missing_pin.cloned());
        let key = match &id {
            Some(id) => format!("desktop:{id}"),
            None if !app.app_id.is_empty() => format!("window:{}", normalized(&app.app_id)),
            None => format!("unknown:{index}"),
        };
        if let Some(i) = keys.iter().position(|existing| existing == &key) { result[i].windows.push(index); }
        else {
            keys.push(key);
            result.push(Item { id, label: entry.map_or_else(|| if app.app_id.is_empty() { app.title.clone() } else { app.app_id.clone() }, |i| entries[i].name.clone()), entry, windows: vec![index] });
        }
    }
    result
}

#[derive(Default)]
struct Motion {
    progress: f32,
    open: bool,
    keyboard: bool,
    closing_keyboard: bool,
    dismissed: bool,
    edge_since: Option<f64>,
    left_since: Option<f64>,
    last: Option<f64>,
}
impl Motion {
    fn update(&mut self, now:f64, edge:bool, inside:bool, menu:bool, still:bool) {
        let dt = self.last.map_or(0.0, |last| (now-last).max(0.0) as f32);
        self.last = Some(now);
        if self.dismissed {
            self.progress = if still { 0.0 } else { (self.progress-dt/ANIMATION_SECONDS).max(0.0) };
            if self.progress == 0.0 {
                self.closing_keyboard=false;
                if !edge && !inside { self.dismissed=false; }
            }
            return;
        }
        if !edge { self.edge_since = None; }
        let edge_ready = edge && now - *self.edge_since.get_or_insert(now) >= REVEAL_DELAY;
        if edge_ready || (inside && self.progress > 0.0) || menu || self.keyboard {
            self.open = true;
            self.left_since = None;
        } else if self.open && now - *self.left_since.get_or_insert(now) >= HIDE_DELAY {
            self.open = false;
        }
        if still { self.progress = if self.open { 1.0 } else { 0.0 }; }
        else {
            let step = dt / ANIMATION_SECONDS;
            self.progress = if self.open { (self.progress+step).min(1.0) } else { (self.progress-step).max(0.0) };
        }
    }
    fn dismiss(&mut self) { self.closing_keyboard |= self.keyboard; self.keyboard=false; self.dismissed=true; self.open=false; self.left_since=None; self.edge_since=None; }
}

pub enum Action {
    Command(Command),
    Launch(AppEntry),
    Launcher,
    Unavailable(String),
    Pin(String, bool),
    MovePin(String, bool),
}
#[derive(Default)]
pub struct Output { pub action: Option<Action>, pub overlay: Option<Rect> }

pub struct Dock {
    entries: Vec<AppEntry>,
    scan: Option<Receiver<Vec<AppEntry>>>,
    scanned: Option<Instant>,
    icons: IconCache,
    motion: Motion,
    menu_open: bool,
    selection: usize,
    coverage: Option<Rect>,
}
impl Default for Dock {
    fn default() -> Self {
        Self { entries:Vec::new(), scan:None, scanned:None, icons:IconCache::new(), motion:Motion::default(), menu_open:false, selection:0, coverage:None }
    }
}
impl Dock {
    pub fn reveal_keyboard(&mut self) { self.motion.keyboard=true; self.motion.dismissed=false; self.motion.closing_keyboard=false; self.selection=0; }
    pub fn captures_keyboard(&self) -> bool { self.motion.keyboard || self.motion.closing_keyboard || self.menu_open }
    /// Underlying shell screens render first, but must not consume dock/menu keys.
    pub fn reserve_keyboard(&self, ctx:&egui::Context) -> Vec<egui::Event> {
        if !self.captures_keyboard() { return Vec::new(); }
        ctx.input_mut(|input| {
            let mut keys=Vec::new();
            input.events.retain(|event| {
                if matches!(event,egui::Event::Key { .. }|egui::Event::Text(_)|egui::Event::Paste(_)|egui::Event::Copy|egui::Event::Cut) {
                    keys.push(event.clone()); false
                } else { true }
            });
            keys
        })
    }
    pub fn reset(&mut self) { self.motion=Motion::default(); self.menu_open=false; self.coverage=None; }
    fn refresh(&mut self, ctx:&egui::Context) {
        if let Some(scan) = &self.scan {
            match scan.try_recv() {
                Ok(entries) => { self.entries=entries; self.scan=None; self.scanned=Some(Instant::now()); self.icons.clear(); }
                Err(mpsc::TryRecvError::Disconnected) => { self.scan=None; self.scanned=Some(Instant::now()); }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.scan.is_none() && self.scanned.is_none_or(|at| at.elapsed() > Duration::from_secs(30)) {
            let (tx,rx)=mpsc::sync_channel(1);
            let ctx=ctx.clone();
            std::thread::spawn(move || { let _=tx.send(apps::scan()); ctx.request_repaint(); });
            self.scan=Some(rx);
        }
    }
    pub fn show(&mut self, ctx:&egui::Context, screen:Rect, state:Option<&DesktopState>, pins:&[String], theme:&Theme, animate:bool) -> Output {
        self.show_on(0,ctx,screen,state,pins,theme,animate)
    }
    pub fn show_on(&mut self,display:usize,ctx:&egui::Context,screen:Rect,state:Option<&DesktopState>,pins:&[String],theme:&Theme,animate:bool)->Output {
        self.refresh(ctx);
        let empty=DesktopState::default();
        let state=state.unwrap_or(&empty);
        let all=items_on(&self.entries,pins,state,Some(display));
        let now=ctx.input(|i| i.time);
        let pointer=ctx.pointer_hover_pos();
        let trigger=des_common::dock_trigger(screen.width(),screen.height());
        let trigger=Rect::from_min_size(screen.min+vec2(trigger.x,trigger.y),vec2(trigger.width,trigger.height));
        let edge=pointer.is_some_and(|point| trigger.contains(point));
        let inside=pointer.is_some_and(|point| self.coverage.is_some_and(|area| area.contains(point)));
        self.motion.update(now,edge,inside,self.menu_open,!animate);
        let mut output=Output::default();
        if self.motion.keyboard && !self.menu_open && ctx.input(|i| i.pointer.any_pressed()) && !inside && !edge {
            self.motion.dismiss();
        }
        if self.motion.progress <= 0.0 {
            self.coverage=None;
            if edge || self.motion.keyboard { ctx.request_repaint_after(Duration::from_millis(16)); }
            // Retain temporary compositor keyboard focus even on the zero-progress first frame.
            if self.motion.keyboard { output.overlay=Some(trigger); }
            return output;
        }
        let count=all.len()+2; // terminal and launcher remain available even with no graphical apps.
        if self.motion.keyboard {
            let (left,right,enter,escape)=ctx.input_mut(|i| (
                i.consume_key(Modifiers::NONE,Key::ArrowLeft),i.consume_key(Modifiers::NONE,Key::ArrowRight),
                i.consume_key(Modifiers::NONE,Key::Enter),i.consume_key(Modifiers::NONE,Key::Escape),
            ));
            if left { self.selection=(self.selection+count-1)%count; }
            if right { self.selection=(self.selection+1)%count; }
            if escape { self.motion.dismiss(); }
            if enter { output.action=Some(self.activate(self.selection,&all,state)); self.motion.dismiss(); }
        }
        self.selection=self.selection.min(count-1);
        let columns=(((screen.width()-24.0-PAD*2.0)/SLOT).floor() as usize).max(1).min(count);
        let rows=count.div_ceil(columns);
        let width=columns as f32*SLOT+PAD*2.0;
        let height=HEIGHT+(rows-1) as f32*SLOT;
        let bottom=screen.bottom()-if display==0{des_common::STATUS_HEIGHT}else{0.0}-10.0;
        let eased=1.0-(1.0-self.motion.progress).powi(3);
        let y=(screen.bottom()+4.0)*(1.0-eased)+(bottom-height)*eased;
        let position=pos2(screen.center().x-width/2.0,y);
        let body=Rect::from_min_size(position,vec2(width,height));
        let entries=&self.entries;
        let icons=&mut self.icons;
        let selection=self.selection;
        let keyboard=self.motion.keyboard;
        let mut hovered=None;
        let mut popup_area:Option<Rect>=None;
        let mut menu_open=false;
        egui::Area::new(egui::Id::new(("app-dock",display)))
            .order(Order::Foreground).fixed_pos(position).movable(false).constrain(false)
            .show(ctx,|ui| {
                ui.set_min_size(vec2(width,height));
                ui.set_max_size(vec2(width,height));
                ui.set_clip_rect(body.intersect(screen));
                ui.painter().rect_filled(body,12.0,ui::PANEL);
                ui.painter().rect_stroke(body,12.0,Stroke::new(1.0,ui::LINE_STRONG),StrokeKind::Inside);
                for n in 0..count {
                    let item=n.checked_sub(1).and_then(|i| all.get(i));
                    let terminal=n==0;
                    let launcher=n==count-1;
                    let label=if terminal { "Terminal" } else if launcher { "Applications" } else { &item.unwrap().label };
                    let rect=Rect::from_min_size(
                        body.min+vec2(PAD+(n%columns) as f32*SLOT,PAD+(n/columns) as f32*SLOT),
                        vec2(SLOT,SLOT),
                    );
                    let response=ui.interact(rect,ui.id().with(("dock-item",n)),Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),label));
                    let active=if terminal { state.focus.is_none() } else { item.is_some_and(|item| state.focus.is_some_and(|focus| item.windows.contains(&focus))) };
                    if response.hovered() || response.has_focus() || (keyboard && n==selection) {
                        ui.painter().rect_filled(rect.shrink2(vec2(3.0,2.0)),7.0,theme.tint());
                    }
                    if response.hovered() { hovered=Some(n); }
                    let icon_rect=Rect::from_center_size(rect.center()-vec2(0.0,3.0),vec2(34.0,34.0));
                    if terminal {
                        icons.paint(ui,icon_rect,Some("utilities-terminal"));
                    } else if launcher {
                        ui::paint_icon(ui.painter(),icon_rect,Icon::Apps,ui::TEXT,false);
                    } else {
                        icons.paint(ui,icon_rect,item.and_then(|item| item.entry).and_then(|i| entries[i].icon.as_deref()));
                    }
                    let running=terminal || item.is_some_and(|item| !item.windows.is_empty());
                    if running { ui.painter().circle_filled(rect.center_bottom()-vec2(0.0,3.0),if active { 2.5 } else { 1.7 },if active { theme.main } else { ui::SECONDARY }); }
                    // A window that asked for attention: an accent dot at the corner, breathing.
                    if item.is_some_and(|item| item.windows.iter().any(|&w| state.apps.get(w).is_some_and(|app| app.urgent))) {
                        let alpha=attention_alpha(now,animate);
                        ui.painter().circle_filled(rect.left_top()+vec2(9.0,9.0),3.5,theme.main.gamma_multiply(alpha));
                        if animate { ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)); }
                    }
                    if let Some(item)=item {
                        if item.windows.len()>1 { ui.painter().text(rect.right_top()+vec2(-5.0,2.0),Align2::RIGHT_TOP,item.windows.len().to_string(),ui::mono(10.0),ui::TEXT); }
                        if pins.iter().any(|id| item.id.as_ref()==Some(id)) {
                            ui.painter().hline((rect.left()+20.0)..=(rect.right()-20.0),rect.top()+1.0,Stroke::new(1.0,ui::LABEL));
                        }
                    }
                    if response.clicked() { hovered=Some(n); }
                    let clicked=response.clicked();
                    if let Some(item)=item {
                        if let Some(menu)=response.context_menu(|ui| {
                            ui.set_min_width(170.0);
                            ui.label(label);
                            ui.separator();
                            if let Some(entry)=item.entry {
                                if ui.button("Open new window").clicked() { output.action=Some(Action::Launch(entries[entry].clone())); ui.close_menu(); }
                            } else if item.windows.is_empty() { ui.label("Application is not installed"); }
                            if let Some(id)=&item.id {
                                let pinned=pins.contains(id);
                                if ui.button(if pinned { "Unpin from dock" } else { "Pin to dock" }).clicked() { output.action=Some(Action::Pin(id.clone(),!pinned)); ui.close_menu(); }
                                if let Some(index)=pins.iter().position(|pin| pin==id) {
                                    if ui.add_enabled(index>0,egui::Button::new("Move left")).clicked() { output.action=Some(Action::MovePin(id.clone(),true)); ui.close_menu(); }
                                    if ui.add_enabled(index+1<pins.len(),egui::Button::new("Move right")).clicked() { output.action=Some(Action::MovePin(id.clone(),false)); ui.close_menu(); }
                                }
                            }
                            egui::ScrollArea::vertical().max_height((screen.height()*0.5).max(90.0)).show(ui,|ui| {
                                for &index in &item.windows {
                                    let app=&state.apps[index];
                                    ui.separator();
                                    if ui.button(format!("{} · workspace {}",crate::panels::truncate(&app.title,48),app.workspace+1)).clicked() { output.action=Some(Action::Command(Command::Activate(index))); ui.close_menu(); }
                                    ui.horizontal(|ui| {
                                        ui::icon(ui,Icon::Close,16.0,ui::SECONDARY);
                                        if ui.button("Close window").clicked() { output.action=Some(Action::Command(Command::Close(index))); ui.close_menu(); }
                                    });
                                    if state.screens>1 {
                                        ui.horizontal(|ui| {
                                            ui::icon(ui,Icon::Send,16.0,ui::SECONDARY);
                                            if ui.button("Send to next display").clicked() { output.action=Some(Action::Command(Command::Move(index))); ui.close_menu(); }
                                        });
                                    }
                                }
                            });
                        }) { menu_open=true; popup_area=Some(popup_area.map_or(menu.response.rect,|area| area.union(menu.response.rect))); }
                    }
                    if clicked && output.action.is_none() {
                        output.action=Some(if terminal { Action::Command(Command::Terminal) } else if launcher { Action::Launcher } else {
                            let item=item.unwrap();
                            if let Some(&index)=item.windows.first() {
                                let next=state.focus.and_then(|focus| item.windows.iter().position(|&index| index==focus)).map_or(0,|index| (index+1)%item.windows.len());
                                Action::Command(Command::Activate(item.windows.get(next).copied().unwrap_or(index)))
                            } else if let Some(entry)=item.entry { Action::Launch(entries[entry].clone()) }
                            else { Action::Unavailable(item.label.clone()) }
                        });
                    }
                }
            });
        self.menu_open=menu_open;
        // Include the bottom corridor: crossing from the edge must never hand input back to the app.
        let mut area=Rect::from_min_max(body.min,pos2(body.right(),screen.bottom())).intersect(screen);
        if let Some(menu)=popup_area { area=area.union(menu).intersect(screen); }
        if let Some(n)=hovered.or_else(|| keyboard.then_some(selection)) {
            let label=if n==0 { "Terminal · Super+D opens the dock".to_string() } else if n==count-1 { "Applications".to_string() } else { crate::panels::truncate(&all[n-1].label,48) };
            let tip_width=(label.chars().count() as f32*7.0+16.0).min(screen.width()-8.0);
            let tip_pos=pos2((body.center().x-tip_width/2.0).clamp(screen.left()+4.0,screen.right()-tip_width-4.0),body.top()-26.0);
            let tip=egui::Area::new(egui::Id::new(("dock-label",display))).order(Order::Tooltip).fixed_pos(tip_pos).interactable(false).show(ctx,|ui| {
                egui::Frame::new().fill(ui::PANEL).stroke(Stroke::new(1.0,ui::LINE)).inner_margin(egui::Margin::symmetric(8,4)).corner_radius(4.0).show(ui,|ui| { ui::text(ui,&label,ui::display(12.0),ui::TEXT); });
            });
            area=area.union(tip.response.rect).intersect(screen);
        }
        self.coverage=Some(area);
        output.overlay=Some(area);
        if output.action.is_some() { self.motion.dismiss(); }
        if self.motion.progress<1.0 || !inside || self.motion.keyboard || self.motion.closing_keyboard || self.menu_open { ctx.request_repaint_after(Duration::from_millis(16)); }
        output
    }
    fn activate(&self,index:usize,all:&[Item],state:&DesktopState) -> Action {
        if index==0 { return Action::Command(Command::Terminal); }
        if index>all.len() { return Action::Launcher; }
        let item=&all[index-1];
        if let Some(&window)=item.windows.first() {
            let next=state.focus.and_then(|focus| item.windows.iter().position(|&n| n==focus)).map_or(0,|n| (n+1)%item.windows.len());
            Action::Command(Command::Activate(item.windows.get(next).copied().unwrap_or(window)))
        } else if let Some(entry)=item.entry { Action::Launch(self.entries[entry].clone()) }
        else { Action::Unavailable(item.label.clone()) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use des_common::ipc::{AppInfo, Place};
    fn entry(id:&str, class:Option<&str>) -> AppEntry {
        AppEntry { name: id.trim_end_matches(".desktop").into(), desktop_id:id.into(), icon:None, startup_wm_class:class.map(str::to_string), exec:vec![id.trim_end_matches(".desktop").into()], terminal:false }
    }
    fn window(id:&str, title:&str) -> AppInfo {
        AppInfo { app_id:id.into(), title:title.into(), screen:0, place:Place::Main, workspace:0, pinned:false,urgent:false }
    }
    #[test]
    fn the_attention_dot_breathes_but_never_vanishes(){
        assert_eq!(attention_alpha(12.3,false),1.0);
        let samples:Vec<f32>=(0..32).map(|i| attention_alpha(i as f64*0.05,true)).collect();
        assert!(samples.iter().all(|a| (0.29..=1.01).contains(a)));
        assert!(samples.iter().cloned().fold(f32::MAX,f32::min) < 0.4 && samples.iter().cloned().fold(0.0,f32::max) > 0.95);
    }
    #[test]
    fn display_docks_keep_shared_pins_but_only_their_own_window_indices(){
        let state=DesktopState{screens:2,apps:vec![
            des_common::ipc::AppInfo{app_id:"browser".into(),title:"Primary".into(),screen:0,place:des_common::ipc::Place::Main,workspace:0,pinned:false,urgent:false},
            des_common::ipc::AppInfo{app_id:"browser".into(),title:"Secondary".into(),screen:1,place:des_common::ipc::Place::Main,workspace:0,pinned:false,urgent:false},
        ],..Default::default()};
        let pins=vec!["browser.desktop".into()];
        let primary=items_on(&[],&pins,&state,Some(0));let secondary=items_on(&[],&pins,&state,Some(1));
        assert_eq!(primary.len(),1);assert_eq!(secondary.len(),1);
        assert_eq!(primary[0].windows,[0]);assert_eq!(secondary[0].windows,[1]);
        assert_eq!(primary[0].id,secondary[0].id);
    }
    #[test]
    fn dock_grows_with_apps_and_keeps_every_item_visible_when_wrapping() {
        let mut widths=Vec::new();
        for (pins_count,screen_width) in [(0usize,800.0),(4,800.0),(18,420.0)] {
            let ctx=egui::Context::default();
            ui::install_fonts(&ctx,None,None);
            let theme=Theme::load("signal");
            let screen=Rect::from_min_size(pos2(0.0,0.0),vec2(screen_width,700.0));
            let mut dock=Dock::default();
            dock.scanned=Some(Instant::now());
            dock.motion.progress=1.0;
            dock.motion.open=true;
            let pins:Vec<String>=(0..pins_count).map(|i| format!("app-{i}.desktop")).collect();
            for frame in 0..3 {
                let input=egui::RawInput { screen_rect:Some(screen),time:Some(frame as f64*0.02),..Default::default() };
                let _=ctx.run(input,|ctx| { dock.show(ctx,screen,None,&pins,&theme,true); });
            }
            let area=ctx.memory(|memory| memory.area_rect(egui::Id::new(("app-dock",0usize)))).unwrap();
            assert!(screen.contains_rect(area));
            assert!((area.center().x-screen.center().x).abs()<1.0);
            for n in 0..pins_count+2 {
                let button=ctx.read_response(egui::Id::new(("app-dock",0usize)).with(("dock-item",n))).unwrap();
                assert!(area.contains_rect(button.rect),"item {n} must fit in dock, {area:?}, {:?}",button.rect);
            }
            widths.push(area.width());
        }
        assert!(widths[1]>widths[0],"adding apps expands the dock");
    }
    #[test]
    fn two_icon_dock_does_not_scroll_when_wheel_is_used() {
        let ctx=egui::Context::default();
        ui::install_fonts(&ctx,None,None);
        let theme=Theme::load("signal");
        let screen=Rect::from_min_size(pos2(0.0,0.0),vec2(800.0,600.0));
        let mut dock=Dock::default();
        dock.scanned=Some(Instant::now());
        dock.motion.progress=1.0;
        dock.motion.open=true;
        let point=pos2(400.0,530.0);
        for frame in 0..4 {
            let mut input=egui::RawInput { screen_rect:Some(screen),time:Some(frame as f64*0.02),..Default::default() };
            input.events.push(egui::Event::PointerMoved(point));
            if frame>=2 { input.events.push(egui::Event::MouseWheel { unit:egui::MouseWheelUnit::Point,delta:vec2(-80.0,0.0),modifiers:Modifiers::NONE }); }
            let _=ctx.run(input,|ctx| { dock.show(ctx,screen,None,&[],&theme,true); });
        }
        let id=egui::Id::new(("app-dock",0usize)).with(egui::Id::new("dock-items"));
        let offset=egui::scroll_area::State::load(&ctx,id).map_or(0.0,|state| state.offset.x);
        assert_eq!(offset,0.0,"the two dock icons must fit without horizontal scrolling");
    }
    #[test]
    fn launcher_cannot_consume_dock_escape_or_text() {
        let ctx=egui::Context::default();
        ui::install_fonts(&ctx,None,None);
        let theme=Theme::load("signal");
        let screen=Rect::from_min_size(pos2(0.0,0.0),vec2(1000.0,700.0));
        let mut dock=Dock::default();
        dock.scanned=Some(Instant::now());
        dock.motion.progress=1.0;
        dock.motion.last=Some(0.0);
        dock.reveal_keyboard();
        let mut launcher=crate::launcher::Launcher::default();
        launcher.open=true;
        let mut input=egui::RawInput { screen_rect:Some(screen),time:Some(0.1),..Default::default() };
        input.events=vec![egui::Event::Text("should stay out of search".into()),egui::Event::Key { key:Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE }];
        let _=ctx.run(input,|ctx| {
            let reserved=dock.reserve_keyboard(ctx);
            assert_eq!(reserved.len(),2);
            assert!(!ctx.input(|i| i.events.iter().any(|event| matches!(event,egui::Event::Text(_)|egui::Event::Key{..}))));
            egui::CentralPanel::default().show(ctx,|ui| { assert!(launcher.show(ui,screen,&theme).is_none()); });
            assert!(launcher.open,"Escape belongs to dock, not launcher");
            ctx.input_mut(|i| i.events.extend(reserved));
            let output=dock.show(ctx,screen,None,&[],&theme,true);
            assert!(output.action.is_none());
            assert!(dock.motion.dismissed);
            assert!(dock.captures_keyboard(),"closing animation still protects underlying screens");
        });
    }
    #[test]
    fn keyboard_ui_reveals_activates_and_finishes_closing_without_leaking_keys() {
        let ctx=egui::Context::default();
        ui::install_fonts(&ctx,None,None);
        let theme=Theme::load("signal");
        let screen=Rect::from_min_size(pos2(0.0,0.0),vec2(800.0,600.0));
        let mut dock=Dock::default();
        dock.scanned=Some(Instant::now());
        dock.reveal_keyboard();
        for (time,enter) in [(0.0,false),(0.3,false),(0.31,true),(0.36,false),(0.6,false)] {
            let mut input=egui::RawInput { screen_rect:Some(screen),time:Some(time),..Default::default() };
            if enter { input.events.push(egui::Event::Key { key:Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE }); }
            let mut output=Output::default();
            let _=ctx.run(input,|ctx| { output=dock.show(ctx,screen,None,&[],&theme,true); });
            if enter { assert!(matches!(output.action,Some(Action::Command(Command::Terminal)))); }
            if time==0.36 { assert!(dock.captures_keyboard()); }
            if time==0.6 { assert!(!dock.captures_keyboard()); assert!(output.overlay.is_none()); }
            if time==0.3 {
                let area=output.overlay.unwrap();
                assert_eq!(area.bottom(),screen.bottom());
                assert!(area.top()<screen.bottom()-des_common::STATUS_HEIGHT);
            }
        }
    }
    #[test]
    fn keyboard_dismissal_cannot_reopen_until_pointer_leaves_and_keeps_capture_until_hidden() {
        let mut motion=Motion::default();
        motion.keyboard=true;
        motion.update(0.0,false,false,false,false);
        motion.update(0.3,false,false,false,false);
        assert_eq!(motion.progress,1.0);
        motion.dismiss();
        assert!(motion.closing_keyboard);
        motion.update(0.35,true,true,false,false);
        assert!(motion.progress<1.0);
        motion.update(0.6,true,true,false,false);
        assert_eq!(motion.progress,0.0);
        assert!(!motion.closing_keyboard);
        motion.update(1.0,true,false,false,false);
        assert_eq!(motion.progress,0.0);
        motion.update(1.1,false,false,false,false);
        motion.update(1.2,true,false,false,false);
        motion.update(1.4,true,false,false,false);
        assert!(motion.progress>0.0);
    }
    #[test]
    fn disabled_animation_snaps_after_dwell() {
        let mut motion=Motion::default();
        motion.update(0.0,true,false,false,true);
        assert_eq!(motion.progress,0.0);
        motion.update(0.1,true,false,false,true);
        assert_eq!(motion.progress,1.0);
        motion.dismiss();
        motion.update(0.2,false,false,false,true);
        assert_eq!(motion.progress,0.0);
    }
    #[test]
    fn matches_real_app_identity_without_guessing_from_titles() {
        let entries = vec![entry("org.example.Editor.desktop", Some("EditorClass")),entry("firefox.desktop",None)];
        assert_eq!(entry_for("org.example.Editor",&entries),Some(0));
        assert_eq!(entry_for("editorclass",&entries),Some(0));
        assert_eq!(entry_for("Firefox",&entries),Some(1));
        assert_eq!(entry_for("",&entries),None);
        assert_eq!(entry_for("firefox - private document",&entries),None);
    }
    #[test]
    fn pins_keep_order_and_running_windows_group_without_duplicates() {
        let entries=vec![entry("firefox.desktop",None),entry("editor.desktop",Some("Editor"))];
        let state=DesktopState { apps:vec![window("Firefox","one"),window("firefox","two"),window("Editor","three"),window("","unknown")],..Default::default() };
        let pins=vec!["editor.desktop".into(),"firefox.desktop".into(),"editor.desktop".into(),"removed.desktop".into()];
        let all=items(&entries,&pins,&state);
        assert_eq!(all.len(),4);
        assert_eq!(all[0].id.as_deref(),Some("editor.desktop"));
        assert_eq!(all[0].windows,vec![2]);
        assert_eq!(all[1].windows,vec![0,1]);
        assert_eq!(all[2].entry,None);
        assert_eq!(all[2].id.as_deref(),Some("removed.desktop"));
        assert_eq!(all[3].windows,vec![3]);
    }
    #[test]
    fn edge_dwell_reveals_and_exit_hides_without_interrupting_menu() {
        let mut motion=Motion::default();
        motion.update(0.0,false,false,false,false);
        motion.update(0.01,true,false,false,false);
        assert_eq!(motion.progress,0.0);
        motion.update(0.11,true,false,false,false);
        motion.update(0.31,true,false,false,false);
        assert_eq!(motion.progress,1.0);
        motion.update(1.0,false,false,true,false);
        assert_eq!(motion.progress,1.0);
        motion.update(1.1,false,false,false,false);
        motion.update(1.3,false,false,false,false);
        assert_eq!(motion.progress,1.0);
        motion.update(1.8,false,false,false,false);
        assert_eq!(motion.progress,0.0);
    }
}
