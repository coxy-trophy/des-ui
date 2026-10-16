//! Universal keyboard palette, with a single bounded background file-search worker.
use crate::{apps::{self,AppEntry},palette::{self,Action,Item},file_search::{self,Found},theme::Theme,ui};
use des_common::ipc::DesktopState;
use eframe::egui::{self,Align2,Key,Modifiers,Rect,ScrollArea,Sense,Stroke,StrokeKind,TextEdit,Ui,UiBuilder,vec2};
use std::path::PathBuf;
use std::sync::{Arc,Mutex,atomic::{AtomicU64,Ordering},mpsc::{self,SyncSender}};
use std::time::{Duration,Instant};

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
    pin_request:Option<String>,query:String,selected:usize,entries:Vec<AppEntry>,state:Option<DesktopState>,
    icons:crate::app_icons::IconCache,error:Option<String>,files:Files,found:Found,
    roots:Vec<PathBuf>,folders:String,edit_folders:bool,focus_query:bool,changed:Instant,submitted:bool,searching:bool,composing:bool,
}
impl Default for Launcher {
    fn default()->Self {
        let home=std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from("/"));
        let roots=vec![home.join("Documents"),home.join("Downloads"),home.join("Desktop")];
        Self {open:false,pin_request:None,query:String::new(),selected:0,entries:Vec::new(),state:None,icons:Default::default(),error:None,files:Default::default(),found:Default::default(),folders:roots.iter().map(|p|p.display().to_string()).collect::<Vec<_>>().join("; "),roots,edit_folders:false,focus_query:true,changed:Instant::now(),submitted:false,searching:false,composing:false}
    }
}
impl Launcher {
    pub fn take_pin_request(&mut self)->Option<String>{self.pin_request.take()}
    pub fn set_state(&mut self,state:Option<DesktopState>){self.state=state;}
    pub fn fail(&mut self,error:String){self.error=Some(error);self.open=true;}
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
    fn invalidate(&mut self){self.files.generation.fetch_add(1,Ordering::Relaxed);self.selected=0;self.found=Found::default();self.changed=Instant::now();self.submitted=false;self.searching=false;}
    fn pump_files(&mut self,ctx:&egui::Context){
        let generation=self.files.generation.load(Ordering::Relaxed);
        if let Some((version,found))=self.files.ready.lock().unwrap().take(){if version==generation{self.found=found;self.searching=false;}}
        if !self.composing && !self.submitted && self.query.trim().chars().count()>=2 && self.changed.elapsed()>=Duration::from_millis(200){
            let job=Job {generation,query:self.query.clone(),roots:self.roots.clone()};
            if self.files.tx.try_send(job).is_ok(){self.submitted=true;self.searching=true;}
        }
        if self.searching || (!self.submitted && self.query.trim().chars().count()>=2){ctx.request_repaint_after(Duration::from_millis(40));}
    }
    pub fn show(&mut self,ui:&mut Ui,rect:Rect,theme:&Theme)->Option<Action>{
        let mut ime_frame=self.composing;
        for event in ui.input(|i|i.events.clone()) {if let egui::Event::Ime(ime)=event {ime_frame=true;self.composing=matches!(ime,egui::ImeEvent::Enabled|egui::ImeEvent::Preedit(_));}}
        let (up,down,enter,escape)=if ime_frame{(false,false,false,false)}else{ui.input_mut(|i|(
            i.consume_key(Modifiers::NONE,Key::ArrowUp),i.consume_key(Modifiers::NONE,Key::ArrowDown),i.consume_key(Modifiers::NONE,Key::Enter),i.consume_key(Modifiers::NONE,Key::Escape)))};
        if escape {self.open=false;return None;}
        self.pump_files(ui.ctx());
        let margin=(rect.width()*0.06).clamp(12.0,72.0);
        let area=Rect::from_min_max(rect.min+vec2(margin,20.0),rect.max-vec2(margin,8.0));
        let mut chosen=None;
        ui.allocate_new_ui(UiBuilder::new().max_rect(area),|ui|{
            ui.horizontal_wrapped(|ui|{
                ui::tracked(ui,"COMMAND PALETTE",ui::display_bold(14.0),ui::TEXT,2.0);
                if ui::button(ui,theme,"FOLDERS",None,self.edit_folders).clicked(){self.edit_folders=!self.edit_folders;if !self.edit_folders{self.focus_query=true;}}
                if ui::button(ui,theme,"DONE",Some("ESC"),false).clicked(){self.open=false;}
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
                let field=ui.add_sized(vec2((ui.available_width()-75.0).max(40.0),38.0),TextEdit::singleline(&mut self.query).char_limit(4096).id(query_id).font(ui::mono(18.0)).hint_text("Apps, windows, settings, actions, files…"));
                if self.focus_query && !self.edit_folders {field.request_focus();self.focus_query=false;}
                if ui.add_enabled(!self.query.is_empty(),egui::Button::new("CLEAR")).clicked(){self.query.clear();field.request_focus();}
            });
            if before!=self.query{self.invalidate();}
            if let Some(error)=&self.error{ui::text(ui,error,ui::mono(12.0),ui::CAUTION);}
            let mut items=palette::results(&self.query,&self.entries,self.state.as_ref());
            items.extend(self.found.paths.iter().map(|path|Item{label:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),detail:path.display().to_string(),kind:"FILE",action:Action::File(path.clone()),pin:None,icon:None}));
            let status=if self.query.trim().chars().count()<2{"Type two characters to include files"}else if self.searching || !self.submitted {"Searching local filenames…"}else if self.found.truncated{"File search bounded · refine the query or folders"}else if self.found.errors>0{"Some folders were unavailable"}else{"Local filenames · hidden and build folders excluded"};
            ui::text(ui,&format!("{} results · {status}",items.len()),ui::mono(11.0),ui::SECONDARY);
            if down{self.selected=self.selected.saturating_add(1);}if up{self.selected=self.selected.saturating_sub(1);}
            self.selected=self.selected.min(items.len().saturating_sub(1));
            if enter && !self.edit_folders{chosen=items.get(self.selected).map(|i|i.action.clone());}
            if items.is_empty(){ui::text(ui,"No matching result. Try another name or choose different folders.",ui::mono(13.0),ui::SECONDARY);}
            let mut scroll=ScrollArea::vertical().auto_shrink([false,false]);
            if up||down{scroll=scroll.vertical_scroll_offset((self.selected as f32*(52.0+ui.spacing().item_spacing.y)-ui.available_height()*0.5).max(0.0));}
            scroll.show_rows(ui,52.0,items.len(),|ui,range|{
                for row in range{
                    let item=&items[row];let selected=row==self.selected;
                    let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width(),52.0),Sense::click());
                    let painter=ui.painter_at(rect);
                    painter.rect_filled(rect,0.0,if selected{theme.tint()}else if response.hovered(){ui::PANEL}else{ui::GROUND});
                    if selected{painter.rect_stroke(rect,0.0,Stroke::new(1.0,theme.main),StrokeKind::Inside);}
                    if let Some(icon)=&item.icon{self.icons.paint(ui,Rect::from_center_size(rect.left_center()+vec2(20.0,0.0),vec2(24.0,24.0)),Some(icon));}
                    let left=rect.left_top()+vec2(40.0,6.0);
                    painter.text(left,Align2::LEFT_TOP,crate::panels::truncate(&item.label,80),ui::display(16.0),ui::TEXT);
                    painter.text(left+vec2(0.0,24.0),Align2::LEFT_TOP,crate::panels::truncate(&item.detail,110),ui::mono(10.0),ui::SECONDARY);
                    painter.text(rect.right_top()-vec2(8.0,-8.0),Align2::RIGHT_TOP,item.kind,ui::mono(10.0),ui::LABEL);
                    if response.clicked(){chosen=Some(item.action.clone());}
                    if let Some(id)=&item.pin{response.context_menu(|ui|{if ui.button("Pin to dock").clicked(){self.pin_request=Some(id.clone());ui.close_menu();}});}
                }
            });
        });
        if chosen.is_some(){self.open=false;}
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(launcher:&mut Launcher,ctx:&egui::Context,events:Vec<egui::Event>)->Option<Action>{
        let mut picked=None;
        let _=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(900.0,600.0))),events,..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{picked=launcher.show(ui,ui.max_rect(),&Theme::load("signal"));});
        });picked
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
