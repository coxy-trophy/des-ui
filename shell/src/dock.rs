//! Native right-panel application and window management.
use crate::{app_icons::IconCache, apps::{self, AppEntry}, theme::Theme, ui::{self}};
use des_common::ipc::{Command, DesktopState};
use eframe::egui::{self, Align2, Key, Modifiers, Rect, Sense, vec2};
#[cfg(test)]
use eframe::egui::pos2;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

/// Keyboard ownership is transient; the ordinary panel does not capture keys.
#[derive(Default)]
struct Motion {keyboard:bool,closing_keyboard:bool}
impl Motion {
    fn update(&mut self) {self.closing_keyboard=false;}
    fn dismiss(&mut self) {self.closing_keyboard |= self.keyboard;self.keyboard=false;}
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
pub struct Output {pub action:Option<Action>,pub overlay:Option<Rect>,pub keyboard:bool}

pub struct Dock {
    entries: Vec<AppEntry>,
    scan: Option<Receiver<Vec<AppEntry>>>,
    scanned: Option<Instant>,
    icons: IconCache,
    motion: Motion,
    menu_open: bool,
    selection: usize,
    revealed_selection: Option<(usize,Option<usize>,u32)>,
    coverage: Option<Rect>,
    search: String,
    expanded: HashSet<String>,
    selected_window: Option<usize>,
    display: usize,
}
impl Default for Dock {
    fn default() -> Self {
        Self { entries:Vec::new(), scan:None, scanned:None, icons:IconCache::new(), motion:Motion::default(), menu_open:false, selection:0, revealed_selection:None, coverage:None, search:String::new(), expanded:HashSet::new(), selected_window:None, display:0 }
    }
}
impl Dock {
    pub fn reveal_keyboard(&mut self) { self.revealed_selection=None;self.selected_window=None; self.motion.keyboard=true; self.motion.closing_keyboard=false; }
    pub fn dismiss_keyboard(&mut self,ctx:&egui::Context) {
        self.motion.dismiss();self.menu_open=false;
        ctx.memory_mut(|m|{m.close_popup();m.surrender_focus(egui::Id::new(("app-panel",self.display)).with("search"));});
    }
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
    /// Render the desktop application manager in its caller's right panel.
    /// The primary display includes every workspace and display.
    pub fn panel_on(&mut self, display:usize, ui:&mut egui::Ui, state:Option<&DesktopState>, pins:&[String], theme:&Theme, animate:bool)->Output {
        let ctx=ui.ctx().clone();self.display=display; self.refresh(&ctx); self.motion.update();
        let empty=DesktopState::default(); let state=state.unwrap_or(&empty);
        let mut all=items_on(&self.entries,pins,state,if display==0 {None}else{Some(display)});
        all.sort_by_key(|item|item.windows.is_empty());
        let query=self.search.trim().to_lowercase();
        all.retain(|item|query.is_empty() || item.label.to_lowercase().contains(&query) || item.windows.iter().any(|&index|state.apps[index].title.to_lowercase().contains(&query)));
        let body=ui.available_rect_before_wrap(); let id=egui::Id::new(("app-panel",display));
        let mut output=Output::default(); let mut popup_area:Option<Rect>=None;
        let mut menu_open=false; let mut key_action=None;
        self.selection=self.selection.min(all.len().saturating_sub(1));
        let popup_owns_keys=ctx.memory(|memory|memory.any_popup_open());
        if popup_owns_keys && ctx.input_mut(|input|input.consume_key(Modifiers::NONE,Key::Escape)) {
            ctx.memory_mut(|memory|memory.close_popup());
        }
        if self.motion.keyboard && !popup_owns_keys {
            let (up,down,right,left,enter,escape)=ctx.input_mut(|i|(
                i.consume_key(Modifiers::NONE,Key::ArrowUp),i.consume_key(Modifiers::NONE,Key::ArrowDown),
                i.consume_key(Modifiers::NONE,Key::ArrowRight),i.consume_key(Modifiers::NONE,Key::ArrowLeft),
                i.consume_key(Modifiers::NONE,Key::Enter),i.consume_key(Modifiers::NONE,Key::Escape)));
            if let Some(item)=all.get(self.selection) {
                if right && !item.windows.is_empty(){self.expanded.insert(item_key(item));self.selected_window=Some(0);}
                if left {self.selected_window=None;self.expanded.remove(&item_key(item));}
                if let Some(window)=self.selected_window.as_mut(){
                    *window=(*window).min(item.windows.len().saturating_sub(1));
                    if up {*window=window.saturating_sub(1);}
                    if down {*window=(*window+1).min(item.windows.len().saturating_sub(1));}
                } else {
                    if up {self.selection=self.selection.saturating_sub(1);}
                    if down {self.selection=(self.selection+1).min(all.len().saturating_sub(1));}
                }
            }
            if escape {self.dismiss_keyboard(&ctx);ctx.memory_mut(|m|m.surrender_focus(id.with("search")));}
            if enter {key_action=all.get(self.selection).map(|item| {
                self.selected_window.and_then(|n|item.windows.get(n)).map_or_else(||activation(display,item,&self.entries,state),|&index|Action::Command(Command::Activate(index)))
            });}
        }
        ui.horizontal(|ui| {
            ui::text(ui,"Open apps",ui::display_bold(15.0),ui::TEXT);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                if ui.small_button("Apps").on_hover_text("Applications launcher").clicked(){output.action=Some(Action::Launcher);}
                if ui.small_button(">_").on_hover_text("Terminal").clicked(){output.action=Some(Action::Command(Command::Terminal));}
            });
        });
        let search=ui.add(egui::TextEdit::singleline(&mut self.search).id(id.with("search")).hint_text("Find app or window").desired_width(ui.available_width()));
        if search.changed(){self.selection=0;self.selected_window=None;}
        if search.has_focus(){self.motion.keyboard=true;}
        ui.add_space(5.0);
        let mut closed_header=false;
        let reveal=self.motion.keyboard && self.revealed_selection!=Some((self.selection,self.selected_window,body.width().to_bits()));
        egui::ScrollArea::vertical().id_salt(id.with("rows")).max_height((ui.available_height()-30.0).max(40.0)).auto_shrink([false,false]).animated(animate).show(ui,|ui| {
            if all.is_empty(){ui.label(if query.is_empty(){"No open apps or pinned applications"}else{"No matching apps or windows"});}
            for (n,item) in all.iter().enumerate() {
                if item.windows.is_empty() && !closed_header {ui.add_space(10.0);ui::text(ui,"Pinned",ui::display(11.0),ui::SECONDARY);closed_header=true;}
                let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width(),52.0),Sense::click());
                // Stable response IDs allow assistive clients and tests to identify app rows.
                let response=ui.interact(rect,id.with(("app",n)),Sense::click()).union(response);
                response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),&item.label));
                let active=state.focus.is_some_and(|focus|item.windows.contains(&focus));
                let selected=self.motion.keyboard && self.selection==n;
                if response.hovered() || selected {ui.painter().rect_filled(rect,4.0,theme.tint());}
                if active {ui.painter().rect_filled(Rect::from_min_size(rect.left_top(),vec2(2.5,rect.height())),1.0,theme.main);}
                let icon=Rect::from_min_size(rect.min+vec2(10.0,10.0),vec2(32.0,32.0));
                self.icons.paint(ui,icon,item.entry.and_then(|i|self.entries[i].icon.as_deref()));
                let label_rect=Rect::from_min_max(rect.min+vec2(51.0,7.0),rect.max-vec2(22.0,20.0));
                let mut child=ui.new_child(egui::UiBuilder::new().max_rect(label_rect));
                child.add(egui::Label::new(egui::RichText::new(&item.label).size(13.0).color(ui::TEXT)).truncate());
                let detail=group_detail(item,state,display);
                let galley=ui.painter().layout_no_wrap(detail,ui::display(10.0),ui::SECONDARY);
                ui.painter().with_clip_rect(rect.shrink(4.0)).galley(rect.min+vec2(51.0,30.0),galley,ui::SECONDARY);
                if item.windows.len()>1 {ui.painter().text(rect.right_center()-vec2(11.0,0.0),Align2::CENTER_CENTER,if self.expanded.contains(&item_key(item)){"⌄"}else{"›"},ui::display(15.0),ui::SECONDARY);}
                if item.windows.iter().any(|&w|state.apps[w].urgent){ui.painter().circle_filled(rect.right_top()+vec2(-9.0,9.0),3.0,theme.main.gamma_multiply(attention_alpha(ctx.input(|i|i.time),animate)));if animate{ctx.request_repaint_after(Duration::from_millis(50));}}
                if reveal && selected {ui.scroll_to_rect(rect,Some(egui::Align::Center));}
                if response.clicked() {
                    self.selection=n;
                    if item.windows.len()>1 {let key=item_key(item);if !self.expanded.remove(&key){self.expanded.insert(key);}}
                    else {output.action=Some(activation(display,item,&self.entries,state));}
                }
                if let Some(menu)=response.context_menu(|ui| {
                    ui.label(&item.label);
                    if let Some(entry)=item.entry {if ui.button("Open new window").clicked(){output.action=Some(Action::Launch(self.entries[entry].clone()));ui.close_menu();}}
                    if let Some(pin)=&item.id {
                        let pinned=pins.contains(pin);
                        if ui.button(if pinned{"Unpin application"}else{"Pin application"}).clicked(){output.action=Some(Action::Pin(pin.clone(),!pinned));ui.close_menu();}
                        if let Some(index)=pins.iter().position(|p|p==pin){
                            for (label,enabled,before) in [("Move up",index>0,true),("Move down",index+1<pins.len(),false)] {
                                if ui.add_enabled(enabled,egui::Button::new(label)).clicked(){output.action=Some(Action::MovePin(pin.clone(),before));ui.close_menu();}
                            }
                        }
                    }
                    if !item.windows.is_empty() && ui.button("Show window controls").clicked(){self.expanded.insert(item_key(item));ui.close_menu();}
                    if !item.windows.is_empty() && ui.button("Preview windows").clicked(){output.action=Some(activation(display,item,&self.entries,state));ui.close_menu();}
                }){menu_open=true;popup_area=Some(popup_area.map_or(menu.response.rect,|r|r.union(menu.response.rect)));}
                if self.expanded.contains(&item_key(item)) {
                    for (window_n,&index) in item.windows.iter().enumerate() {
                        let app=&state.apps[index];
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            let available=(ui.available_width()-30.0).max(10.0);
                            let response=ui.add_sized(vec2(available,32.0),egui::Button::new(egui::RichText::new(format!("{} · W{}{}",app.title,app.workspace+1,if app.screen!=display{format!(" · D{}",app.screen+1)}else{String::new()})).size(11.0)).truncate().frame(false));
                            if self.motion.keyboard && self.selection==n && self.selected_window==Some(window_n) {
                                ui.painter().rect_stroke(response.rect,3.0,egui::Stroke::new(1.0,theme.main),egui::StrokeKind::Inside);
                                if reveal {ui.scroll_to_rect(response.rect,Some(egui::Align::Center));}
                            }
                            if response.clicked(){output.action=Some(Action::Command(Command::Activate(index)));}
                            if ui.small_button("×").on_hover_text("Close window").clicked(){output.action=Some(Action::Command(Command::Close(index)));}
                            if let Some(menu)=response.context_menu(|ui| {
                                if ui.button("Activate").clicked(){output.action=Some(Action::Command(Command::Activate(index)));ui.close_menu();}
                                for workspace in 0..des_common::ipc::WORKSPACES {if ui.button(format!("Move to workspace {}",workspace+1)).clicked(){output.action=Some(Action::Command(Command::SendTo(index,workspace)));ui.close_menu();}}
                                if state.screens>1 && ui.button("Move to next display").clicked(){output.action=Some(Action::Command(Command::Move(index)));ui.close_menu();}
                                if ui.button(if app.pinned{"Leave picture in picture"}else{"Picture in picture"}).clicked(){output.action=Some(Action::Command(if app.pinned{Command::Unpin}else{Command::Pin(index)}));ui.close_menu();}
                                if ui.button("Close window").clicked(){output.action=Some(Action::Command(Command::Close(index)));ui.close_menu();}
                            }) {menu_open=true;popup_area=Some(popup_area.map_or(menu.response.rect,|r|r.union(menu.response.rect)));}
                        });
                    }
                }
            }
        });
        if ui.small_button("All windows").clicked(){output.action=Some(Action::Command(Command::Overview));}
        if key_action.is_some(){output.action=key_action;}
        self.revealed_selection=Some((self.selection,self.selected_window,body.width().to_bits()));
        self.menu_open=menu_open;
        if let Some(Action::Command(Command::AppWindows(_,ids)))=&output.action {
            let capable=ctx.data(|data|data.get_temp::<bool>(egui::Id::new("window-shelf-capable"))).unwrap_or(false);
            let spans_displays=state.apps.iter().any(|app|ids.contains(&app.app_id) && app.screen!=display);
            if !capable || spans_displays {
                for item in &all {if item.windows.iter().any(|&w|ids.contains(&state.apps[w].app_id)){self.expanded.insert(item_key(item));}}
                output.action=None;
            }
        }
        if output.action.is_some(){self.motion.dismiss();search.surrender_focus();}
        output.keyboard=self.motion.keyboard || self.menu_open;
        self.coverage=Some(body);
        output.overlay=(output.keyboard || self.motion.closing_keyboard).then(||popup_area.map_or(body,|r|body.union(r)));
        output
    }
}

