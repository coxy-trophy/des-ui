//! Universal keyboard palette: one bounded background worker for files and one for
//! hosts, commands, processes and units; neither ever runs what was typed.
use crate::{apps::{self,AppEntry},clipboard,commands,palette::{self,Action,Item},file_search::{self,Found},theme::Theme,ui};
use des_common::ipc::DesktopState;
use eframe::egui::{self,Align2,Key,Modifiers,Rect,ScrollArea,Sense,Stroke,StrokeKind,TextEdit,Ui,UiBuilder,vec2};
use std::path::PathBuf;
use std::sync::{Arc,Mutex,atomic::{AtomicU64,Ordering},mpsc::{self,SyncSender}};
use std::time::{Duration,Instant};

const TILE_HEIGHT:f32=116.0;
const TILE_GAP:f32=12.0;
fn grid_columns(width:f32)->usize { (((width+TILE_GAP)/132.0).floor() as usize).clamp(1,8) }

struct Job { generation:u64, query:String, roots:Vec<PathBuf> }
struct Files {
    tx:SyncSender<Job>, generation:Arc<AtomicU64>, ready:Arc<Mutex<Option<(u64,Found)>>>,
}
impl Default for Files {
    fn default()->Self {
        let (tx,rx)=mpsc::sync_channel::<Job>(1);
        let generation=Arc::new(AtomicU64::new(0));
        let ready=Arc::new(Mutex::new(None));
        let (current,result)=(generation.clone(),ready.clone());
        std::thread::spawn(move || while let Ok(job)=rx.recv() {
            let found=file_search::search(&job.roots,&job.query,job.generation,&current);
            if current.load(Ordering::Relaxed)==job.generation { *result.lock().unwrap()=Some((job.generation,found)); }
        });
        Self {tx,generation,ready}
    }
}

