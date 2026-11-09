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
    app_scan:Option<mpsc::Receiver<Vec<AppEntry>>>,app_scanned:Option<Instant>,
    pin_request:Option<String>,query:String,selected:usize,entries:Vec<AppEntry>,state:Option<DesktopState>,
    icons:crate::app_icons::IconCache,error:Option<String>,files:Files,found:Found,
    providers:commands::Worker,catalog:commands::Catalog,catalog_asked:bool,
    /// The clipboard history, when the shell watches it.
    clipboard:Option<clipboard::Handle>,
    /// What background actions (unit changes) reported: title, body, whether it went wrong.
    outcomes:Arc<Mutex<Vec<(String,String,bool)>>>,
    roots:Vec<PathBuf>,folders:String,edit_folders:bool,focus_query:bool,changed:Instant,submitted:bool,searching:bool,composing:bool,
    /// The kind of application shown, if one is chosen; the pinned ones; those opened lately.
    group:Option<&'static str>,pins:Vec<String>,recent:Vec<String>,
    /// Where each tile was drawn last, for the arrows; where each letter begins, and a jump asked for.
    tile_rects:Vec<(usize,Rect)>,sections:Vec<(char,f32)>,jump_to:Option<f32>,
}
impl Default for Launcher {
    fn default()->Self {
        let home=std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from("/"));
        let roots=vec![home.join("Documents"),home.join("Downloads"),home.join("Desktop")];
        Self {open:false,grid_navigation:false,app_scan:None,app_scanned:None,pin_request:None,query:String::new(),selected:0,entries:Vec::new(),state:None,icons:Default::default(),error:None,files:Default::default(),found:Default::default(),providers:Default::default(),catalog:Default::default(),catalog_asked:false,clipboard:None,outcomes:Default::default(),folders:roots.iter().map(|p|p.display().to_string()).collect::<Vec<_>>().join("; "),roots,edit_folders:false,focus_query:true,changed:Instant::now(),submitted:false,searching:false,composing:false,group:None,pins:Vec::new(),recent:Vec::new(),tile_rects:Vec::new(),sections:Vec::new(),jump_to:None}
    }
}
impl Launcher {
    pub fn take_pin_request(&mut self)->Option<String>{self.pin_request.take()}
    pub fn set_state(&mut self,state:Option<DesktopState>){self.state=state;}
    /// The applications pinned to the dock, shown first.
    pub fn set_pins(&mut self,pins:&[String]){if self.pins!=pins{self.pins=pins.to_vec();}}
    pub fn fail(&mut self,error:String){self.error=Some(error);self.open=true;}
    pub fn set_clipboard(&mut self,handle:clipboard::Handle){self.clipboard=Some(handle);}
    pub fn clipboard(&self)->Option<&clipboard::Handle>{self.clipboard.as_ref()}
    /// Where a background action reports how it went; drained by the shell into notices.
    pub fn outcomes(&self)->Arc<Mutex<Vec<(String,String,bool)>>>{self.outcomes.clone()}
    pub fn take_outcomes(&mut self)->Vec<(String,String,bool)>{std::mem::take(&mut *self.outcomes.lock().unwrap_or_else(|e|e.into_inner()))}
    pub fn open(&mut self) {
        if self.app_scan.is_none() && self.app_scanned.is_none_or(|at|at.elapsed()>Duration::from_secs(30)) {
            let (tx,rx)=mpsc::sync_channel(1);
            std::thread::spawn(move||{let _=tx.send(apps::scan());});
            self.app_scan=Some(rx);
        }
        self.query.clear();self.error=None;self.open=true;self.edit_folders=false;self.focus_query=true;self.group=None;
        if let Some(text)=des_common::input::config_path().and_then(|p|std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                if let Some(("recent_apps",value))=line.split_once('=').map(|(k,v)|(k.trim(),v.trim())) {
                    if let Ok(recent)=serde_json::from_str::<Vec<String>>(value){self.recent=recent.into_iter().take(RECENT).collect();}
                }
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
    /// One application or result as a tile: its icon, or its first letter on a colour
    /// of its own, and its name over two lines at most, broken between words.
    fn tile(&mut self,ui:&mut Ui,theme:&Theme,item:&Item,index:usize,tile_width:f32)->Option<Action>{
        let mut chosen=None;
        let (rect,response)=ui.allocate_exact_size(vec2(tile_width,TILE_HEIGHT),Sense::click());
        self.tile_rects.push((index,rect));
        if response.gained_focus(){self.selected=index;}
        let selected=index==self.selected;
        response.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel,true,selected,format!("{}, {}: {}",item.label,item.kind,item.detail)));
        let painter=ui.painter_at(rect);
        if selected || response.hovered() || response.has_focus() {
            painter.rect_filled(rect,ui::SURFACE_RADIUS+2.0,if selected{theme.main.gamma_multiply(0.14)}else{ui::TRACK});
        }
        if selected || response.has_focus(){painter.rect_stroke(rect,ui::SURFACE_RADIUS+2.0,Stroke::new(1.0,theme.main),StrokeKind::Inside);}
        let icon=Rect::from_center_size(egui::pos2(rect.center().x,rect.top()+36.0),vec2(44.0,44.0));
        if item.kind=="APP"{
            if !self.icons.paint_found(ui,icon,item.icon.as_deref()) {
                painter.rect_filled(icon,11.0,letter_colour(&item.label));
                let letter=item.label.chars().find(|c|c.is_alphanumeric()).map(|c|c.to_uppercase().to_string()).unwrap_or_default();
                painter.text(icon.center(),Align2::CENTER_CENTER,letter,ui::display_bold(18.0),ui::GROUND);
            }
        }else{
            let symbol=match item.kind{"SETTING"|"UNIT"=>ui::Icon::Settings,"WINDOW"|"HOST"=>ui::Icon::Display,"CLIP"=>ui::Icon::Copy,"CALC"|"COMMAND"=>ui::Icon::Keyboard,_=>ui::Icon::Apps};
            ui::paint_icon(&painter,icon,symbol,ui::SECONDARY,false);
        }
        let mut label=egui::text::LayoutJob::simple(item.label.clone(),ui::display(12.5),ui::TEXT,(tile_width-14.0).max(1.0));
        label.halign=egui::Align::Center;
        label.wrap.max_rows=2;
        label.wrap.break_anywhere=false;
        label.wrap.overflow_character=Some('…');
        let label=ui.fonts(|fonts|fonts.layout_job(label));
        painter.galley(egui::pos2(rect.center().x-label.rect.center().x,rect.top()+66.0-label.rect.top()),label,ui::TEXT);
        if item.kind=="APP" && matches!(&item.action,Action::Command(des_common::ipc::Command::Activate(_))) {
            painter.circle_filled(egui::pos2(rect.center().x,rect.bottom()-6.0),2.0,theme.main);
        }
        if item.kind!="APP"{painter.text(rect.right_top()+vec2(-6.0,5.0),Align2::RIGHT_TOP,item.kind,ui::mono(9.0),ui::LABEL);}
        response.clone().on_hover_text(format!("{}\n{} · {}",item.label,item.kind,item.detail)).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked(){chosen=Some(item.action.clone());if let Action::Copy(text)=&item.action{ui.ctx().copy_text(text.clone());}}
        if let Some(id)=&item.pin{let pinned=self.pins.contains(id);response.context_menu(|ui|{if !pinned && ui.button("Pin application").clicked(){self.pin_request=Some(id.clone());ui.close_menu();}});}
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
        if chosen.is_some(){self.remember(item);}
        chosen
    }

