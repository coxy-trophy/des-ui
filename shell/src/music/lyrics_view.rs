//! Stable lyric layout and playback-follow presentation, independent of fetching.
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn manual_browsing_survives_line_changes_until_explicit_resume() {
        let mut follow=Follow::default();follow.browse(240.0);
        assert_eq!(follow.offset(1.0,500.0,true),240.0);
        assert_eq!(follow.offset(1.1,700.0,true),240.0);
        follow.resume();assert!(follow.offset(1.2,700.0,true)<700.0);
    }
    #[test] fn reduced_motion_and_seeks_align_without_a_long_sweep() {
        let mut follow=Follow::default();follow.offset(0.0,0.0,true);
        assert_eq!(follow.offset(0.1,200.0,false),200.0);
        follow.snap_next=true;assert_eq!(follow.offset(0.2,800.0,true),800.0);
    }
    #[test] fn spring_is_frame_rate_independent_and_settles() {
        let run=|hz:u32|{let mut spring=Spring::default();spring.step(0.0,0.0,true);spring.target=300.0;
            for i in 1..=hz {spring.step(i as f64/hz as f64,300.0,true);}spring};
        let a=run(60);let b=run(120);assert!((a.position-b.position).abs()<0.01);assert!(!a.active());
    }
}

#[derive(Default)]
struct Spring { position:f64, velocity:f64, target:f64, time:Option<f64> }
impl Spring {
    fn snap(&mut self, now:f64,target:f64) {self.position=target;self.target=target;self.velocity=0.0;self.time=Some(now);}
    fn active(&self)->bool {(self.position-self.target).abs()>0.1 || self.velocity.abs()>1.0}
    fn step(&mut self,now:f64,target:f64,animate:bool)->f64 {
        let dt=now-self.time.unwrap_or(now);
        if !animate || self.time.is_none() || !(0.0..=0.5).contains(&dt) {self.snap(now,target);return target;}
        let w=19.0;let error=self.position-self.target;let b=self.velocity+w*error;let decay=(-w*dt).exp();
        self.position=self.target+(error+b*dt)*decay;self.velocity=(self.velocity-w*b*dt)*decay;
        self.target=target;self.time=Some(now);
        if !self.active(){self.snap(now,target);}
        self.position
    }
}
struct Follow {following:bool,spring:Spring,snap_next:bool}
impl Default for Follow {fn default()->Self{Self{following:true,spring:Spring::default(),snap_next:true}}}
impl Follow {
    fn browse(&mut self,offset:f64){self.following=false;self.spring.position=offset;self.spring.target=offset;self.spring.velocity=0.0;}
    fn resume(&mut self){self.following=true;self.snap_next=false;}
    fn offset(&mut self,now:f64,target:f64,animate:bool)->f64{
        if !self.following {self.spring.snap(now,self.spring.position);return self.spring.position;}
        let snap=self.snap_next;self.snap_next=false;self.spring.step(now,target,animate&&!snap)
    }
}