fn item_key(item:&Item)->String {item.id.clone().unwrap_or_else(||format!("{}:{:?}",item.label,item.windows))}
fn group_detail(item:&Item,state:&DesktopState,display:usize)->String {
    if item.windows.is_empty(){return "Ready to open".into();}
    let mut places=Vec::new();
    for &index in &item.windows {
        let app=&state.apps[index];
        let label=if app.screen==display{format!("W{}",app.workspace+1)}else{format!("W{} · D{}",app.workspace+1,app.screen+1)};
        if !places.contains(&label){places.push(label);}
    }
    format!("{} {} · {}",item.windows.len(),if item.windows.len()==1{"window"}else{"windows"},places.join(", "))
}

fn activation(display:usize,item:&Item,entries:&[AppEntry],state:&DesktopState)->Action {
    match item.windows.as_slice() {
        [] => item.entry.map_or_else(|| Action::Unavailable(item.label.clone()), |entry| Action::Launch(entries[entry].clone())),
        [window] => Action::Command(Command::Activate(*window)),
        [_,..] => {
            let mut ids=Vec::new();
            for index in &item.windows {if let Some(app)=state.apps.get(*index) {if !ids.contains(&app.app_id){ids.push(app.app_id.clone());}}}
            let command=Command::AppWindows(display,ids);
            if command.encode().len()>3500 {Action::Unavailable("This application reports too many distinct identities to preview safely".into())}
            else {Action::Command(command)}
        },
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
    fn keyboard_dismissal_releases_capture_on_next_frame_without_visibility_state() {
        let mut motion=Motion::default();motion.keyboard=true;
        motion.dismiss();assert!(!motion.keyboard);assert!(motion.closing_keyboard);
        motion.update();assert!(!motion.keyboard);assert!(!motion.closing_keyboard);
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


}


#[cfg(test)]
mod shelf_behavior_tests {
    use super::*;
    use des_common::ipc::{AppInfo,Place};
    #[test]
    fn clicking_a_group_opens_a_chooser_instead_of_advancing_the_focused_window() {
        let app=|id:&str,title:&str,workspace|AppInfo{app_id:id.into(),title:title.into(),workspace,screen:0,place:Place::Main,pinned:false,urgent:false};
        let state=DesktopState{apps:vec![app("browser","First",0),app("browser-beta","Second",3)],focus:Some(0),..Default::default()};
        let group=Item{id:None,label:"Browser".into(),entry:None,windows:vec![0,1]};
        assert!(matches!(activation(0,&group,&[],&state),Action::Command(Command::AppWindows(0,ids)) if ids==["browser","browser-beta"]));
        let single=Item{windows:vec![1],..group};
        assert!(matches!(activation(0,&single,&[],&state),Action::Command(Command::Activate(1))));
    }

}

#[cfg(test)]
mod right_panel_tests {
    use super::*;
    use des_common::ipc::{AppInfo,Place};
    fn window(id:&str,title:&str)->AppInfo {AppInfo{app_id:id.into(),title:title.into(),screen:0,workspace:0,place:Place::Main,pinned:false,urgent:false}}
    #[test]
    fn native_rows_fit_narrow_and_wide_panels_without_idle_overlay() {
        for width in [192.0, 240.0, 340.0] {
            let ctx=egui::Context::default(); ui::install_fonts(&ctx,None,None);
            let mut dock=Dock::default(); dock.scanned=Some(Instant::now());
            let pins=vec!["one.desktop".into(),"two.desktop".into()];
            let _=ctx.run(egui::RawInput {screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(width,600.0))),..Default::default()},|ctx| {
                egui::CentralPanel::default().show(ctx,|ui| {
                    let output=dock.panel_on(0,ui,None,&pins,&Theme::load("signal"),false);
                    assert!(output.overlay.is_none()); assert!(!output.keyboard);
                });
            });
            let first=ctx.read_response(egui::Id::new(("app-panel",0usize)).with(("app",0usize))).unwrap();
            let second=ctx.read_response(egui::Id::new(("app-panel",0usize)).with(("app",1usize))).unwrap();
            assert_eq!(first.rect.height(),52.0); assert!(first.rect.width()<=width);
            assert!(second.rect.top()>=first.rect.bottom());
        }
    }    #[test]
    fn primary_groups_every_screen_and_details_expose_location() {
        let state=DesktopState{apps:vec![window("browser","one"),AppInfo{screen:1,workspace:3,..window("browser","two")}],screens:2,..Default::default()};
        let all=items_on(&[],&[],&state,None);
        assert_eq!(all.len(),1); assert_eq!(all[0].windows,[0,1]);
        assert!(group_detail(&all[0],&state,0).contains("2 windows"));
        assert!(group_detail(&all[0],&state,0).contains("D2"));
    }
    #[test]
    fn keyboard_escape_is_reserved_then_releases_without_an_action() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut dock=Dock::default();dock.scanned=Some(Instant::now());dock.reveal_keyboard();
        let input=egui::RawInput{events:vec![egui::Event::Key{key:Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE}],..Default::default()};
        let _=ctx.run(input,|ctx|{
            let reserved=dock.reserve_keyboard(ctx);assert_eq!(reserved.len(),1);
            assert!(!ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{..}))));
            ctx.input_mut(|i|i.events.extend(reserved));
            egui::CentralPanel::default().show(ctx,|ui|{
                let output=dock.panel_on(0,ui,None,&[],&Theme::load("signal"),false);
                assert!(!output.keyboard);assert!(output.action.is_none());
            });
        });
    }
    #[test]
    fn stale_keyboard_selection_is_bounded_by_search_results() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut dock=Dock::default();dock.scanned=Some(Instant::now());dock.reveal_keyboard();
        dock.selection=999;dock.search="missing".into();
        let _=ctx.run(Default::default(),|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{dock.panel_on(0,ui,None,&["browser.desktop".into()],&Theme::load("signal"),false);});
        });
        assert_eq!(dock.selection,0);
    }

    #[test]
    fn keyboard_selects_an_expanded_window_without_cycling_the_app() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut dock=Dock::default();dock.scanned=Some(Instant::now());dock.reveal_keyboard();
        let state=DesktopState{apps:vec![window("browser","one"),window("browser","two")],..Default::default()};
        let events=[Key::ArrowRight,Key::ArrowDown,Key::Enter].into_iter().map(|key|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE}).collect();
        let _=ctx.run(egui::RawInput{events,..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{
                let output=dock.panel_on(0,ui,Some(&state),&[],&Theme::load("signal"),false);
                assert!(matches!(output.action,Some(Action::Command(Command::Activate(1)))));
            });
        });
    }
    #[test]
    fn long_vertical_list_scrolls_keyboard_selection_into_panel() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut dock=Dock::default();dock.scanned=Some(Instant::now());dock.reveal_keyboard();dock.selection=127;
        let pins=(0..128).map(|n|format!("app-{n}.desktop")).collect::<Vec<_>>();
        let screen=Rect::from_min_size(pos2(0.0,0.0),vec2(240.0,480.0));
        for n in 0..4 {let _=ctx.run(egui::RawInput{screen_rect:Some(screen),time:Some(n as f64*0.1),..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{dock.panel_on(0,ui,None,&pins,&Theme::load("signal"),false);});
        });}
        let row=ctx.read_response(egui::Id::new(("app-panel",0usize)).with(("app",127usize))).unwrap();
        assert!(screen.intersects(row.rect),"last app must be reachable: {:?}",row.rect);
    }

    #[test]
    fn popup_owns_navigation_and_escape_returns_to_app_list() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut dock=Dock::default();dock.scanned=Some(Instant::now());dock.reveal_keyboard();
        let pins=vec!["one.desktop".into(),"two.desktop".into()];
        let popup=egui::Id::new("test-app-popup");
        let events=[Key::ArrowDown,Key::Enter,Key::Escape].into_iter().map(|key|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE}).collect();
        let _=ctx.run(egui::RawInput{events,..Default::default()},|ctx|{
            ctx.memory_mut(|m|m.open_popup(popup));
            egui::CentralPanel::default().show(ctx,|ui|{
                let output=dock.panel_on(0,ui,None,&pins,&Theme::load("signal"),false);
                assert!(output.action.is_none());assert!(output.keyboard);
            });
            assert_eq!(dock.selection,0);assert!(!ctx.memory(|m|m.any_popup_open()));
        });
    }

}