    /// An application opened from here goes to the front of the recent ones.
    fn remember(&mut self,item:&Item){
        let Some(id)=item.pin.clone().filter(|_|item.kind=="APP") else {return;};
        self.recent.retain(|known|known!=&id);
        self.recent.insert(0,id);
        self.recent.truncate(RECENT);
        // Tests open applications too; they must never write the user's own settings.
        #[cfg(not(test))]
        if let Ok(text)=serde_json::to_string(&self.recent){let _=crate::config::set("recent_apps",Some(&text));}
    }

    /// The tile nearest the selected one in a direction, by where they were drawn.
    fn step(&self,dx:f32,dy:f32)->Option<usize>{
        let from=self.tile_rects.iter().find(|(index,_)|*index==self.selected).map(|(_,rect)|rect.center())?;
        self.tile_rects.iter().filter_map(|(index,rect)|{
            let d=rect.center()-from;
            let along=d.x*dx+d.y*dy;
            let across=(d.x*dy).abs()+(d.y*dx).abs();
            (along>4.0).then_some((*index,along+across*2.5))
        }).min_by(|a,b|a.1.total_cmp(&b.1)).map(|(index,_)|index)
    }

    /// With nothing typed: the kinds to narrow to, what is pinned and was opened
    /// lately, then every application from A to Z.
    fn browse(&mut self,ui:&mut Ui,theme:&Theme,apps:&[Item],keys:(bool,bool,bool,bool,bool))->Option<Action>{
        let (up,down,left,right,enter)=keys;
        let group_of=|item:&Item|item.pin.as_ref().and_then(|id|self.entries.iter().find(|entry|&entry.desktop_id==id)).map_or("Other",|entry|entry.group());
        ui.horizontal_wrapped(|ui|{
            ui.spacing_mut().item_spacing=vec2(6.0,6.0);
            if ui::chip(ui,theme,&format!("All {}",apps.len()),None,self.group.is_none()).clicked(){self.group=None;self.selected=0;}
            for group in crate::apps::GROUPS {
                let count=apps.iter().filter(|item|group_of(item)==group).count();
                if count==0 {continue;}
                if ui::chip(ui,theme,&format!("{group} {count}"),None,self.group==Some(group)).clicked(){self.group=Some(group);self.selected=0;}
            }
        });
        ui.add_space(10.0);
        let find=|id:&String|apps.iter().find(|item|item.pin.as_ref()==Some(id)).cloned();
        let (pinned,recent):(Vec<Item>,Vec<Item>)=if self.group.is_none(){
            (self.pins.iter().filter_map(find).take(8).collect(),self.recent.iter().filter_map(find).take(8).collect())
        }else{(Vec::new(),Vec::new())};
        let listed:Vec<Item>=apps.iter().filter(|item|self.group.is_none_or(|group|group_of(item)==group)).cloned().collect();
        let tiles:Vec<Item>=pinned.iter().chain(recent.iter()).chain(listed.iter()).cloned().collect();
        let navigating=up||down||left||right;
        if navigating {
            let moved=if right{self.step(1.0,0.0)}else if left{self.step(-1.0,0.0)}else if down{self.step(0.0,1.0)}else{self.step(0.0,-1.0)};
            if let Some(index)=moved{self.selected=index;}
            self.grid_navigation=true;
        }
        self.selected=self.selected.min(tiles.len().saturating_sub(1));
        let mut chosen=None;
        if enter && !self.edit_folders { if let Some(item)=tiles.get(self.selected){chosen=Some(item.action.clone());self.remember(&item.clone());} }
        if tiles.is_empty(){
            ui::text(ui,if self.app_scan.is_some(){"Loading applications…"}else{"No applications found."},ui::mono(13.0),ui::SECONDARY);
            return chosen;
        }
        self.tile_rects.clear();
        let letters:Vec<char>={let mut seen=Vec::new();for item in &listed{let c=initial(&item.label);if !seen.contains(&c){seen.push(c);}}seen};
        let mut jump=None;
        ui.horizontal_top(|ui|{
            let strip=22.0;
            let width=(ui.available_width()-strip-8.0).max(120.0);
            let height=ui.available_height().max(0.0);
            ui.allocate_ui_with_layout(vec2(width,height),egui::Layout::top_down(egui::Align::Min),|ui|{
                ui.set_width(width);
                let mut scroll=ScrollArea::vertical().id_salt("launcher-browse").auto_shrink([false,false]);
                if let Some(offset)=self.jump_to.take(){scroll=scroll.vertical_scroll_offset(offset);}
                let shown=scroll.show(ui,|ui|{
                    let top=ui.min_rect().top();
                    let width=ui.available_width();
                    let columns=grid_columns(width);
                    let tile_width=((width-TILE_GAP*(columns-1) as f32)/columns as f32).max(1.0);
                    let mut index=0;
                    let row_of=|this:&mut Self,ui:&mut Ui,items:&[Item],start:&mut usize,columns:usize,tile_width:f32,chosen:&mut Option<Action>|{
                        for row in items.chunks(columns){
                            ui.horizontal(|ui|{
                                ui.spacing_mut().item_spacing.x=TILE_GAP;
                                for item in row{
                                    let at=*start;
                                    ui.push_id(("launcher-tile",at),|ui|{ if let Some(action)=this.tile(ui,theme,item,at,tile_width){*chosen=Some(action);} });
                                    *start+=1;
                                }
                            });
                        }
                    };
                    if !pinned.is_empty() || !recent.is_empty() {
                        let half=((width-28.0)/2.0).max(tile_width);
                        let per=((half+TILE_GAP)/(tile_width+TILE_GAP)).floor().max(1.0) as usize;
                        ui.horizontal_top(|ui|{
                            ui.spacing_mut().item_spacing.x=28.0;
                            for (title,hint,list) in [("PINNED","Right-click an app to pin it",&pinned),("RECENT","Last opened first",&recent)] {
                                ui.allocate_ui_with_layout(vec2(half,0.0),egui::Layout::top_down(egui::Align::Min),|ui|{
                                    ui.set_width(half);
                                    heading(ui,title,hint);
                                    if list.is_empty(){ui::text(ui,if title=="PINNED"{"Nothing pinned yet."}else{"Nothing opened yet."},ui::mono(11.0),ui::LABEL);}
                                    let shown:Vec<Item>=list.iter().take(per).cloned().collect();
                                    let mut at=if title=="PINNED"{0}else{pinned.len()};
                                    row_of(self,ui,&shown,&mut at,per,tile_width,&mut chosen);
                                });
                            }
                        });
                        index=pinned.len()+recent.len();
                        ui.add_space(8.0);
                        let (line,_)=ui.allocate_exact_size(vec2(width,1.0),Sense::hover());
                        ui.painter().hline(line.x_range(),line.center().y,Stroke::new(1.0,ui::LINE));
                        ui.add_space(8.0);
                    }
                    heading(ui,&self.group.map_or("ALL APPLICATIONS".to_string(),|group|group.to_uppercase()),&format!("{} · A–Z",listed.len()));
                    let mut sections=Vec::new();
                    let mut start=0;
                    while start<listed.len(){
                        let letter=initial(&listed[start].label);
                        let end=listed[start..].iter().position(|item|initial(&item.label)!=letter).map_or(listed.len(),|n|start+n);
                        sections.push((letter,ui.cursor().top()-top));
                        ui.label(egui::RichText::new(letter.to_string()).font(ui::mono(12.0)).color(theme.main));
                        row_of(self,ui,&listed[start..end],&mut index,columns,tile_width,&mut chosen);
                        start=end;
                    }
                    sections
                });
                self.sections=shown.inner;
                if navigating { if let Some((_,rect))=self.tile_rects.iter().find(|(index,_)|*index==self.selected){ui.scroll_to_rect(*rect,None);} }
            });
            // A strip of letters down the side jumps to where each begins.
            ui.allocate_ui_with_layout(vec2(strip,height),egui::Layout::top_down(egui::Align::Center),|ui|{
                ui.spacing_mut().item_spacing.y=1.0;
                for letter in &letters {
                    let response=ui.add(egui::Label::new(egui::RichText::new(letter.to_string()).font(ui::mono(10.0)).color(ui::SECONDARY)).sense(Sense::click()));
                    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){jump=Some(*letter);}
                }
            });
        });
        if let Some(letter)=jump { if let Some((_,offset))=self.sections.iter().find(|(c,_)|*c==letter){self.jump_to=Some(*offset);} }
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
        if let Some(scan)=&self.app_scan {
            match scan.try_recv() {
                Ok(entries)=>{self.entries=entries;self.app_scan=None;self.app_scanned=Some(Instant::now());},
                Err(mpsc::TryRecvError::Disconnected)=>{self.app_scan=None;self.app_scanned=Some(Instant::now());},
                Err(mpsc::TryRecvError::Empty)=>ui.ctx().request_repaint_after(Duration::from_millis(50)),
            }
        }
        self.pump_files(ui.ctx());
        self.pump_catalog(ui.ctx());
        let margin=(rect.width()*0.04).clamp(12.0,48.0);
        let area=Rect::from_min_max(rect.min+vec2(margin,22.0),rect.max-vec2(margin,10.0));
        let mut chosen=None;
        ui.allocate_new_ui(UiBuilder::new().max_rect(area),|ui|{
            ui.spacing_mut().item_spacing.y=10.0;
            let before=self.query.clone();
            let query_id=ui.id().with("palette-query");
            // The search field: one rounded field, with what the keys do at its end.
            ui.horizontal(|ui|{
                ui.spacing_mut().item_spacing.x=8.0;
                let width=(ui.available_width()-2.0*(40.0+8.0)).max(120.0);
                let (field_rect,_)=ui.allocate_exact_size(vec2(width,52.0),Sense::hover());
                ui.painter().rect_filled(field_rect,26.0,ui::PANEL);
                ui.painter().rect_stroke(field_rect,26.0,Stroke::new(1.0,theme.main),StrokeKind::Inside);
                ui::paint_icon(ui.painter(),Rect::from_center_size(field_rect.left_center()+vec2(26.0,0.0),vec2(18.0,18.0)),ui::Icon::Search,ui::SECONDARY,false);
                let hint=if self.query.is_empty() && width>=520.0 {"↵ open · arrows move · Esc close"}else{""};
                let hint=ui.painter().layout_no_wrap(hint.into(),ui::mono(11.0),ui::LABEL);
                let clear_room=if self.query.is_empty(){0.0}else{36.0};
                let right=(field_rect.right()-20.0-hint.size().x-clear_room).max(field_rect.left()+46.0+20.0);
                let text_rect=Rect::from_min_max(field_rect.left_top()+vec2(46.0,0.0),egui::pos2(right,field_rect.bottom()));
                ui.painter().galley(egui::pos2(field_rect.right()-18.0-hint.size().x,field_rect.center().y-hint.size().y/2.0),hint,ui::LABEL);
                let mut inner=ui.new_child(UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let field=inner.add_sized(text_rect.size(),TextEdit::singleline(&mut self.query).frame(false).vertical_align(egui::Align::Center).char_limit(4096).id(query_id).font(ui::display(17.0)).hint_text(crate::ui::hint("Search apps, files, settings and actions")));
                if self.focus_query && !self.edit_folders {field.request_focus();self.focus_query=false;}
                if field.clicked(){self.grid_navigation=false;}
                if !self.query.is_empty() {
                    let spot=Rect::from_center_size(egui::pos2(field_rect.right()-24.0,field_rect.center().y),vec2(28.0,28.0));
                    let mut clear_ui=ui.new_child(UiBuilder::new().max_rect(spot));
                    let clear=ui::icon_button(&mut clear_ui,theme,ui::Icon::Close,28.0,false).on_hover_text("Clear search");
                    clear.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,"Clear search"));
                    if clear.clicked(){self.query.clear();field.request_focus();}
                }
                let folders=ui::icon_button(ui,theme,ui::Icon::Settings,40.0,self.edit_folders).on_hover_text("Where file search looks");
                folders.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,"Search folders"));
                if folders.clicked(){self.edit_folders=!self.edit_folders;if !self.edit_folders{self.focus_query=true;}}
                if ui::icon_button(ui,theme,ui::Icon::Close,40.0,false).on_hover_text("Close · Escape").clicked(){self.open=false;}
            });
            if self.edit_folders {
                ui::text(ui,"Local file search folders · absolute paths separated by semicolons",ui::mono(11.0),ui::SECONDARY);
                ui.add(TextEdit::singleline(&mut self.folders).margin(eframe::egui::vec2(10.0, 4.0)).vertical_align(eframe::egui::Align::Center).char_limit(32768).font(ui::mono(12.0)).desired_width(f32::INFINITY));
                if ui::button(ui,theme,"SAVE FOLDERS",None,false).clicked(){match file_search::parse_roots(&self.folders){
                    Ok(roots)=>match crate::config::set("search_folders",Some(&serde_json::to_string(&roots).unwrap())){Ok(_)=>{self.roots=roots;self.edit_folders=false;self.focus_query=true;self.error=None;self.invalidate();},Err(e)=>self.error=Some(e.to_string())},
                    Err(e)=>self.error=Some(e.into()),
                }}
            }
            if before!=self.query{self.invalidate();}
            if let Some(error)=&self.error{ui::text(ui,error,ui::mono(12.0),ui::CAUTION);}
            let clips=self.clipboard.as_ref().map(clipboard::Handle::entries).unwrap_or_default();
            let mut items=palette::results(&self.query,&self.entries,self.state.as_ref(),&self.catalog,&clips);
            items.extend(self.found.paths.iter().map(|path|Item{label:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),detail:path.display().to_string(),kind:"FILE",action:Action::File(path.clone()),pin:None,icon:None,clip:None}));
            // Browsing starts with applications; typed queries retain every provider.
            if self.query.trim().is_empty(){
                items.retain(|item|item.kind=="APP");
                chosen=self.browse(ui,theme,&items,(up,down,left,right,enter));
                if let Some(Action::Copy(text))=&chosen{ui.ctx().copy_text(text.clone());}
                return;
            }
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
            if enter && !self.edit_folders{chosen=items.get(self.selected).map(|i|i.action.clone());if let Some(item)=items.get(self.selected).cloned(){self.remember(&item);}}
            if let Some(Action::Copy(text))=&chosen{ui.ctx().copy_text(text.clone());}
            if items.is_empty() && self.app_scan.is_none(){ui::text(ui,"No matches. Try another search.",ui::mono(13.0),ui::SECONDARY);}
            let mut scroll=ScrollArea::vertical().auto_shrink([false,false]);
            if navigating{scroll=scroll.vertical_scroll_offset(((self.selected/columns) as f32*(TILE_HEIGHT+ui.spacing().item_spacing.y)-ui.available_height()*0.5).max(0.0));}
            self.tile_rects.clear();
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

