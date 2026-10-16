//! Secondary native viewports each own a display-local dock; pin configuration is shared.
use super::*;
impl App {
    pub(super) fn secondary_views(&mut self,ctx:&egui::Context,state:Option<&DesktopState>,locked:bool){
        let count=state.map_or(1,|state|state.screens).min(32);
        self.secondary_docks.retain(|screen,_|*screen<count);
        self.secondary_areas.retain(|screen,_|*screen<count);
        let mut actions=Vec::new();let mut areas=Vec::new();
        for screen in 1..count {
            let dock=self.secondary_docks.entry(screen).or_default();
            let mut output=None;
            let suppress=locked || self.capture.marking();
            let (theme,mon,pins,animate)=(&self.theme,&self.mon,&self.config.dock_pins,self.config.animate);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("des-display",screen+1)),
                ViewportBuilder::default().with_title(format!("des-ui display {}",screen+1)).with_app_id("des-ui"),
                |ctx,class|{
                    if class==egui::ViewportClass::Embedded{return;}
                    CentralPanel::default().frame(Frame::new().fill(ui::GROUND)).show(ctx,|ui|{
                        if locked {ui.centered_and_justified(|ui|{ui::text(ui,"LOCKED",ui::display(24.0),ui::SECONDARY);});}
                        else{panels::backdrop(ui,theme,mon,screen+1);}
                    });
                    output=Some(if suppress{dock.reset();dock::Output::default()}else{dock.show_on(screen,ctx,ctx.screen_rect(),state,pins,theme,animate)});
                },
            );
            if let Some(output)=output{
                if let Some(action)=output.action{actions.push(action);}
                let area=output.overlay.map(|r|(r.left().floor() as i32,r.top().floor() as i32,r.width().ceil() as i32+1,r.height().ceil() as i32+1));
                areas.push((screen,area));
            }
        }
        for (screen,area) in areas{
            if self.secondary_areas.get(&screen)!=Some(&area){
                if let Some(desktop)=&mut self.desktop{
                    if desktop.send(Command::DockOn(screen,area)).is_ok(){self.secondary_areas.insert(screen,area);}
                }
            }
        }
        for action in actions{self.apply_dock_action(action);}
    }
}