use super::{Lyrics,Now};
use crate::{theme::Theme,ui};
use eframe::egui::{self,Color32,Galley,Rect,ScrollArea,Sense,Shape,Ui,pos2,vec2};
use std::{collections::{HashMap,hash_map::DefaultHasher},hash::{Hash,Hasher},sync::Arc,time::Duration};
#[derive(Default)]
pub(super) struct View {
    follow:Follow,
    track:String,
    layout_key:u64,
    rows:Vec<(Arc<Galley>,f32)>,
    total:f32,
    current:Option<usize>,
    emphasis:HashMap<usize,Emphasis>,
    last_render:Option<f64>,
    last_position:Option<(u32,f64)>,
    pending_seek:Option<(usize,f64)>,
}
#[derive(Clone,Copy)]
struct Emphasis {from:f32,goal:f32,since:f64}
impl Emphasis {
    fn value(self,time:f64)->f32 {let t=((time-self.since)/0.25).clamp(0.0,1.0) as f32;self.from+(self.goal-self.from)*t*t*(3.0-2.0*t)}
    fn active(self,time:f64)->bool {self.from!=self.goal && time-self.since<0.25}
}
impl View {
    fn focus(&mut self,current:Option<usize>,time:f64) {
        if current!=self.current {
            if let Some(old)=self.current.and_then(|i|self.emphasis.get_mut(&i)) {old.from=old.value(time);old.goal=0.0;old.since=time;}
            if let Some(index)=current {let from=self.emphasis.get(&index).map_or(0.0,|e|e.value(time));self.emphasis.insert(index,Emphasis{from,goal:1.0,since:time});}
            self.current=current;
        }
        self.emphasis.retain(|_,e|e.goal!=0.0 || e.active(time));
    }
    pub fn reset(&mut self){*self=Self::default();}
    pub fn show(&mut self,ui:&mut Ui,theme:&Theme,now:&Now,lyrics:&Lyrics,animate:bool)->Option<u32>{
        let time=ui.input(|i|i.time);
        if self.track!=lyrics.track {self.reset();self.track=lyrics.track.clone();}
        let position=now.position_ms();let current=lyrics.current(position);
        if let Some((old,at))=self.last_position {
            let expected=if now.playing{(time-at).max(0.0)*1000.0}else{0.0};
            if ((position as f64-old as f64)-expected).abs()>1500.0 {self.follow.snap_next=true;}
        }
        self.last_position=Some((position,time));
        self.focus(current,time);
        if !animate || !now.playing {for e in self.emphasis.values_mut(){e.from=e.goal;e.since=time;}}
        if lyrics.lines.is_empty(){ui::text(ui,"No lyrics are available for this track.",ui::display(14.0),ui::SECONDARY);return None;}
        if lyrics.synced {
            ui.horizontal(|ui|{
                ui.set_min_height(ui::CONTROL_HEIGHT);
                if self.follow.following {ui::meta(ui,"FOLLOWING PLAYBACK",ui::LABEL);}
                else if ui::button(ui,theme,"Return to current line",None,true).clicked(){self.follow.resume();}
            });
        } else {ui::text(ui,"These words are not timed to the music.",ui::display(11.0),ui::LABEL);}
        let width=(ui.available_width()-18.0).max(40.0);
        let height=ui.available_height().max(40.0);
        let mut hash=DefaultHasher::new();lyrics.lines.hash(&mut hash);width.to_bits().hash(&mut hash);ui.ctx().pixels_per_point().to_bits().hash(&mut hash);
        let key=hash.finish();let reopened=self.last_render.is_some_and(|last|time-last>2.0);
        if self.layout_key!=key || reopened {
            self.rows.clear();self.total=0.0;
            for (_,text) in &lyrics.lines {
                let text=if text.is_empty(){"· · ·"}else{text};
                let galley=ui.painter().layout(text.to_string(),ui::display_bold(26.0),Color32::WHITE,(width-16.0).max(24.0));
                let offset=self.total;self.total+=galley.size().y+24.0;self.rows.push((galley,offset));
            }
            self.layout_key=key;self.follow.snap_next=true;
        }
        self.last_render=Some(time);
        if self.pending_seek.is_some_and(|(index,at)|current==Some(index)||time-at>1.0) {self.pending_seek=None;}
        let focus=self.pending_seek.map(|(index,_)|index).or(current);
        let target=focus.and_then(|i|self.rows.get(i)).map_or(0.0,|(_,y)|*y);
        let following=lyrics.synced&&self.follow.following;
        let desired=self.follow.offset(time,target as f64,animate).clamp(0.0,self.total as f64) as f32;
        let mut scroll=ScrollArea::vertical().id_salt("music-lyric-motion").auto_shrink([false,false]).max_height(height);
        if following {scroll=scroll.vertical_scroll_offset(desired);}
        let mut seek=None;
        // Read intent before ScrollArea consumes the wheel delta.
        let (wheel,dragging)=ui.input(|i|(i.raw_scroll_delta.y!=0.0 || i.smooth_scroll_delta.y!=0.0,i.pointer.any_down()));
        let result=scroll.show(ui,|ui|{
            let (content,_)=ui.allocate_exact_size(vec2(width,self.total+height),Sense::hover());
            let painter=ui.painter().clone();let clip=ui.clip_rect();
            for (index,(galley,y)) in self.rows.iter().enumerate(){
                let top=content.top()+height*0.36+y;
                let row=Rect::from_min_size(pos2(content.left()+8.0,top),vec2(width-8.0,galley.size().y+24.0));
                if !row.intersects(clip){continue;}
                let emphasis=if !lyrics.synced{1.0}else{self.emphasis.get(&index).map_or(0.0,|e|e.value(time))};
                let response=ui.interact(row,ui.id().with(("lyric",index)),if lyrics.synced{Sense::click()}else{Sense::hover()});
                response.widget_info(||egui::WidgetInfo::labeled(if lyrics.synced{egui::WidgetType::Button}else{egui::WidgetType::Label},lyrics.synced,&lyrics.lines[index].1));
                if response.clicked(){seek=Some(lyrics.lines[index].0);self.pending_seek=Some((index,time));}
                let base=if !self.follow.following {0.78}else if current.is_some_and(|i|index<i){0.32}else{0.48};
                let brightness=if response.hovered(){1.0}else{base+(1.0-base)*emphasis};
                let edge=((row.center().y-clip.top())/32.0).min((clip.bottom()-row.center().y)/32.0).clamp(0.15,1.0);
                let color=ui::TEXT.gamma_multiply(brightness*edge);
                let scale=0.92+0.08*emphasis;
                let at=row.min+vec2(0.0,galley.size().y*(1.0-scale)*0.5);
                let mut shape=Shape::galley_with_override_text_color(at,galley.clone(),color);
                shape.transform(egui::emath::TSTransform{translation:at.to_vec2()*(1.0-scale),scaling:scale});
                painter.add(shape);
                if current==Some(index)&&lyrics.synced {
                    painter.rect_filled(Rect::from_min_size(pos2(content.left(),top+4.0),vec2(2.0,(galley.size().y-8.0).max(8.0))),1.0,theme.main.gamma_multiply(emphasis));
                }
            }
        });
        let over=ui.input(|i|i.pointer.hover_pos()).is_some_and(|p|result.inner_rect.expand(16.0).contains(p));
        if lyrics.synced&&(!following || (over&&(wheel || (dragging&&(result.state.offset.y-desired).abs()>0.5)))) {self.follow.browse(result.state.offset.y as f64); }
        if seek.is_some(){self.follow.resume();self.follow.snap_next=true;}
        if animate&&lyrics.synced&&((following&&self.follow.spring.active()) || (now.playing&&self.emphasis.values().any(|e|e.active(time)))) {ui.ctx().request_repaint_after(Duration::from_millis(16));}
        else if now.playing&&lyrics.synced {
            let next=lyrics.lines.iter().filter_map(|(ms,_)|(*ms>position).then(||*ms-position)).min().unwrap_or(250);
            ui.ctx().request_repaint_after(Duration::from_millis(next.clamp(10,250) as u64));
        }
        seek
    }
}