/// How many applications the launcher remembers having opened.
const RECENT: usize = 8;

/// The letter an application is listed under: its first letter or digit, upper case.
fn initial(label:&str)->char {
    label.chars().find(|c|c.is_alphanumeric()).map_or('#',|c|if c.is_ascii_digit(){'#'}else{c.to_uppercase().next().unwrap_or(c)})
}

/// A colour of its own for an application drawn as its first letter.
fn letter_colour(name:&str)->egui::Color32 {
    const COLOURS:[egui::Color32;6]=[egui::Color32::from_rgb(0x6F,0xA8,0xDC),egui::Color32::from_rgb(0x5F,0xD0,0x8A),egui::Color32::from_rgb(0xE8,0x83,0x4A),egui::Color32::from_rgb(0x8C,0x9E,0xFF),egui::Color32::from_rgb(0xD9,0x8B,0xC8),egui::Color32::from_rgb(0xE6,0xC3,0x5C)];
    let sum=name.bytes().fold(7u32,|sum,byte|sum.wrapping_mul(31).wrapping_add(byte as u32));
    COLOURS[sum as usize%COLOURS.len()]
}

/// A section's name at the left and a note at the right.
fn heading(ui:&mut Ui,title:&str,note:&str){
    let (rect,_)=ui.allocate_exact_size(vec2(ui.available_width(),20.0),Sense::hover());
    let title=ui::galley(ui,title,ui::display_bold(12.5),ui::SECONDARY,1.2);
    ui.painter().galley(egui::pos2(rect.left(),rect.center().y-title.size().y/2.0),title,ui::SECONDARY);
    ui.painter().text(rect.right_center(),Align2::RIGHT_CENTER,note,ui::mono(11.0),ui::LABEL);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrows_move_across_tiles_and_between_grid_rows() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let entries=(0..20).map(|n|AppEntry {name:format!("App {n:02}"),desktop_id:format!("app-{n}.desktop"),exec:vec!["true".into()],icon:None,startup_wm_class:None,terminal:false, categories: Vec::new()}).collect();
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
    fn applications_sort_into_kinds_and_letters_and_remember_what_was_opened() {
        let app=|name:&str,categories:&[&str]|AppEntry{name:name.into(),desktop_id:format!("{name}.desktop"),exec:vec!["true".into()],icon:None,startup_wm_class:None,terminal:false,categories:categories.iter().map(|c|c.to_string()).collect()};
        assert_eq!(app("Firefox",&["Network","WebBrowser"]).group(),"Internet");
        assert_eq!(app("Burp",&["X-Kali-Web"]).group(),"Security");
        assert_eq!(app("binwalk",&["kali-forensics"]).group(),"Security");
        assert_eq!(app("Thing",&[]).group(),"Other");
        assert_eq!(initial("aircrack-ng"),'A');
        assert_eq!(initial("7zip"),'#');
        let mut launcher=Launcher{open:true,..Default::default()};
        let item=|id:&str|Item{label:id.into(),detail:String::new(),kind:"APP",action:Action::Files,pin:Some(id.into()),icon:None,clip:None};
        for id in ["a","b","a","c"] { launcher.remember(&item(id)); }
        assert_eq!(launcher.recent,["c","a","b"],"latest first, without repeats");
        assert_eq!(launcher.recent.len().min(RECENT),3);
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
        // Both under A, so they share a section and, where there is room, a row.
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);ctx.enable_accesskit();
        for width in [240.0,900.0] {
            let entries=["Alpha editor with a long application name","Atlas browser"].map(|name|AppEntry{name:name.into(),desktop_id:format!("{name}.desktop"),exec:vec!["hidden-executable-detail".into()],icon:None,startup_wm_class:None,terminal:false, categories: Vec::new()}).to_vec();
            let mut launcher=Launcher{open:true,entries,..Default::default()};
            let output=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(width,640.0))),..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{launcher.show(ui,ui.max_rect(),&Theme::load("signal"));});
            });
            let tree=output.platform_output.accesskit_update.expect("accessibility tree");
            let app_bounds:Vec<_>=tree.nodes.iter().filter_map(|(_,node)|node.label().filter(|label|label.contains(", APP:")).and_then(|_|node.bounds())).collect();
            assert_eq!(app_bounds.len(),2);
            assert!(!tree.nodes.iter().any(|(_,node)|node.label().is_some_and(|label|label.contains(", SETTING:"))),"default view should only show applications");
            if width>500.0 {assert_eq!(app_bounds[0].y0,app_bounds[1].y0);assert!(app_bounds[1].x0>app_bounds[0].x0);}
            else {assert!((app_bounds[1].y0-app_bounds[0].y0).abs()>50.0,"one column when narrow: {app_bounds:?}");}
            for bounds in &app_bounds {assert!(bounds.x0>=0.0 && bounds.x1<=width as f64);}
            let visible:Vec<_>=output.shapes.iter().filter_map(|shape|match &shape.shape{egui::epaint::Shape::Text(text)=>Some(text.galley.job.text.as_str()),_=>None}).collect();
            assert!(!visible.iter().any(|text|text.contains("hidden-executable-detail")));
            if width==900.0 {
                let bounds=app_bounds[1];
                let point=egui::pos2(((bounds.x0+bounds.x1)*0.5) as f32,((bounds.y0+bounds.y1)*0.5) as f32);
                frame(&mut launcher,&ctx,vec![egui::Event::PointerMoved(point),egui::Event::PointerButton{pos:point,button:egui::PointerButton::Primary,pressed:true,modifiers:Modifiers::NONE}]);
                let picked=frame(&mut launcher,&ctx,vec![egui::Event::PointerButton{pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:Modifiers::NONE}]);
                assert!(matches!(picked,Some(Action::Launch(app)) if app.name=="Atlas browser"));
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