pub struct Launcher {
    pub open:bool,
    grid_navigation:bool,
    pin_request:Option<String>,query:String,selected:usize,entries:Vec<AppEntry>,state:Option<DesktopState>,
    icons:crate::app_icons::IconCache,error:Option<String>,files:Files,found:Found,
    providers:commands::Worker,catalog:commands::Catalog,catalog_asked:bool,
    /// The clipboard history, when the shell watches it.
    clipboard:Option<clipboard::Handle>,
    /// What background actions (unit changes) reported: title, body, whether it went wrong.
    outcomes:Arc<Mutex<Vec<(String,String,bool)>>>,
    roots:Vec<PathBuf>,folders:String,edit_folders:bool,focus_query:bool,changed:Instant,submitted:bool,searching:bool,composing:bool,
}
impl Default for Launcher {
    fn default()->Self {
        let home=std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from("/"));
        let roots=vec![home.join("Documents"),home.join("Downloads"),home.join("Desktop")];
        Self {open:false,grid_navigation:false,pin_request:None,query:String::new(),selected:0,entries:Vec::new(),state:None,icons:Default::default(),error:None,files:Default::default(),found:Default::default(),providers:Default::default(),catalog:Default::default(),catalog_asked:false,clipboard:None,outcomes:Default::default(),folders:roots.iter().map(|p|p.display().to_string()).collect::<Vec<_>>().join("; "),roots,edit_folders:false,focus_query:true,changed:Instant::now(),submitted:false,searching:false,composing:false}
    }
}
impl Launcher {
    pub fn take_pin_request(&mut self)->Option<String>{self.pin_request.take()}
    pub fn set_state(&mut self,state:Option<DesktopState>){self.state=state;}
    pub fn fail(&mut self,error:String){self.error=Some(error);self.open=true;}
    pub fn set_clipboard(&mut self,handle:clipboard::Handle){self.clipboard=Some(handle);}
    pub fn clipboard(&self)->Option<&clipboard::Handle>{self.clipboard.as_ref()}
    /// Where a background action reports how it went; drained by the shell into notices.
    pub fn outcomes(&self)->Arc<Mutex<Vec<(String,String,bool)>>>{self.outcomes.clone()}
    pub fn take_outcomes(&mut self)->Vec<(String,String,bool)>{std::mem::take(&mut *self.outcomes.lock().unwrap_or_else(|e|e.into_inner()))}
    pub fn open(&mut self) {
        self.entries=apps::scan(); self.icons.clear();self.query.clear();self.error=None;self.open=true;self.edit_folders=false;self.focus_query=true;
        if let Some(text)=des_common::input::config_path().and_then(|p|std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                if let Some(("search_folders",value))=line.split_once('=').map(|(k,v)|(k.trim(),v.trim())) {
                    if let Ok(roots)=serde_json::from_str::<Vec<PathBuf>>(value) {
                        let text=roots.iter().map(|p|p.display().to_string()).collect::<Vec<_>>().join("; ");
                        if let Ok(roots)=file_search::parse_roots(&text){self.roots=roots;self.folders=text;}
                    }
                }
            }
        }
        self.invalidate();
    }
    pub fn toggle(&mut self){if self.open {self.open=false;}else{self.open();}}
    fn invalidate(&mut self){
        self.files.generation.fetch_add(1,Ordering::Relaxed);self.providers.generation().fetch_add(1,Ordering::Relaxed);
        self.grid_navigation=false;self.selected=0;self.found=Found::default();self.changed=Instant::now();self.submitted=false;self.searching=false;self.catalog_asked=false;
        // The hosts and commands already known stay in view; processes answer only the new query.
        self.catalog.processes.clear();
    }
    fn pump_catalog(&mut self,ctx:&egui::Context){
        let generation=self.providers.generation().load(Ordering::Relaxed);
        if let Some(catalog)=self.providers.take(generation){self.catalog=catalog;}
        else if !self.catalog_asked && !self.composing && (self.query.trim().is_empty() || self.changed.elapsed()>=Duration::from_millis(200)) {
            self.catalog_asked=self.providers.request(generation,&self.query);
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
    fn pump_files(&mut self,ctx:&egui::Context){
        let generation=self.files.generation.load(Ordering::Relaxed);
        if let Some((version,found))=self.files.ready.lock().unwrap().take(){if version==generation{self.found=found;self.searching=false;}}
        if !self.composing && !self.submitted && self.query.trim().chars().count()>=2 && self.changed.elapsed()>=Duration::from_millis(200){
            let job=Job {generation,query:self.query.clone(),roots:self.roots.clone()};
            if self.files.tx.try_send(job).is_ok(){self.submitted=true;self.searching=true;}
        }
        if self.searching || (!self.submitted && self.query.trim().chars().count()>=2){ctx.request_repaint_after(Duration::from_millis(40));}
    }
    /// App names stay on the tile; descriptions and paths belong in its tooltip.
    fn tile(&mut self,ui:&mut Ui,theme:&Theme,item:&Item,index:usize,tile_width:f32)->Option<Action>{
        let mut chosen=None;
        let (rect,response)=ui.allocate_exact_size(vec2(tile_width,TILE_HEIGHT),Sense::click());
        if response.gained_focus(){self.selected=index;}
        let selected=index==self.selected;
        response.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel,true,selected,format!("{}, {}: {}",item.label,item.kind,item.detail)));
        let painter=ui.painter_at(rect);
        if selected || response.hovered() || response.has_focus() {
            painter.rect_filled(rect,ui::SURFACE_RADIUS,if selected{theme.tint()}else{ui::TRACK});
        }
        if selected || response.has_focus(){painter.rect_stroke(rect,ui::SURFACE_RADIUS,Stroke::new(1.0,theme.main),StrokeKind::Inside);}
        let icon=Rect::from_center_size(egui::pos2(rect.center().x,rect.top()+38.0),vec2(48.0,48.0));
        if item.kind=="APP"{self.icons.paint(ui,icon,item.icon.as_deref());}else{
            let symbol=match item.kind{"SETTING"|"UNIT"=>ui::Icon::Settings,"WINDOW"|"HOST"=>ui::Icon::Display,"CLIP"=>ui::Icon::Copy,"CALC"|"COMMAND"=>ui::Icon::Keyboard,_=>ui::Icon::Apps};
            ui::paint_icon(&painter,icon,symbol,ui::SECONDARY,false);
        }
        let mut label=egui::text::LayoutJob::simple(item.label.clone(),ui::display(13.0),ui::TEXT,(tile_width-16.0).max(1.0));
        label.halign=egui::Align::Center;
        label.wrap.max_rows=2;
        label.wrap.break_anywhere=true;
        label.wrap.overflow_character=Some('…');
        let label=ui.fonts(|fonts|fonts.layout_job(label));
        painter.galley(egui::pos2(rect.center().x-label.rect.center().x,rect.top()+74.0-label.rect.top()),label,ui::TEXT);
        if item.kind!="APP"{painter.text(rect.right_top()+vec2(-6.0,5.0),Align2::RIGHT_TOP,item.kind,ui::mono(9.0),ui::LABEL);}
        response.clone().on_hover_text(format!("{}\n{} · {}",item.label,item.kind,item.detail)).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked(){chosen=Some(item.action.clone());if let Action::Copy(text)=&item.action{ui.ctx().copy_text(text.clone());}}
        if let Some(id)=&item.pin{response.context_menu(|ui|{if ui.button("Pin to dock").clicked(){self.pin_request=Some(id.clone());ui.close_menu();}});}
        if let Some((id,pinned))=item.clip{response.context_menu(|ui|{
            if ui.button(if pinned{"Unpin"}else{"Pin"}).clicked(){chosen=Some(Action::ClipPin(id,!pinned));ui.close_menu();}
            if ui.button("Remove").clicked(){chosen=Some(Action::ClipRemove(id));ui.close_menu();}
            if ui.button("Clear history").clicked(){chosen=Some(Action::ClipClear);ui.close_menu();}
        });}
        if let Action::Unit{name,user,..}=&item.action{response.context_menu(|ui|{
            for op in [commands::UnitOp::Start,commands::UnitOp::Stop,commands::UnitOp::Restart]{
                if ui.button(op.word()).clicked(){chosen=Some(Action::Unit{name:name.clone(),user:*user,op});ui.close_menu();}
            }
        });}
        chosen
    }
    pub fn show(&mut self,ui:&mut Ui,rect:Rect,theme:&Theme)->Option<Action>{
        let mut ime_frame=self.composing;
        for event in ui.input(|i|i.events.clone()) {if let egui::Event::Ime(ime)=event {ime_frame=true;self.composing=matches!(ime,egui::ImeEvent::Enabled|egui::ImeEvent::Preedit(_));}}
        // Horizontal arrows edit the query until grid navigation starts.
        let lateral=self.query.is_empty() || self.grid_navigation;
        let (up,down,left,right,enter,escape)=if ime_frame{(false,false,false,false,false,false)}else{ui.input_mut(|i|(
            !self.edit_folders && i.consume_key(Modifiers::NONE,Key::ArrowUp),
            !self.edit_folders && i.consume_key(Modifiers::NONE,Key::ArrowDown),
            lateral && !self.edit_folders && i.consume_key(Modifiers::NONE,Key::ArrowLeft),
            lateral && !self.edit_folders && i.consume_key(Modifiers::NONE,Key::ArrowRight),
            i.consume_key(Modifiers::NONE,Key::Enter),i.consume_key(Modifiers::NONE,Key::Escape)))};
        if escape {self.open=false;return None;}
        self.pump_files(ui.ctx());
        self.pump_catalog(ui.ctx());
        let margin=(rect.width()*0.06).clamp(12.0,72.0);
        let area=Rect::from_min_max(rect.min+vec2(margin,20.0),rect.max-vec2(margin,8.0));
        let mut chosen=None;
        ui.allocate_new_ui(UiBuilder::new().max_rect(area),|ui|{
            ui.horizontal_wrapped(|ui|{
                ui::tracked(ui,"APPLICATIONS",ui::display_bold(16.0),ui::TEXT,0.8);
                let folders=ui::icon_button(ui,theme,ui::Icon::Settings,30.0,self.edit_folders).on_hover_text("Search folders");
                folders.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,"Search folders"));
                if folders.clicked(){self.edit_folders=!self.edit_folders;if !self.edit_folders{self.focus_query=true;}}
                if ui::icon_button(ui,theme,ui::Icon::Close,30.0,false).on_hover_text("Close · Escape").clicked(){self.open=false;}
            });
            if self.edit_folders {
                ui::text(ui,"Local file search folders · absolute paths separated by semicolons",ui::mono(11.0),ui::SECONDARY);
                ui.add(TextEdit::singleline(&mut self.folders).char_limit(32768).font(ui::mono(12.0)).desired_width(f32::INFINITY));
                if ui::button(ui,theme,"SAVE FOLDERS",None,false).clicked(){match file_search::parse_roots(&self.folders){
                    Ok(roots)=>match crate::config::set("search_folders",Some(&serde_json::to_string(&roots).unwrap())){Ok(_)=>{self.roots=roots;self.edit_folders=false;self.focus_query=true;self.error=None;self.invalidate();},Err(e)=>self.error=Some(e.to_string())},
                    Err(e)=>self.error=Some(e.into()),
                }}
            }
            let before=self.query.clone();
            let query_id=ui.id().with("palette-query");
            ui.horizontal(|ui|{
                let field=ui.add_sized(vec2((ui.available_width()-38.0).max(40.0),38.0),TextEdit::singleline(&mut self.query).char_limit(4096).id(query_id).font(ui::display(18.0)).hint_text("Search applications, files and actions"));
                if self.focus_query && !self.edit_folders {field.request_focus();self.focus_query=false;}
                if field.clicked(){self.grid_navigation=false;}
                let clear=ui.add_enabled_ui(!self.query.is_empty(),|ui|ui::icon_button(ui,theme,ui::Icon::Close,30.0,false)).inner.on_hover_text("Clear search");
                clear.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,!self.query.is_empty(),"Clear search"));
                if clear.clicked(){self.query.clear();field.request_focus();}
            });
            if before!=self.query{self.invalidate();}
            if let Some(error)=&self.error{ui::text(ui,error,ui::mono(12.0),ui::CAUTION);}
            let clips=self.clipboard.as_ref().map(clipboard::Handle::entries).unwrap_or_default();
            let mut items=palette::results(&self.query,&self.entries,self.state.as_ref(),&self.catalog,&clips);
            items.extend(self.found.paths.iter().map(|path|Item{label:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),detail:path.display().to_string(),kind:"FILE",action:Action::File(path.clone()),pin:None,icon:None,clip:None}));
            // Browsing starts with applications; typed queries retain every provider.
            if self.query.trim().is_empty(){items.retain(|item|item.kind=="APP");}
            if palette::clip_query(&self.query).is_some() {
                let note=match self.clipboard.as_ref().map(clipboard::Handle::availability) {
                    None|Some(clipboard::Availability::Missing)=>Some("Clipboard history needs wl-clipboard (wl-paste)."),
                    Some(_) if clips.is_empty()=>Some("Nothing copied yet. History is kept in memory only and forgotten at logout."),
                    Some(_)=>None,
                };
                if let Some(note)=note{ui::text(ui,note,ui::mono(11.0),ui::SECONDARY);}
            }
            let status=if self.query.trim().chars().count()<2{None}else if self.searching || !self.submitted {Some("Searching files…")}else if self.found.truncated{Some("Refine search to see more files")}else if self.found.errors>0{Some("Some search folders are unavailable")}else{None};
            let (status_rect,_)=ui.allocate_exact_size(vec2(ui.available_width(),16.0),Sense::hover());
            if let Some(status)=status{ui.painter().text(status_rect.left_center(),Align2::LEFT_CENTER,status,ui::mono(11.0),ui::SECONDARY);}
            let columns=grid_columns((ui.available_width()-ui.spacing().scroll.allocated_width()).max(1.0));
            let navigating=up||down||left||right;
            if navigating{self.grid_navigation=true;ui.memory_mut(|memory|memory.request_focus(query_id));}
            if down{self.selected=self.selected.saturating_add(columns);}
            if up && self.selected>=columns{self.selected-=columns;}
            if right{self.selected=self.selected.saturating_add(1);}
            if left{self.selected=self.selected.saturating_sub(1);}
            self.selected=self.selected.min(items.len().saturating_sub(1));
            if enter && !self.edit_folders{chosen=items.get(self.selected).map(|i|i.action.clone());}
            if let Some(Action::Copy(text))=&chosen{ui.ctx().copy_text(text.clone());}
            if items.is_empty(){ui::text(ui,if self.query.trim().is_empty(){"No applications found."}else{"No matches. Try another search."},ui::mono(13.0),ui::SECONDARY);}
            let mut scroll=ScrollArea::vertical().auto_shrink([false,false]);
            if navigating{scroll=scroll.vertical_scroll_offset(((self.selected/columns) as f32*(TILE_HEIGHT+ui.spacing().item_spacing.y)-ui.available_height()*0.5).max(0.0));}
            scroll.show_rows(ui,TILE_HEIGHT,items.len().div_ceil(columns),|ui,range|{
                let tile_width=((ui.available_width()-TILE_GAP*(columns-1) as f32)/columns as f32).max(1.0);
                for grid_row in range{
                    ui.horizontal(|ui|{
                        ui.spacing_mut().item_spacing.x=TILE_GAP;
                        for row in grid_row*columns..((grid_row+1)*columns).min(items.len()){
                            ui.push_id(("launcher-tile",row),|ui|{
                                if let Some(action)=self.tile(ui,theme,&items[row],row,tile_width){chosen=Some(action);}
                            });
                        }
                    });
                }
            });
        });
        if chosen.as_ref().is_some_and(|action|!action.keeps_palette_open()){self.open=false;}
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrows_move_across_tiles_and_between_grid_rows() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let entries=(0..20).map(|n|AppEntry {name:format!("App {n:02}"),desktop_id:format!("app-{n}.desktop"),exec:vec!["true".into()],icon:None,startup_wm_class:None,terminal:false}).collect();
        let mut launcher=Launcher{open:true,entries,..Default::default()};
        frame(&mut launcher,&ctx,vec![]);
        let key=|key|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE};
        frame(&mut launcher,&ctx,vec![key(Key::ArrowRight)]);
        assert_eq!(launcher.selected,1);
        frame(&mut launcher,&ctx,vec![key(Key::ArrowDown)]);
        assert!(launcher.selected>2,"Down must move one grid row, not one item");
        frame(&mut launcher,&ctx,vec![key(Key::ArrowUp)]);
        assert_eq!(launcher.selected,1);
        assert!(matches!(frame(&mut launcher,&ctx,vec![key(Key::Enter)]),Some(Action::Launch(app)) if app.name=="App 01"));
    }

    #[test]
    fn search_keeps_normal_caret_editing_before_grid_navigation() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut launcher=Launcher{open:true,..Default::default()};
        frame(&mut launcher,&ctx,vec![]);
        frame(&mut launcher,&ctx,vec![egui::Event::Text("abc".into())]);
        frame(&mut launcher,&ctx,vec![egui::Event::Key{key:Key::ArrowLeft,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE}]);
        frame(&mut launcher,&ctx,vec![egui::Event::Text("X".into())]);
        assert_eq!(launcher.query,"abXc");
        assert_eq!(launcher.selected,0);
    }

    #[test]
    fn default_app_tiles_form_a_grid_and_hide_command_details() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);ctx.enable_accesskit();
        for width in [240.0,900.0] {
            let entries=["Alpha editor with a long application name","Beta browser"].map(|name|AppEntry{name:name.into(),desktop_id:format!("{name}.desktop"),exec:vec!["hidden-executable-detail".into()],icon:None,startup_wm_class:None,terminal:false}).to_vec();
            let mut launcher=Launcher{open:true,entries,..Default::default()};
            let output=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(width,640.0))),..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{launcher.show(ui,ui.max_rect(),&Theme::load("signal"));});
            });
            let tree=output.platform_output.accesskit_update.expect("accessibility tree");
            let app_bounds:Vec<_>=tree.nodes.iter().filter_map(|(_,node)|node.label().filter(|label|label.contains(", APP:")).and_then(|_|node.bounds())).collect();
            assert_eq!(app_bounds.len(),2);
            assert!(!tree.nodes.iter().any(|(_,node)|node.label().is_some_and(|label|label.contains(", SETTING:"))),"default view should only show applications");
            if width>500.0 {assert_eq!(app_bounds[0].y0,app_bounds[1].y0);assert!(app_bounds[1].x0>app_bounds[0].x0);}
            else {assert!(app_bounds[1].y0>app_bounds[0].y0);}
            for bounds in &app_bounds {assert!(bounds.x0>=0.0 && bounds.x1<=width as f64);}
            let visible:Vec<_>=output.shapes.iter().filter_map(|shape|match &shape.shape{egui::epaint::Shape::Text(text)=>Some(text.galley.job.text.as_str()),_=>None}).collect();
            assert!(!visible.iter().any(|text|text.contains("hidden-executable-detail")));
            if width==900.0 {
                let bounds=app_bounds[1];
                let point=egui::pos2(((bounds.x0+bounds.x1)*0.5) as f32,((bounds.y0+bounds.y1)*0.5) as f32);
                frame(&mut launcher,&ctx,vec![egui::Event::PointerMoved(point),egui::Event::PointerButton{pos:point,button:egui::PointerButton::Primary,pressed:true,modifiers:Modifiers::NONE}]);
                let picked=frame(&mut launcher,&ctx,vec![egui::Event::PointerButton{pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:Modifiers::NONE}]);
                assert!(matches!(picked,Some(Action::Launch(app)) if app.name=="Beta browser"));
            }
        }
    }

    #[test]
    fn screen_readers_hear_each_result_with_its_kind(){
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);ctx.enable_accesskit();
        let mut launcher=Launcher{open:true,..Default::default()};
        frame(&mut launcher,&ctx,vec![]);
        frame(&mut launcher,&ctx,vec![egui::Event::Text("health".into())]);
        let output=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(900.0,600.0))),..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{launcher.show(ui,ui.max_rect(),&Theme::load("signal"));});
        });
        let tree=output.platform_output.accesskit_update.expect("accesskit is on");
        let labels:Vec<&str>=tree.nodes.iter().filter_map(|(_,node)|node.label()).collect();
        assert!(labels.iter().any(|l|l.contains("Settings")&&l.contains("SETTING")),"{labels:?}");
    }
    fn frame(launcher:&mut Launcher,ctx:&egui::Context,events:Vec<egui::Event>)->Option<Action>{ frame_copying(launcher,ctx,events).0 }
    fn frame_copying(launcher:&mut Launcher,ctx:&egui::Context,events:Vec<egui::Event>)->(Option<Action>,Option<String>){
        let mut picked=None;
        let output=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(900.0,600.0))),events,..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{picked=launcher.show(ui,ui.max_rect(),&Theme::load("signal"));});
        });
        let copied=output.platform_output.commands.iter().find_map(|c|match c{egui::OutputCommand::CopyText(t)=>Some(t.clone()),_=>None});
        (picked,copied)
    }
    #[test]
    fn typing_arithmetic_shows_a_calculation_and_enter_copies_it(){
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);let mut launcher=Launcher{open:true,..Default::default()};
        frame(&mut launcher,&ctx,vec![]);
        frame(&mut launcher,&ctx,vec![egui::Event::Text("2+2".into())]);
        let items=palette::results(&launcher.query,&launcher.entries,None,&launcher.catalog,&[]);
        assert_eq!((items[0].kind,items[0].label.as_str()),("CALC","2+2 = 4"));
        let enter=egui::Event::Key{key:Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE};
        let (picked,copied)=frame_copying(&mut launcher,&ctx,vec![enter]);
        assert_eq!(picked,Some(Action::Copy("4".into())));
        assert_eq!(copied.as_deref(),Some("4"));
    }
    #[test]
    fn keyboard_palette_opens_settings_and_composition_cannot_execute(){
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);let mut launcher=Launcher{open:true,..Default::default()};
        frame(&mut launcher,&ctx,vec![]);
        frame(&mut launcher,&ctx,vec![egui::Event::Text("health".into())]);
        let enter=||egui::Event::Key{key:Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:Modifiers::NONE};
        frame(&mut launcher,&ctx,vec![egui::Event::Ime(egui::ImeEvent::Preedit("文".into()))]);
        assert!(frame(&mut launcher,&ctx,vec![enter()]).is_none());assert!(launcher.open);
        assert!(frame(&mut launcher,&ctx,vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new())),enter()]).is_none());
        launcher.query="health".into();launcher.invalidate();
        assert!(matches!(frame(&mut launcher,&ctx,vec![enter()]),Some(Action::Settings("HEALTH"))));
        assert!(!launcher.open);
    }
}
