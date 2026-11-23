//! Secondary native viewports each own a display-local dock; pin configuration is shared.
use super::*;
impl App {
    pub(super) fn secondary_views(&mut self,ctx:&egui::Context,state:Option<&DesktopState>,locked:bool){
        let count=state.map_or(1,|state|state.screens).min(32);
        self.secondary_docks.retain(|screen,_|*screen<count);
        self.dock_keyboard.retain(|screen,_|*screen<count);
        self.secondary_areas.retain(|screen,_|*screen<count);
        let mut actions=Vec::new();let mut areas=Vec::new();let mut extended_acts=Vec::new();
        let overview=self.desktop.as_ref().and_then(|d|d.overview.clone());
        let saver = if locked { self.idle_saver.started() } else { None };
        let (style, reduced, on_battery) = (
            self.config.screensaver,
            !self.config.animate || self.config.appearance.reduce_motion,
            self.mon.power.is_some_and(|battery| !battery.plugged),
        );
        for screen in 1..count {
            if let Some(started) = saver {
                // This surface has never painted desktop content. The compositor
                // keeps ordinary secondary viewports unmapped during the lock.
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of(("des-saver-display", screen)),
                    ViewportBuilder::default()
                        .with_title(format!("DEs-UI screensaver — display {}", screen + 1))
                        .with_app_id(format!("des-ui-saver-{screen}")),
                    |ctx, class| {
                        if class != egui::ViewportClass::Embedded {
                            screensaver::show(ctx, style, started.elapsed(), reduced, on_battery);
                        }
                    },
                );
            }
            if locked {
                // Made only while locked, so it has never drawn the session either;
                // the compositor shows it for this lock only, when no saver runs.
                let theme = &self.theme;
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of(("des-lock-display", screen)),
                    ViewportBuilder::default()
                        .with_title(format!("DEs-UI locked — display {}", screen + 1))
                        .with_app_id(format!("des-ui-lock-{screen}")),
                    |ctx, class| {
                        if class != egui::ViewportClass::Embedded {
                            CentralPanel::default().frame(Frame::new().fill(ui::GROUND)).show(ctx, |ui| crate::extended::locked(ui, theme, screen));
                        }
                    },
                );
            }
            let dock=self.secondary_docks.entry(screen).or_default();
            let mut output=None;
            let mut extended_act=None;
            let empty=DesktopState::default();
            let desktop=state.unwrap_or(&empty);
            let suppress=locked || self.capture.marking();
            let (theme,mon,pins,animate)=(&self.theme,&self.mon,&self.config.dock_pins,self.config.animate && !self.config.appearance.reduce_motion);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("des-display",screen+1)),
                ViewportBuilder::default().with_title(format!("des-ui display {}",screen+1)).with_app_id("des-ui"),
                |ctx,class|{
                    if class==egui::ViewportClass::Embedded{return;}
                    CentralPanel::default().frame(Frame::new().fill(ui::GROUND)).show(ctx,|ui|{
                        // Locked, the compositor hides this surface and shows the lock
                        // display instead; nothing of the session is drawn meanwhile.
                        if locked {}
                        else if crate::extended::shown_on(desktop,screen).is_empty() {extended_act=crate::extended::idle(ui,theme,mon,desktop,screen);}
                        else {extended_act=crate::extended::strip(ui,theme,desktop,screen);}
                    });
                    output=Some(if suppress {dock.reset();dock::Output::default()} else if dock.captures_keyboard() {
                        let bounds=ctx.screen_rect();
                        let rect=Rect::from_min_max(pos2((bounds.right()-360.0).max(bounds.left()),bounds.top()),bounds.right_bottom());
                        let mut result=dock::Output::default();
                        let keys=dock.reserve_keyboard(ctx);
                        ctx.input_mut(|i|i.events.extend(keys));
                        egui::Area::new(egui::Id::new(("display-apps",screen))).fixed_pos(rect.min).show(ctx,|ui| {
                            ui.set_min_size(rect.size());ui.set_max_size(rect.size());
                            egui::Frame::new().fill(ui::GROUND).inner_margin(20.0).show(ui,|ui| {
                                result=dock.panel_on(screen,ui,state,pins,theme,animate);
                            });
                        });
                        if let Some(area)=result.overlay {result.overlay=Some(area.union(rect));}
                        result
                    } else {dock::Output::default()});
                    if !suppress {if let Some(layout)=overview.as_ref(){crate::overview::show_on(ctx,theme,layout,state,screen);}}
                },
            );
            if let Some(act)=extended_act.filter(|_|!suppress){extended_acts.push((screen,act));}
            if let Some(output)=output{
                if let Some(action)=output.action{actions.push(action);}
                let area=output.overlay.map(|r|(r.left().floor() as i32,r.top().floor() as i32,r.width().ceil() as i32+1,r.height().ceil() as i32+1));
                areas.push((screen,area,output.keyboard));
            }
        }
        for (screen,area,keyboard) in areas{
            if self.secondary_areas.get(&screen)!=Some(&area){
                if let Some(desktop)=&mut self.desktop{
                    if desktop.send(Command::DockOn(screen,area)).is_ok(){self.secondary_areas.insert(screen,area);}
                }
            }
            if self.dock_keyboard.get(&screen).copied()!=Some(keyboard) {
                if let Some(desktop)=&mut self.desktop {
                    if desktop.send(Command::DockKeyboard {screen,active:keyboard}).is_ok(){self.dock_keyboard.insert(screen,keyboard);}
                }
            }
        }
        for action in actions{self.apply_dock_action(action);}
        for (screen,act) in extended_acts {
            match act {
                crate::extended::Act::Command(command)=>self.apply_dock_action(dock::Action::Command(command)),
                crate::extended::Act::OpenApps=>self.focus_apps(screen),
                crate::extended::Act::Displays=>{let config=self.config.clone();self.settings.open_at(&config,"displays");}
            }
        }
    }
}