#[cfg(test)] mod rendering_tests {
    use super::*;
    use eframe::egui::{Context,RawInput,CentralPanel,Event,MouseWheelUnit,Modifiers};
    fn words()->Lyrics {Lyrics {track:"demo".into(),synced:true,lines:(0..30).map(|i|(i*2000,format!("A sample line with enough words to wrap on a narrow window, number {i}"))).collect(),error:None}}
    fn now(ms:u32)->Now {Now::parse(&serde_json::json!({"track":{"uri":"demo","duration_ms":60000},"position_ms":ms,"playing":false}))}
    fn frame(ctx:&Context,view:&mut View,time:f64,position:u32,events:Vec<Event>){
        let input=RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(500.0,400.0))),time:Some(time),events,..Default::default()};
        let _=ctx.run(input,|ctx|{CentralPanel::default().show(ctx,|ui|{view.show(ui,&Theme::load("signal"),&now(position),&words(),false);});});
    }
    #[test] fn highlighting_does_not_rewrap_or_relayout_the_song(){
        let ctx=Context::default();ui::install_fonts(&ctx,None,None);let mut view=View::default();
        frame(&ctx,&mut view,0.0,0,vec![]);let before=view.rows.clone();
        frame(&ctx,&mut view,0.1,8000,vec![]);
        assert_eq!(before.len(),view.rows.len());
        for ((galley,y),(next,next_y)) in before.iter().zip(&view.rows){assert_eq!(y,next_y);assert_eq!(galley.size(),next.size());assert!(Arc::ptr_eq(galley,next));}
    }
    #[test] fn a_real_wheel_scroll_disables_following_across_playback_changes(){
        let ctx=Context::default();ui::install_fonts(&ctx,None,None);let mut view=View::default();
        frame(&ctx,&mut view,0.0,0,vec![]);
        frame(&ctx,&mut view,0.05,0,vec![Event::PointerMoved(pos2(240.0,220.0)),Event::MouseWheel{unit:MouseWheelUnit::Point,delta:vec2(0.0,-180.0),modifiers:Modifiers::NONE}]);
        assert!(!view.follow.following,"wheel scrolling must switch to browsing");
        frame(&ctx,&mut view,0.1,8000,vec![]);assert!(!view.follow.following,"a new lyric must not steal scrolling back");
    }
}

#[cfg(test)] mod interruption_tests {
    use super::*;
    #[test] fn fast_lines_continue_from_the_existing_highlight_strength() {
        let mut view=View::default();view.focus(Some(0),0.0);
        let before=view.emphasis[&0].value(0.125);
        view.focus(Some(1),0.125);assert!((view.emphasis[&0].value(0.125)-before).abs()<0.001);
        let before=view.emphasis[&0].value(0.13);
        view.focus(Some(2),0.13);assert!((view.emphasis[&0].value(0.13)-before).abs()<0.001);
    }
}

#[cfg(test)] mod playback_tests {
    use super::*;
    #[test] fn playing_after_earlier_lines_never_underflows_the_next_deadline() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let lyrics=Lyrics{track:"demo".into(),synced:true,lines:vec![(0,"First sample".into()),(2000,"Second sample".into()),(8000,"Third sample".into())],error:None};
        let now=Now::parse(&serde_json::json!({"track":{"uri":"demo","duration_ms":12000},"position_ms":3456,"playing":true}));
        let mut view=View::default();
        let _=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(500.0,400.0))),..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{view.show(ui,&Theme::load("signal"),&now,&lyrics,true);});});
        assert_eq!(view.current,Some(1));
    }
}
